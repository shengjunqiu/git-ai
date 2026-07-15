use crate::error::GitAiError;
use crate::git::repository::exec_git;
use crate::mdm::utils::{git_ai_binary_name, git_shim_binary_name, managed_install_bin_dir};
use std::fs;
use std::path::{Path, PathBuf};

/// Ensures the libexec symlink exists for Fork compatibility.
/// Creates a symlink from <binary_parent>/../libexec to the real git's libexec.
pub fn ensure_git_symlinks() -> Result<(), GitAiError> {
    // Get current executable path
    let exe_path = std::env::current_exe()?;

    // Skip symlink creation if running from Nix store (read-only filesystem)
    // or other read-only install locations. In these cases, the packaging system
    // (e.g., Nix flake) should handle creating the libexec symlink at build time.
    if exe_path.to_string_lossy().contains("/nix/store") {
        return Ok(());
    }

    if is_managed_install_binary(&exe_path) {
        let real_git_path = PathBuf::from(crate::config::Config::get().git_cmd());
        ensure_git_proxy_shims_for_binary(&exe_path, &real_git_path)?;
    }

    ensure_libexec_symlink_for_binary(&exe_path)
}

fn is_managed_install_binary(binary_path: &Path) -> bool {
    let Some(binary_dir) = binary_path.parent() else {
        return false;
    };

    let managed_dir = managed_install_bin_dir();
    paths_equal(binary_dir, &managed_dir)
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn ensure_git_proxy_shims_for_binary(
    binary_path: &Path,
    real_git_path: &Path,
) -> Result<bool, GitAiError> {
    if !is_managed_install_binary(binary_path) {
        return Ok(false);
    }

    let binary_dir = binary_path
        .parent()
        .ok_or_else(|| GitAiError::Generic("Cannot get binary directory".to_string()))?;
    let git_ai_path = binary_dir.join(git_ai_binary_name());
    if !git_ai_path.exists() {
        return Ok(false);
    }

    let mut changed = false;
    changed |= ensure_git_proxy_shim(&git_ai_path, &binary_dir.join(git_shim_binary_name()))?;
    changed |= ensure_git_og_shim(real_git_path, binary_dir)?;
    Ok(changed)
}

#[cfg(unix)]
fn ensure_git_proxy_shim(git_ai_path: &Path, shim_path: &Path) -> Result<bool, GitAiError> {
    ensure_symlink(git_ai_path, shim_path)
}

#[cfg(windows)]
fn ensure_git_proxy_shim(git_ai_path: &Path, shim_path: &Path) -> Result<bool, GitAiError> {
    if shim_path.exists() || shim_path.symlink_metadata().is_ok() {
        return Ok(false);
    }
    fs::copy(git_ai_path, shim_path)?;
    Ok(true)
}

#[cfg(unix)]
fn ensure_git_og_shim(real_git_path: &Path, binary_dir: &Path) -> Result<bool, GitAiError> {
    ensure_symlink(real_git_path, &binary_dir.join("git-og"))
}

#[cfg(windows)]
fn ensure_git_og_shim(real_git_path: &Path, binary_dir: &Path) -> Result<bool, GitAiError> {
    let path = binary_dir.join("git-og.cmd");
    let contents = format!("@echo off\r\n\"{}\" %*\r\n", real_git_path.display());
    if fs::read_to_string(&path).ok().as_deref() == Some(contents.as_str()) {
        return Ok(false);
    }
    fs::write(path, contents)?;
    Ok(true)
}

#[cfg(unix)]
fn ensure_symlink(target: &Path, link_path: &Path) -> Result<bool, GitAiError> {
    if symlink_points_to(link_path, target) {
        return Ok(false);
    }

    remove_existing_file(link_path)?;
    std::os::unix::fs::symlink(target, link_path)?;
    Ok(true)
}

#[cfg(unix)]
fn symlink_points_to(link_path: &Path, expected_target: &Path) -> bool {
    let Ok(target) = fs::read_link(link_path) else {
        return false;
    };
    let resolved_target = if target.is_absolute() {
        target
    } else {
        link_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(target)
    };
    paths_equal(&resolved_target, expected_target)
}

fn remove_existing_file(path: &Path) -> Result<(), GitAiError> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        return Err(GitAiError::Generic(format!(
            "Refusing to replace directory {}",
            path.display()
        )));
    }
    fs::remove_file(path)?;
    Ok(())
}

