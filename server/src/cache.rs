use crate::error::{AppError, Result};
use crate::util::format_bytes;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Cached files up to this size are read whole. Larger ones are streamed.
pub const STREAM_THRESHOLD: u64 = 1024 * 1024;

/// Read size for a streamed file. One chunk is in flight per response.
const STREAM_CHUNK: usize = 256 * 1024;

/// A cache hit: the bytes of a small file, or an open handle for a large one.
pub enum CachedFile {
    Small(Vec<u8>),
    Large { file: std::fs::File, len: u64 },
}

impl CachedFile {
    /// Response body and its length. A large file is not buffered.
    pub fn into_body(self) -> (axum::body::Body, u64) {
        match self {
            CachedFile::Small(data) => {
                let len = data.len() as u64;
                (axum::body::Body::from(data), len)
            }
            CachedFile::Large { file, len } => (file_body(file), len),
        }
    }
}

/// Body that reads an open file one chunk at a time.
pub fn file_body(file: std::fs::File) -> axum::body::Body {
    use tokio::io::AsyncReadExt;
    let file = fs::File::from_std(file);
    let stream = futures::stream::try_unfold(file, |mut file| async move {
        let mut chunk = vec![0u8; STREAM_CHUNK];
        let read = file.read(&mut chunk).await?;
        if read == 0 {
            return Ok::<_, std::io::Error>(None);
        }
        chunk.truncate(read);
        Ok(Some((axum::body::Bytes::from(chunk), file)))
    });
    axum::body::Body::from_stream(stream)
}

/// Default subdomain name used when no subdomain is specified
pub const DEFAULT_SUBDOMAIN: &str = "_default";

/// Disk-based image cache with subdomain support
pub struct DiskCache {
    base_path: PathBuf,
    subdomain: String,
}

impl DiskCache {
    /// Create a new cache for a specific subdomain
    pub fn new(base_path: PathBuf, subdomain: Option<&str>) -> Self {
        let subdomain = subdomain
            .map(sanitize_subdomain_name)
            .unwrap_or_else(|| DEFAULT_SUBDOMAIN.to_string());
        Self {
            base_path,
            subdomain,
        }
    }

    /// Initialize the cache directory (including subdomain folder)
    pub async fn init(&self) -> Result<()> {
        let subdomain_path = self.base_path.join(&self.subdomain);
        fs::create_dir_all(&subdomain_path).await.map_err(|e| {
            AppError::CacheError(format!("Failed to create cache directory: {}", e))
        })?;
        Ok(())
    }

    /// Create a new cache and initialize its directory in one call.
    /// Logs the initialization with the provided name for identification.
    pub async fn new_initialized(
        base_path: PathBuf,
        name: &str,
        subdomain: Option<&str>,
    ) -> Result<Self> {
        let cache = Self::new(base_path, subdomain);
        cache.init().await.map_err(|e| {
            tracing::error!(
                cache_path = ?cache.base_path,
                cache_name = %name,
                subdomain = %cache.subdomain,
                error = %e,
                "Failed to initialize {} cache directory", name
            );
            e
        })?;
        tracing::debug!(cache_path = ?cache.base_path, subdomain = %cache.subdomain, "Initialized {} cache", name);
        Ok(cache)
    }

