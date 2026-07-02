use crate::config as app_config;
use crate::error::GitAiError;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub internal_dir: PathBuf,
    pub lock_path: PathBuf,
    pub trace_socket_path: PathBuf,
    pub control_socket_path: PathBuf,
}

impl DaemonConfig {
    fn from_internal_dir(internal_dir: PathBuf) -> Self {
        let daemon_dir = internal_dir.join("daemon");
        #[cfg(unix)]
        let (lock_path, trace_socket_path, control_socket_path) = {
            let mut lock_path = daemon_dir.join("daemon.lock");
            let mut trace_socket_path = daemon_dir.join("trace2.sock");
            let mut control_socket_path = daemon_dir.join("control.sock");
            let too_long = |path: &Path| path.to_string_lossy().len() >= 100;

            if too_long(&trace_socket_path) || too_long(&control_socket_path) {
                let mut hasher = Sha256::new();
                hasher.update(internal_dir.to_string_lossy().as_bytes());
                let digest = format!("{:x}", hasher.finalize());
                let short = &digest[..16];
                let short_dir = std::env::temp_dir().join(format!("git-ai-d-{}", short));
                lock_path = short_dir.join("daemon.lock");
                trace_socket_path = short_dir.join("trace.sock");
                control_socket_path = short_dir.join("control.sock");
            }

            (lock_path, trace_socket_path, control_socket_path)
        };

        #[cfg(not(unix))]
        let (lock_path, trace_socket_path, control_socket_path) = {
            let mut hasher = Sha256::new();
            hasher.update(internal_dir.to_string_lossy().as_bytes());
            let digest = format!("{:x}", hasher.finalize());
            let short = &digest[..16];
            (
                daemon_dir.join("daemon.lock"),
                PathBuf::from(format!(r"\\.\pipe\git-ai-{}-trace2", short)),
                PathBuf::from(format!(r"\\.\pipe\git-ai-{}-control", short)),
            )
        };

        Self {
            internal_dir,
            lock_path,
            trace_socket_path,
            control_socket_path,
        }
    }

    pub fn from_home(home: &Path) -> Self {
        let internal_dir = home.join(".git-ai").join("internal");
        Self::from_internal_dir(internal_dir)
    }

    pub fn from_default_paths() -> Result<Self, GitAiError> {
        let internal_dir = app_config::internal_dir_path().ok_or_else(|| {
            GitAiError::Generic("Unable to determine ~/.git-ai/internal path".to_string())
        })?;
        Ok(Self::from_internal_dir(internal_dir))
    }

    pub fn from_env_or_default_paths() -> Result<Self, GitAiError> {
        let mut config = if let Ok(home) = std::env::var("GIT_AI_DAEMON_HOME")
            && !home.trim().is_empty()
        {
            Self::from_home(Path::new(&home))
        } else {
            Self::from_default_paths()?
        };

        if let Ok(path) = std::env::var("GIT_AI_DAEMON_CONTROL_SOCKET")
            && !path.trim().is_empty()
        {
            config.control_socket_path = PathBuf::from(path);
        }

        if let Ok(path) = std::env::var("GIT_AI_DAEMON_TRACE_SOCKET")
            && !path.trim().is_empty()
        {
            config.trace_socket_path = PathBuf::from(path);
        }

        Ok(config)
    }

    pub fn ensure_parent_dirs(&self) -> Result<(), GitAiError> {
        let daemon_dir = self
            .lock_path
            .parent()
            .ok_or_else(|| GitAiError::Generic("daemon lock path has no parent".to_string()))?;
        fs::create_dir_all(daemon_dir)?;
        fs::create_dir_all(&self.internal_dir)?;
        Ok(())
    }

    pub fn trace2_event_target(&self) -> String {
        Self::trace2_event_target_for_path(&self.trace_socket_path)
    }

    pub fn test_completion_log_dir(&self) -> PathBuf {
        self.internal_dir.join("daemon").join("test-completions")
    }

    pub fn test_completion_log_path_for_family(&self, family_key: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(family_key.as_bytes());
        let digest = format!("{:x}", hasher.finalize());
        self.test_completion_log_dir()
            .join(format!("{}.jsonl", &digest[..16]))
    }

    pub fn trace2_event_target_for_path(path: &Path) -> String {
        #[cfg(unix)]
        {
            format!("af_unix:stream:{}", path.to_string_lossy())
        }
        #[cfg(not(unix))]
        {
            path.to_string_lossy().to_string()
        }
    }
}