fn ensure_libexec_symlink_for_binary(exe_path: &Path) -> Result<(), GitAiError> {
    // Get parent directories: binary_dir is e.g. ~/.git-ai/bin, base_dir is ~/.git-ai
    let binary_dir = exe_path
        .parent()
        .ok_or_else(|| GitAiError::Generic("Cannot get binary directory".to_string()))?;
    let base_dir = binary_dir
        .parent()
        .ok_or_else(|| GitAiError::Generic("Cannot get base directory".to_string()))?;

    // Get real git's exec-path (e.g. /usr/libexec/git-core)
    let output = exec_git(&["--exec-path".to_string()])?;
    let exec_path = String::from_utf8(output.stdout)?.trim().to_string();
    let exec_path = PathBuf::from(exec_path);

    // Get the libexec directory (parent of git-core)
    let libexec_target = exec_path.parent().ok_or_else(|| {
        GitAiError::Generic("Cannot get libexec directory from exec-path".to_string())
    })?;

    // Create symlink: base_dir/libexec -> /usr/libexec
    let symlink_path = base_dir.join("libexec");

    // Remove existing symlink/junction if present
    if symlink_path.exists() || symlink_path.symlink_metadata().is_ok() {
        // On Windows, junctions are directories, so use remove_dir
        #[cfg(windows)]
        {
            if windows_junction_exists(&symlink_path)? {
                junction::delete(&symlink_path).map_err(|e| {
                    GitAiError::Generic(format!(
                        "Failed to remove existing libexec junction {}: {}",
                        symlink_path.display(),
                        e
                    ))
                })?;
            } else {
                let metadata = symlink_path.symlink_metadata()?;
                if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
                    // Older/failed installers can leave an ordinary empty
                    // directory here. It is safe to migrate, but never delete
                    // a non-empty directory that may contain user files.
                    if fs::read_dir(&symlink_path)?.next().is_none() {
                        fs::remove_dir(&symlink_path)?;
                    } else {
                        return Err(GitAiError::Generic(format!(
                            "Refusing to replace non-empty libexec directory {}",
                            symlink_path.display()
                        )));
                    }
                } else if !metadata.file_type().is_symlink() {
                    return Err(GitAiError::Generic(format!(
                        "Refusing to replace non-link libexec path {}",
                        symlink_path.display()
                    )));
                } else {
                    // Windows uses remove_dir for directory symlinks. Broken
                    // links cannot be followed to determine their kind, so
                    // keep the safe remove_file fallback after verifying this
                    // is a reparse link.
                    if fs::remove_dir(&symlink_path).is_err() {
                        fs::remove_file(&symlink_path)?;
                    }
                }
            }
        }
        #[cfg(unix)]
        std::fs::remove_file(&symlink_path)?;
    }

    #[cfg(unix)]
    std::os::unix::fs::symlink(libexec_target, &symlink_path)?;

    #[cfg(windows)]
    create_junction(&symlink_path, libexec_target)?;

    Ok(())
}

#[cfg(windows)]
fn windows_junction_exists(path: &Path) -> Result<bool, GitAiError> {
    match junction::exists(path) {
        Ok(exists) => Ok(exists),
        // ERROR_NOT_A_REPARSE_POINT means the path exists but is an ordinary
        // file/directory. Treat it as "not a junction" and inspect it safely.
        Err(error) if error.raw_os_error() == Some(4390) => Ok(false),
        Err(error) => Err(GitAiError::Generic(format!(
            "Failed to inspect existing libexec junction {}: {}",
            path.display(),
            error
        ))),
    }
}

