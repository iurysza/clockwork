use std::fs::{self, File};

use anyhow::{Context, Result};
use fs4::{FileExt, TryLockError};

use crate::store::paths;

/// An RAII lock guard backed by an OS advisory file lock.
/// The lock is released when dropped (or when the process exits).
pub struct FileLock {
    _file: File,
}

impl FileLock {
    /// Acquire the global state lock (blocking).
    pub fn state() -> Result<Self> {
        let path = paths::state_lock_path()?;
        Self::acquire_blocking(&path)
    }

    /// Acquire the dispatch lock (non-blocking).
    /// Returns `None` if already held by another process.
    pub fn dispatch_non_blocking() -> Result<Option<Self>> {
        let path = paths::dispatch_lock_path()?;
        Self::acquire_non_blocking(&path)
    }

    /// Acquire a per-job lock (non-blocking).
    /// Returns `None` if already held (overlap prevention).
    pub fn job_non_blocking(job_id: &str) -> Result<Option<Self>> {
        let path = paths::job_lock_path(job_id)?;
        Self::acquire_non_blocking(&path)
    }

    /// Acquire a non-blocking lock at an arbitrary path.
    /// Returns `None` if already held.
    pub fn acquire_non_blocking_path(path: &std::path::Path) -> Result<Option<Self>> {
        Self::acquire_non_blocking(path)
    }

    fn acquire_blocking(path: &std::path::Path) -> Result<Self> {
        ensure_lock_parent(path)?;
        let file = File::create(path)
            .with_context(|| format!("failed to create lock file: {}", path.display()))?;
        // UFCS keeps the call on fs4. Method syntax trips `unstable_name_collisions`
        // because `std::fs::File::lock` is not stable on the 1.87 toolchain.
        FileExt::lock(&file)
            .with_context(|| format!("failed to acquire lock: {}", path.display()))?;
        Ok(Self { _file: file })
    }

    fn acquire_non_blocking(path: &std::path::Path) -> Result<Option<Self>> {
        ensure_lock_parent(path)?;
        let file = File::create(path)
            .with_context(|| format!("failed to create lock file: {}", path.display()))?;
        match FileExt::try_lock(&file) {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => {
                Err(error).with_context(|| format!("failed to acquire lock: {}", path.display()))
            }
        }
    }
}

fn ensure_lock_parent(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_blocking_lock_returns_none_while_held() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("job.lock");

        let held = FileLock::acquire_non_blocking_path(&path)
            .unwrap()
            .expect("lock should be free");
        assert!(
            FileLock::acquire_non_blocking_path(&path)
                .unwrap()
                .is_none()
        );

        drop(held);
        assert!(
            FileLock::acquire_non_blocking_path(&path)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn blocking_lock_excludes_a_non_blocking_acquire() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.lock");

        let held = FileLock::acquire_blocking(&path).unwrap();
        assert!(
            FileLock::acquire_non_blocking_path(&path)
                .unwrap()
                .is_none()
        );
        drop(held);

        FileLock::acquire_blocking(&path).unwrap();
    }
}
