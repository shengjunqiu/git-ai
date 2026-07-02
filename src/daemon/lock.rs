use crate::error::GitAiError;
use crate::utils::LockFile;
use std::fs;
use std::path::Path;

pub struct DaemonLock {
    _lock: LockFile,
}

impl DaemonLock {
    pub fn acquire(path: &Path) -> Result<Self, GitAiError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock = LockFile::try_acquire(path).ok_or_else(|| {
            GitAiError::Generic(
                "git-ai background service is already running (lock held)".to_string(),
            )
        })?;
        Ok(Self { _lock: lock })
    }
}