/// Create a directory junction on Windows (doesn't require admin privileges)
#[cfg(windows)]
fn create_junction(
    junction_path: &std::path::Path,
    target: &std::path::Path,
) -> Result<(), GitAiError> {
    junction::create(target, junction_path).map_err(|e| {
        GitAiError::Generic(format!(
            "Failed to create junction {} -> {}: {}",
            junction_path.display(),
            target.display(),
            e
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::tempdir;

    struct EnvVarGuard {
        key: &'static str,
        old: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var(key).ok();
            // SAFETY: tests marked `serial` avoid concurrent env mutation.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: tests marked `serial` avoid concurrent env mutation.
            unsafe {
                if let Some(old) = &self.old {
                    std::env::set_var(self.key, old);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    fn test_binary_path(home: &Path) -> PathBuf {
        home.join(".git-ai").join("bin").join(git_ai_binary_name())
    }

    #[cfg(windows)]
    #[test]
    fn windows_junction_check_treats_plain_directory_as_non_junction() {
        let temp = tempdir().unwrap();
        let plain_dir = temp.path().join("libexec");
        fs::create_dir(&plain_dir).unwrap();

        assert!(!windows_junction_exists(&plain_dir).unwrap());
    }

    #[test]
    #[serial]
    fn ensure_git_proxy_shims_creates_missing_managed_shims() {
        let temp = tempdir().unwrap();
        let bin_dir = temp.path().join(".git-ai").join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let git_ai = test_binary_path(temp.path());
        fs::write(&git_ai, "fake git-ai").unwrap();
        let real_git = temp.path().join("real-git");
        fs::write(&real_git, "fake git").unwrap();
        let _home = EnvVarGuard::set("HOME", temp.path().to_str().unwrap());
        #[cfg(windows)]
        let _userprofile = EnvVarGuard::set("USERPROFILE", temp.path().to_str().unwrap());

        let changed = ensure_git_proxy_shims_for_binary(&git_ai, &real_git).unwrap();

        assert!(changed);
        let git_shim = bin_dir.join(git_shim_binary_name());
        assert!(git_shim.exists() || git_shim.symlink_metadata().is_ok());
        #[cfg(unix)]
        assert!(paths_equal(&fs::read_link(&git_shim).unwrap(), &git_ai));
        #[cfg(windows)]
        assert_eq!(fs::read(&git_shim).unwrap(), fs::read(&git_ai).unwrap());

        #[cfg(unix)]
        assert!(paths_equal(
            &fs::read_link(bin_dir.join("git-og")).unwrap(),
            &real_git
        ));
        #[cfg(windows)]
        assert_eq!(
            fs::read_to_string(bin_dir.join("git-og.cmd")).unwrap(),
            format!("@echo off\r\n\"{}\" %*\r\n", real_git.display())
        );
    }

    #[cfg(unix)]
    #[test]
    #[serial]
    fn ensure_git_proxy_shims_replaces_stale_unix_symlink() {
        let temp = tempdir().unwrap();
        let bin_dir = temp.path().join(".git-ai").join("bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let git_ai = test_binary_path(temp.path());
        fs::write(&git_ai, "fake git-ai").unwrap();
        let stale_target = temp.path().join("old-git-ai");
        std::os::unix::fs::symlink(&stale_target, bin_dir.join(git_shim_binary_name())).unwrap();
        let real_git = temp.path().join("real-git");
        fs::write(&real_git, "fake git").unwrap();
        let _home = EnvVarGuard::set("HOME", temp.path().to_str().unwrap());

        let changed = ensure_git_proxy_shims_for_binary(&git_ai, &real_git).unwrap();

        assert!(changed);
        assert!(paths_equal(
            &fs::read_link(bin_dir.join(git_shim_binary_name())).unwrap(),
            &git_ai
        ));
    }

    #[test]
    #[serial]
    fn ensure_git_proxy_shims_skips_non_install_binary() {
        let temp = tempdir().unwrap();
        let project_bin = temp.path().join("target").join("debug");
        fs::create_dir_all(&project_bin).unwrap();
        let git_ai = project_bin.join(git_ai_binary_name());
        fs::write(&git_ai, "fake git-ai").unwrap();
        let real_git = temp.path().join("real-git");
        fs::write(&real_git, "fake git").unwrap();
        let _home = EnvVarGuard::set("HOME", temp.path().to_str().unwrap());
        #[cfg(windows)]
        let _userprofile = EnvVarGuard::set("USERPROFILE", temp.path().to_str().unwrap());

        let changed = ensure_git_proxy_shims_for_binary(&git_ai, &real_git).unwrap();

        assert!(!changed);
        assert!(!project_bin.join(git_shim_binary_name()).exists());
        assert!(!project_bin.join("git-og").exists());
        assert!(!project_bin.join("git-og.cmd").exists());
    }
}