    /// Generate a cache key from resize parameters (for resize cache)
    pub fn generate_key(
        url: &str,
        width: u32,
        height: u32,
        fit_mode: &str,
        quality: u8,
        format: &str,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        hasher.update(width.to_le_bytes());
        hasher.update(height.to_le_bytes());
        hasher.update(fit_mode.as_bytes());
        hasher.update([quality]);
        hasher.update(format.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Generate a cache key for source images (just URL hash)
    pub fn generate_source_key(url: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"source:");
        hasher.update(url.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Cache key for a reduced source copy. The size is part of the key, so
    /// changing `source_proxy_min_side` does not reuse a copy built for another size.
    pub fn generate_proxy_key(url: &str, min_side: u32) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"proxy:");
        hasher.update(min_side.to_le_bytes());
        hasher.update(url.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Get the file path for a cache key (includes subdomain folder)
    fn cache_path(&self, key: &str, extension: &str) -> PathBuf {
        // Path: base_path/subdomain/2-char-prefix/hash.ext
        let subdir = &key[0..2];
        self.base_path
            .join(&self.subdomain)
            .join(subdir)
            .join(format!("{}.{}", key, extension))
    }

    /// Try to get a cached image
    pub async fn get(&self, key: &str, extension: &str) -> Result<Option<Vec<u8>>> {
        let path = self.cache_path(key, extension);

        match fs::read(&path).await {
            Ok(data) => {
                tracing::debug!("Cache hit: {}", key);
                Ok(Some(data))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("Cache miss: {}", key);
                Ok(None)
            }
            Err(e) => Err(AppError::CacheError(format!("Failed to read cache: {}", e))),
        }
    }

    /// Open a cached file for serving. Small files come back whole, in one
    /// blocking step. Large files come back as a handle to stream from.
    pub async fn open(&self, key: &str, extension: &str) -> Result<Option<CachedFile>> {
        let path = self.cache_path(key, extension);
        let opened = tokio::task::spawn_blocking(move || -> std::io::Result<Option<CachedFile>> {
            use std::io::Read;
            let mut file = match std::fs::File::open(&path) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e),
            };
            let len = file.metadata()?.len();
            if len > STREAM_THRESHOLD {
                return Ok(Some(CachedFile::Large { file, len }));
            }
            let mut data = Vec::with_capacity(len as usize);
            file.read_to_end(&mut data)?;
            Ok(Some(CachedFile::Small(data)))
        })
        .await
        .map_err(|e| AppError::CacheError(format!("Cache read task failed: {e}")))?;
        opened.map_err(|e| AppError::CacheError(format!("Failed to read cache: {e}")))
    }

    /// Store an image in the cache
    pub async fn put(&self, key: &str, extension: &str, data: &[u8]) -> Result<()> {
        let path = self.cache_path(key, extension);

        // Ensure the subdirectory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                AppError::CacheError(format!("Failed to create cache subdir: {}", e))
            })?;
        }

        // Write to a temp file first, then rename for atomicity.
        // Use a unique temp name to avoid races when multiple tasks cache the
        // same key concurrently — otherwise two writers share the same .tmp
        // file and the loser's rename fails with ENOENT.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let unique_suffix = format!("{}-{}", std::process::id(), nanos);
        let temp_path = path.with_extension(format!("tmp.{}", unique_suffix));

        let result: Result<()> = async {
            let mut file = fs::File::create(&temp_path)
                .await
                .map_err(|e| AppError::CacheError(format!("Failed to create temp file: {}", e)))?;

            file.write_all(data)
                .await
                .map_err(|e| AppError::CacheError(format!("Failed to write cache data: {}", e)))?;

            file.sync_all()
                .await
                .map_err(|e| AppError::CacheError(format!("Failed to sync cache file: {}", e)))?;

            fs::rename(&temp_path, &path)
                .await
                .map_err(|e| AppError::CacheError(format!("Failed to rename temp file: {}", e)))?;

            Ok(())
        }
        .await;

        // Clean up temp file on failure (best-effort)
        if result.is_err() {
            let _ = fs::remove_file(&temp_path).await;
        }

        result?;

        tracing::info!(
            cache_key = %key,
            path = ?path,
            size_bytes = data.len(),
            size_human = %format_bytes(data.len()),
            "Saved file to cache"
        );
        Ok(())
    }
}

/// Sanitize subdomain name for filesystem use
fn sanitize_subdomain_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Disk stats for a single subdomain
#[derive(Debug, Clone, Serialize, Default)]
pub struct SubdomainDiskStats {
    pub resize_kb: u64,
    pub source_kb: u64,
}

/// Response structure for disk stats endpoint
#[derive(Debug, Clone, Serialize)]
pub struct DiskStatsResponse {
    pub subdomains: HashMap<String, SubdomainDiskStats>,
}

/// Calculate total size of a directory in bytes (recursive)
fn dir_size_sync(path: &std::path::Path) -> u64 {
    let mut total: u64 = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Ok(meta) = entry.metadata() {
                    total += meta.len();
                }
            } else if path.is_dir() {
                total += dir_size_sync(&path);
            }
        }
    }
    total
}

/// Get disk stats for all subdomains in the cache
/// Returns size in KB (rounded to nearest 1 KB) per subdomain
pub async fn get_disk_stats(cache_path: &std::path::Path) -> DiskStatsResponse {
    let resize_path = cache_path.join("resize");
    let source_path = cache_path.join("source");

    let mut subdomains: HashMap<String, SubdomainDiskStats> = HashMap::new();

    // Scan resize cache subdomains
    if let Ok(entries) = std::fs::read_dir(&resize_path) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    let size_bytes = dir_size_sync(&entry.path());
                    let size_kb = (size_bytes + 512) / 1024; // Round to nearest KB
                    subdomains.entry(name.to_string()).or_default().resize_kb = size_kb;
                }
            }
        }
    }

    // Scan source cache subdomains
    if let Ok(entries) = std::fs::read_dir(&source_path) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    let size_bytes = dir_size_sync(&entry.path());
                    let size_kb = (size_bytes + 512) / 1024; // Round to nearest KB
                    subdomains.entry(name.to_string()).or_default().source_kb = size_kb;
                }
            }
        }
    }

    DiskStatsResponse { subdomains }
}

