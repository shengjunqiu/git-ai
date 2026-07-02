use crate::mdm::utils::home_dir;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use uuid::Uuid;

pub(crate) fn config_file_path() -> Option<PathBuf> {
    Some(home_dir().join(".git-ai").join("config.json"))
}

/// Public accessor for config file path
#[allow(dead_code)]
pub fn config_file_path_public() -> Option<PathBuf> {
    config_file_path()
}

/// Returns the path to the git-ai base directory (~/.git-ai)
pub fn git_ai_dir_path() -> Option<PathBuf> {
    Some(home_dir().join(".git-ai"))
}

/// Returns the path to the internal state directory (~/.git-ai/internal)
/// This is where git-ai stores internal files like distinct_id, update_check, etc.
pub fn internal_dir_path() -> Option<PathBuf> {
    git_ai_dir_path().map(|dir| dir.join("internal"))
}

/// Returns the path to the skills directory (~/.git-ai/skills)
/// This is where git-ai installs skills for Claude Code and other agents
pub fn skills_dir_path() -> Option<PathBuf> {
    git_ai_dir_path().map(|dir| dir.join("skills"))
}

/// Public accessor for ID file path (~/.git-ai/internal/distinct_id)
pub fn id_file_path() -> Option<PathBuf> {
    internal_dir_path().map(|dir| dir.join("distinct_id"))
}

/// Cache for the distinct_id to avoid repeated file reads
static DISTINCT_ID: OnceLock<String> = OnceLock::new();

/// Get or create the distinct_id (UUID) from ~/.git-ai/internal/distinct_id
/// If the file doesn't exist, generates a new UUID and writes it to the file.
/// The result is cached for the lifetime of the process.
pub fn get_or_create_distinct_id() -> String {
    DISTINCT_ID
        .get_or_init(|| {
            let id_path = match id_file_path() {
                Some(path) => path,
                None => return "unknown".to_string(),
            };

            // Try to read existing ID
            if let Ok(existing_id) = fs::read_to_string(&id_path) {
                let trimmed = existing_id.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }

            // Generate new UUID
            let new_id = Uuid::new_v4().to_string();

            // Ensure directory exists
            if let Some(parent) = id_path.parent() {
                let _ = fs::create_dir_all(parent);
            }

            // Write the new ID to file
            if let Err(e) = fs::write(&id_path, &new_id) {
                eprintln!("Warning: Failed to write distinct_id file: {}", e);
            }

            new_id
        })
        .clone()
}

/// Returns the path to the update check cache file (~/.git-ai/internal/update_check)
pub fn update_check_path() -> Option<PathBuf> {
    internal_dir_path().map(|dir| dir.join("update_check"))
}

pub(crate) fn is_executable(path: &Path) -> bool {
    if !path.exists() || !path.is_file() {
        return false;
    }
    // Basic check: existence is sufficient for our purposes; OS will enforce exec perms.
    // On Unix we could check permissions, but many filesystems differ. Keep it simple.
    true
}

/// Check whether two paths refer to the same underlying file.
/// On Unix this compares (dev, ino); on other platforms it falls back to
/// comparing canonicalized paths.
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b)) {
            return ma.dev() == mb.dev() && ma.ino() == mb.ino();
        }
    }
    #[cfg(not(unix))]
    {
        if let (Ok(ca), Ok(cb)) = (a.canonicalize(), b.canonicalize()) {
            return ca == cb;
        }
    }
    false
}

/// Detect if a path is actually the git-ai binary (or a symlink to it).
/// This prevents `git_cmd()` from returning the git-ai shim, which would
/// cause infinite recursion: handle_git() -> proxy_to_git() -> shim -> handle_git() -> ...
pub(crate) fn path_is_git_ai_binary(path: &Path) -> bool {
    // Check canonical path — if the path resolves to a binary whose name
    // is git-ai (or a variant), it is the git-ai binary regardless of what
    // the original path looks like (catches symlinks like `git -> git-ai`).
    if let Ok(canonical) = path.canonicalize()
        && let Some(name) = canonical.file_name().and_then(|n| n.to_str())
    {
        let stem = name.strip_suffix(".exe").unwrap_or(name);
        if stem == "git-ai" || stem.starts_with("git-ai-") || stem.starts_with("git_ai") {
            return true;
        }
    }

    // Check if a sibling "git-ai" exists in the same directory AND both
    // refer to the same underlying file (hard-link, bind-mount, or copy
    // installed as a shim). This catches hard-linked shims that the
    // canonical-name check above misses, without false-positiving on
    // environments where a real git binary legitimately coexists with a
    // git-ai symlink.
    if let Some(parent) = path.parent() {
        let git_ai_name = if cfg!(windows) {
            "git-ai.exe"
        } else {
            "git-ai"
        };
        let sibling = parent.join(git_ai_name);
        if sibling.exists() && same_file(path, &sibling) {
            return true;
        }
    }

    false
}

/// Returns true if `p` is an executable git binary that is NOT git-ai.
/// Used by test infrastructure to probe for the real git binary independently
/// of `Config::get()` (which reads HOME and must not be called before HOME is isolated).
pub fn is_real_git_candidate(p: &Path) -> bool {
    is_executable(p) && !path_is_git_ai_binary(p)
}
