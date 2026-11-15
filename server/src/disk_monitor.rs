//! Background disk space monitor.
//!
//! Periodically checks free disk space and evicts cached files when space runs low.
//! Eviction order: source cache files (oldest first), then resize cache files (oldest first).
//! This runs out of band — never blocks image serving requests.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::time;

use crate::util::format_bytes;

/// Start the background disk monitor task.
///
/// If `min_free_mb` is 0, the monitor is disabled.
/// Otherwise, it checks free disk space every `check_interval_secs` seconds
/// and evicts oldest cached files when free space drops below the threshold.
pub fn start_disk_monitor(cache_path: PathBuf, min_free_mb: u64, check_interval_secs: u64) {
    if min_free_mb == 0 {
        tracing::info!("Disk monitor disabled (disk_min_free_mb = 0)");
        return;
    }

    let threshold_bytes = min_free_mb * 1024 * 1024;

    tracing::info!(
        min_free_mb = min_free_mb,
        check_interval_secs = check_interval_secs,
        "Disk monitor started — will evict cached files when free space < {}",
        format_bytes(threshold_bytes as usize),
    );

    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(check_interval_secs));
        // The first tick completes immediately; skip it so we don't run
        // an eviction check the instant the server starts.
        interval.tick().await;

        loop {
            interval.tick().await;
            if let Err(e) = check_and_evict(&cache_path, threshold_bytes).await {
                tracing::error!(error = %e, "Disk monitor check failed");
            }
        }
    });
}

/// Check free disk space and run eviction if needed.
async fn check_and_evict(cache_path: &Path, threshold_bytes: u64) -> Result<(), String> {
    let free = get_free_space(cache_path)?;

    if free >= threshold_bytes {
        tracing::debug!(
            free_space = %format_bytes(free as usize),
            threshold = %format_bytes(threshold_bytes as usize),
            "Disk space OK"
        );
        return Ok(());
    }

    tracing::warn!(
        free_space = %format_bytes(free as usize),
        threshold = %format_bytes(threshold_bytes as usize),
        "Disk space low — starting cache eviction"
    );

    // Phase 1: evict from source cache (oldest first)
    let source_dir = cache_path.join("source");
    if source_dir.is_dir() {
        let needed = threshold_bytes.saturating_sub(free);
        let (files_deleted, bytes_freed) = evict_oldest_files(&source_dir, needed).await;
        if files_deleted > 0 {
            tracing::info!(
                files_deleted = files_deleted,
                bytes_freed = %format_bytes(bytes_freed as usize),
                "Source cache eviction complete"
            );
        }
    }

    // Re-check free space before touching resize cache
    let free = get_free_space(cache_path)?;
    if free >= threshold_bytes {
        tracing::info!(
            free_space = %format_bytes(free as usize),
            "Disk space restored after source cache eviction"
        );
        return Ok(());
    }

    // Phase 2: evict from resize cache (oldest first)
    let resize_dir = cache_path.join("resize");
    if resize_dir.is_dir() {
        let needed = threshold_bytes.saturating_sub(free);
        let (files_deleted, bytes_freed) = evict_oldest_files(&resize_dir, needed).await;
        if files_deleted > 0 {
            tracing::info!(
                files_deleted = files_deleted,
                bytes_freed = %format_bytes(bytes_freed as usize),
                "Resize cache eviction complete"
            );
        }
    }

    let free = get_free_space(cache_path)?;
    if free < threshold_bytes {
        tracing::warn!(
            free_space = %format_bytes(free as usize),
            threshold = %format_bytes(threshold_bytes as usize),
            "Disk space still low after evicting all cache files"
        );
    } else {
        tracing::info!(
            free_space = %format_bytes(free as usize),
            "Disk space restored after cache eviction"
        );
    }

    Ok(())
}

/// Get available disk space in bytes for the filesystem containing `path`.
fn get_free_space(path: &Path) -> Result<u64, String> {
    let stat = nix::sys::statvfs::statvfs(path)
        .map_err(|e| format!("statvfs({:?}) failed: {}", path, e))?;
    // blocks_available * fragment_size = bytes available to unprivileged users
    Ok(stat.blocks_available() as u64 * stat.fragment_size() as u64)
}

/// Collect all files under `dir` recursively with their mtime and size.
fn collect_files_with_metadata(dir: &Path) -> Vec<(PathBuf, SystemTime, u64)> {
    let mut files = Vec::new();
    collect_files_recursive(dir, &mut files);
    files
}

fn collect_files_recursive(dir: &Path, out: &mut Vec<(PathBuf, SystemTime, u64)>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Ok(meta) = entry.metadata() {
                let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                out.push((path, mtime, meta.len()));
            }
        } else if path.is_dir() {
            collect_files_recursive(&path, out);
        }
    }
}

/// Delete oldest files in `dir` until `needed_bytes` have been freed.
/// Returns `(files_deleted, bytes_freed)`.
async fn evict_oldest_files(dir: &Path, needed_bytes: u64) -> (u64, u64) {
    // Collect file metadata on a blocking thread to avoid starving the async runtime
    let dir_owned = dir.to_path_buf();
    let mut files =
        match tokio::task::spawn_blocking(move || collect_files_with_metadata(&dir_owned)).await {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(error = %e, "Failed to collect cache files for eviction");
                return (0, 0);
            }
        };

    // Sort by mtime ascending (oldest first)
    files.sort_by_key(|(_, mtime, _)| *mtime);

    let mut files_deleted: u64 = 0;
    let mut bytes_freed: u64 = 0;

    for (path, _mtime, size) in &files {
        if bytes_freed >= needed_bytes {
            break;
        }
        match tokio::fs::remove_file(path).await {
            Ok(()) => {
                files_deleted += 1;
                bytes_freed += size;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Already deleted (e.g. by another instance), skip
            }
            Err(e) => {
                tracing::warn!(path = ?path, error = %e, "Failed to delete cache file during eviction");
            }
        }
    }

    // Clean up empty subdirectories (best-effort)
    if files_deleted > 0 {
        let dir_owned = dir.to_path_buf();
        let _ = tokio::task::spawn_blocking(move || {
            remove_empty_dirs(&dir_owned);
        })
        .await;
    }

    (files_deleted, bytes_freed)
}

/// Recursively remove empty directories under `dir` (but not `dir` itself).
fn remove_empty_dirs(dir: &Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_empty_dirs(&path);
            // Try to remove — will silently fail if not empty
            let _ = std::fs::remove_dir(&path);
        }
    }
}
