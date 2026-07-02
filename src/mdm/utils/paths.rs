use crate::error::GitAiError;
use std::path::{Path, PathBuf};

/// Get the user's home directory
pub fn home_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(userprofile) = std::env::var("USERPROFILE")
            && !userprofile.is_empty()
        {
            return PathBuf::from(userprofile);
        }

        if let (Ok(home_drive), Ok(home_path)) =
            (std::env::var("HOMEDRIVE"), std::env::var("HOMEPATH"))
            && !home_drive.is_empty()
            && !home_path.is_empty()
        {
            return PathBuf::from(format!("{}{}", home_drive, home_path));
        }

        if let Ok(home) = std::env::var("HOME")
            && !home.is_empty()
        {
            return PathBuf::from(home);
        }

        dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
    }

    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME")
            && !home.is_empty()
        {
            return PathBuf::from(home);
        }

        dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
    }
}

/// Claude config directory, respecting the CLAUDE_CONFIG_DIR env var.
/// Falls back to ~/.claude when unset.
pub fn claude_config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    home_dir().join(".claude")
}

/// Strip the Windows extended-length path prefix (`\\?\`) if present.
/// On Windows, `std::fs::canonicalize` returns paths prefixed with `\\?\`
/// (e.g. `\\?\C:\Users\...`). This prefix causes problems when the path is
/// embedded in hook command strings for tools like Claude Code, Cursor, etc.
pub fn clean_path(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(stripped);
    }
    path
}

/// Convert a Windows path to a forward-slash path suitable for native Windows apps.
/// e.g. `C:\Users\Administrator\.git-ai\bin\git.exe` -> `C:/Users/Administrator/.git-ai/bin/git.exe`
/// Also strips the `\\?\` extended-length prefix if present (via `clean_path`).
/// This is needed because native GUI apps like Fork and Sublime Merge store paths
/// with forward slashes in their JSON settings files.
/// Non-Windows paths are returned unchanged.
pub fn to_windows_git_bash_style_path(path: &Path) -> String {
    clean_path(path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

/// Convert a Windows path to git bash (MSYS/MinGW) style path.
/// e.g. `C:\Users\Administrator\.git-ai\bin\git-ai.exe` -> `/c/Users/Administrator/.git-ai/bin/git-ai.exe`
/// This is needed because Claude Code runs hooks in git bash shell on Windows.
/// Non-Windows paths (or paths that don't match `X:\...` pattern) are returned unchanged.
pub fn to_git_bash_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    // Match a Windows absolute path like "C:\..." or "D:\..."
    let bytes = s.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        let drive_letter = (bytes[0] as char).to_ascii_lowercase();
        let rest = &s[2..]; // skip "C:"
        let rest_unix = rest.replace('\\', "/");
        return format!("/{}{}", drive_letter, rest_unix);
    }
    // Also handle the case where the path has no separator after the drive letter (e.g. C:foo)
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive_letter = (bytes[0] as char).to_ascii_lowercase();
        let rest = &s[2..];
        let rest_unix = rest.replace('\\', "/");
        return format!("/{}/{}", drive_letter, rest_unix);
    }
    // For non-Windows paths, just return as-is
    s.into_owned()
}

/// Get the absolute path to the currently running binary
pub fn get_current_binary_path() -> Result<PathBuf, GitAiError> {
    let path = std::env::current_exe()?;

    // Canonicalize to resolve any symlinks
    let canonical = path.canonicalize()?;

    Ok(clean_path(canonical))
}

/// Platform-specific executable name for the git-ai binary.
pub fn git_ai_binary_name() -> &'static str {
    if cfg!(windows) {
        "git-ai.exe"
    } else {
        "git-ai"
    }
}

/// Platform-specific executable name for the git shim.
pub fn git_shim_binary_name() -> &'static str {
    if cfg!(windows) { "git.exe" } else { "git" }
}

/// The managed install directory used by the install scripts.
pub fn managed_install_bin_dir() -> PathBuf {
    home_dir().join(".git-ai").join("bin")
}

/// Path to the git shim that git clients should use
/// This is in the same directory as the git-ai executable, but named "git"
pub fn git_shim_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join(git_shim_binary_name())))
        .unwrap_or_else(|| managed_install_bin_dir().join(git_shim_binary_name()))
}

/// Get the git shim path as a string (for use in settings files)
pub fn git_shim_path_string() -> String {
    git_shim_path().to_string_lossy().to_string()
}