/// Determine the file extension from content type or URL
pub fn extension_from_content_type(content_type: Option<&str>, url: &str) -> &'static str {
    if let Some(ct) = content_type {
        match ct {
            "image/jpeg" | "image/jpg" => return "jpg",
            "image/png" => return "png",
            "image/gif" => return "gif",
            "image/webp" => return "webp",
            "image/bmp" => return "bmp",
            _ => {}
        }
    }

    // Fall back to URL extension
    let url_lower = url.to_lowercase();
    if url_lower.contains(".png") {
        "png"
    } else if url_lower.contains(".gif") {
        "gif"
    } else if url_lower.contains(".webp") {
        "webp"
    } else if url_lower.contains(".bmp") {
        "bmp"
    } else {
        // Default to JPEG
        "jpg"
    }
}

/// Get the MIME type for an extension
pub fn mime_for_extension(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

/// Result of clearing a subdomain cache
#[derive(Debug, Clone, Serialize)]
pub struct CacheClearResult {
    pub files_deleted: u64,
    pub bytes_freed: u64,
}

/// Clear a subdomain's cache directory (resize or source)
/// Returns the number of files and bytes deleted
pub async fn clear_subdomain_cache(
    cache_path: &std::path::Path,
    cache_type: &str,
    subdomain: &str,
) -> std::io::Result<CacheClearResult> {
    let sanitized = sanitize_subdomain_name(subdomain);
    let subdomain_path = cache_path.join(cache_type).join(&sanitized);

    if !subdomain_path.exists() {
        return Ok(CacheClearResult {
            files_deleted: 0,
            bytes_freed: 0,
        });
    }

    // Calculate stats before deletion
    let bytes_freed = dir_size_sync(&subdomain_path);
    let files_deleted = count_files_sync(&subdomain_path);

    // Remove the entire subdomain directory
    fs::remove_dir_all(&subdomain_path).await?;

    // Recreate the empty directory
    fs::create_dir_all(&subdomain_path).await?;

    tracing::info!(
        cache_type = %cache_type,
        subdomain = %subdomain,
        files_deleted = files_deleted,
        bytes_freed = bytes_freed,
        "Cleared subdomain cache"
    );

    Ok(CacheClearResult {
        files_deleted,
        bytes_freed,
    })
}

/// Count files recursively in a directory
fn count_files_sync(path: &std::path::Path) -> u64 {
    let mut count: u64 = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                count += 1;
            } else if path.is_dir() {
                count += count_files_sync(&path);
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    async fn round_trip(bytes: &[u8]) -> CachedFile {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = DiskCache::new(dir.path().to_path_buf(), None);
        cache.init().await.expect("init");
        cache.put("abcd", "bin", bytes).await.expect("put");
        let opened = cache
            .open("abcd", "bin")
            .await
            .expect("open")
            .expect("present");
        // Unlink the directory while a large file's descriptor is still open.
        drop(dir);
        opened
    }

    #[tokio::test]
    async fn files_at_the_threshold_are_buffered_and_larger_ones_stream() {
        let small = vec![9u8; STREAM_THRESHOLD as usize];
        match round_trip(&small).await {
            CachedFile::Small(data) => assert_eq!(data, small),
            CachedFile::Large { .. } => panic!("a file at the threshold was streamed"),
        }

        let big = vec![4u8; STREAM_THRESHOLD as usize + 1];
        match round_trip(&big).await {
            CachedFile::Large { file, len } => {
                assert_eq!(len, big.len() as u64);
                let (body, body_len) = CachedFile::Large { file, len }.into_body();
                assert_eq!(body_len, big.len() as u64);
                let collected = body.collect().await.expect("stream").to_bytes();
                assert_eq!(&collected[..], &big[..]);
            }
            CachedFile::Small(_) => panic!("a file above the threshold was buffered"),
        }
    }

    #[tokio::test]
    async fn missing_file_is_a_miss() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = DiskCache::new(dir.path().to_path_buf(), None);
        cache.init().await.expect("init");
        assert!(cache.open("abcd", "bin").await.expect("open").is_none());
    }
}
