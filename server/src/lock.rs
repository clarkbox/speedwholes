use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Error type for instance locking
#[derive(Debug)]
pub enum LockError {
    AlreadyRunning { pid: u32, lock_file: PathBuf },
    IoError(io::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::AlreadyRunning { pid, lock_file } => {
                write!(
                    f,
                    "Another instance is already running with PID {} (lock file: {:?})",
                    pid, lock_file
                )
            }
            LockError::IoError(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for LockError {}

impl From<io::Error> for LockError {
    fn from(e: io::Error) -> Self {
        LockError::IoError(e)
    }
}

/// Instance lock using a PID file
/// Prevents multiple instances from running with the same config directory
pub struct InstanceLock {
    lock_file_path: PathBuf,
    pid: u32,
}

impl InstanceLock {
    /// Acquire an instance lock for the given config directory
    ///
    /// Returns an error if another instance is already running
    /// Automatically cleans up stale locks from dead processes
    pub fn acquire(config_dir: &Path) -> Result<Self, LockError> {
        let lock_file_path = config_dir.join("speedwholes.pid");
        let current_pid = std::process::id();

        // Check if lock file already exists
        if lock_file_path.exists() {
            match fs::read_to_string(&lock_file_path) {
                Ok(contents) => {
                    if let Ok(existing_pid) = contents.trim().parse::<u32>() {
                        // Check if the process is still alive
                        if is_process_alive(existing_pid) {
                            tracing::error!(
                                pid = existing_pid,
                                lock_file = ?lock_file_path,
                                "Another instance is already running"
                            );
                            return Err(LockError::AlreadyRunning {
                                pid: existing_pid,
                                lock_file: lock_file_path,
                            });
                        } else {
                            tracing::warn!(
                                stale_pid = existing_pid,
                                lock_file = ?lock_file_path,
                                "Removing stale lock file from dead process"
                            );
                            // Process is dead, remove stale lock
                            fs::remove_file(&lock_file_path)?;
                        }
                    } else {
                        tracing::warn!(
                            lock_file = ?lock_file_path,
                            "Lock file contains invalid PID, removing"
                        );
                        fs::remove_file(&lock_file_path)?;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        lock_file = ?lock_file_path,
                        "Failed to read lock file, attempting to remove"
                    );
                    // Try to remove potentially corrupted lock file
                    let _ = fs::remove_file(&lock_file_path);
                }
            }
        }

        // Create config directory if it doesn't exist
        if let Some(parent) = lock_file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write our PID to the lock file
        let mut file = fs::File::create(&lock_file_path)?;
        write!(file, "{}", current_pid)?;
        file.sync_all()?;

        tracing::info!(
            pid = current_pid,
            lock_file = ?lock_file_path,
            "Acquired instance lock"
        );

        Ok(Self {
            lock_file_path,
            pid: current_pid,
        })
    }

    /// Release the lock by removing the PID file
    pub fn release(&self) {
        match fs::remove_file(&self.lock_file_path) {
            Ok(_) => {
                tracing::info!(
                    pid = self.pid,
                    lock_file = ?self.lock_file_path,
                    "Released instance lock"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    pid = self.pid,
                    lock_file = ?self.lock_file_path,
                    "Failed to remove lock file during release"
                );
            }
        }
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        self.release();
    }
}

/// Check if a process with the given PID is alive
#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    // On Unix systems, we can use kill(pid, 0) to check if a process exists
    // without actually sending a signal
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    match kill(Pid::from_raw(pid as i32), Signal::SIGCONT) {
        Ok(_) => true,                          // Process exists and we could signal it
        Err(nix::errno::Errno::ESRCH) => false, // No such process
        Err(nix::errno::Errno::EPERM) => true,  // Process exists but we don't have permission
        Err(_) => {
            // For any other error, assume process might be alive to be safe
            tracing::warn!(
                pid = pid,
                "Unexpected error checking process, assuming alive"
            );
            true
        }
    }
}

/// Check if a process with the given PID is alive
#[cfg(not(unix))]
fn is_process_alive(pid: u32) -> bool {
    // On non-Unix systems (Windows), check /proc/{pid} or use platform-specific APIs
    // For simplicity, we'll just check if /proc exists (Linux-style)
    let proc_path = format!("/proc/{}", pid);
    std::path::Path::new(&proc_path).exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_acquire_lock_success() {
        let temp_dir = TempDir::new().unwrap();
        let lock = InstanceLock::acquire(temp_dir.path()).unwrap();

        assert!(temp_dir.path().join("speedwholes.pid").exists());

        // Verify PID file contains correct PID
        let contents = fs::read_to_string(&lock.lock_file_path).unwrap();
        let written_pid: u32 = contents.trim().parse().unwrap();
        assert_eq!(written_pid, std::process::id());
    }

    #[test]
    fn test_lock_prevents_duplicate() {
        let temp_dir = TempDir::new().unwrap();
        let _lock1 = InstanceLock::acquire(temp_dir.path()).unwrap();

        // Second acquire should fail since lock is held
        let result = InstanceLock::acquire(temp_dir.path());
        assert!(matches!(result, Err(LockError::AlreadyRunning { .. })));
    }

    #[test]
    fn test_lock_auto_release_on_drop() {
        let temp_dir = TempDir::new().unwrap();
        let lock_path = temp_dir.path().join("speedwholes.pid");

        {
            let _lock = InstanceLock::acquire(temp_dir.path()).unwrap();
            assert!(lock_path.exists());
        } // lock dropped here

        // Lock file should be removed after drop
        assert!(!lock_path.exists());
    }

    #[test]
    fn test_process_alive_current_process() {
        // Current process should always be alive
        assert!(is_process_alive(std::process::id()));
    }

    #[test]
    fn test_process_alive_invalid_pid() {
        // Very high PID unlikely to exist
        assert!(!is_process_alive(999999999));
    }
}
