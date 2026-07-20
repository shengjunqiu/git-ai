use crate::error::GitAiError;
use crate::git::repository::Repository;
use std::path::Path;
use url::Url;

const LOCAL_REPOSITORY_PREFIX: &str = "local/";

/// Normalize repo URL to canonical HTTPS format
/// Accepts: HTTPS, HTTP, SSH (scp-like user@host:path or ssh://), git:// URLs
/// Returns: Canonical HTTPS URL without credentials, .git suffix, or trailing slash
pub fn normalize_repo_url(url_str: &str) -> Result<String, String> {
    let url_str = url_str.trim();

    // Handle SSH scp-like format: user@host:path
    if !url_str.contains("://")
        && let Some((user_host, path)) = url_str.split_once(':')
        && let Some((_, host)) = user_host.rsplit_once('@')
    {
        return normalize_ssh_url(host, path);
    }

    // Parse as URL
    let url = Url::parse(url_str).map_err(|e| format!("Invalid URL: {}", e))?;

    // Validate scheme
    let scheme = url.scheme();
    if !["https", "http", "git", "ssh"].contains(&scheme) {
        return Err(format!("Unsupported URL scheme: {}", scheme));
    }

    // Extract host
    let host = url.host_str().ok_or("URL must have a host")?;

    // Normalize path: remove .git suffix and trailing slash
    let path = url.path().trim_end_matches('/').trim_end_matches(".git");

    // Build canonical HTTPS URL
    let canonical = format!("https://{}{}", host, path);

    // Validate the normalized URL
    validate_normalized_url(&canonical)?;

    Ok(canonical)
}

/// Resolve the stable identifier sent in the metrics `repo_url` field.
///
/// A normalized remote URL remains the preferred cross-clone identity. Repositories
/// without a remote fall back to a namespaced local directory name so the server can
/// display them as projects without receiving the full local filesystem path.
pub fn repository_identifier(repo: &Repository) -> Result<Option<String>, GitAiError> {
    let remote_url = if let Some(default_remote) = repo.get_default_remote()? {
        repo.remotes_with_urls()?
            .into_iter()
            .find(|(name, _)| name == &default_remote)
            .map(|(_, url)| url)
    } else {
        None
    };

    Ok(repository_identifier_from_parts(
        remote_url.as_deref(),
        &repo.workdir()?,
    ))
}

fn repository_identifier_from_parts(remote_url: Option<&str>, workdir: &Path) -> Option<String> {
    match remote_url {
        Some(url) => normalize_repo_url(url).ok(),
        None => local_repository_identifier(workdir),
    }
}

fn local_repository_identifier(workdir: &Path) -> Option<String> {
    workdir
        .file_name()
        .map(|name| name.to_string_lossy())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .map(|name| format!("{LOCAL_REPOSITORY_PREFIX}{name}"))
}

/// Validate that normalized URL is a proper HTTPS URL
fn validate_normalized_url(url_str: &str) -> Result<(), String> {
    let url = Url::parse(url_str).map_err(|e| format!("Failed to parse normalized URL: {}", e))?;

    if url.scheme() != "https" {
        return Err("Normalized URL must be HTTPS".to_string());
    }

    if url.host_str().is_none() {
        return Err("Normalized URL must have a valid host".to_string());
    }

    // Ensure path is not empty (at minimum /)
    if url.path().is_empty() || url.path() == "/" {
        return Err("Normalized URL must have a path (repo identifier)".to_string());
    }

    Ok(())
}

/// Normalize SSH scp-like URL (user@host:path) to HTTPS
fn normalize_ssh_url(host: &str, path: &str) -> Result<String, String> {
    if host.is_empty() || path.is_empty() {
        return Err("Invalid SSH URL format".to_string());
    }

    // Normalize path
    let path = path
        .trim_start_matches('/')
        .trim_end_matches('/')
        .trim_end_matches(".git");

    let canonical = format!("https://{}/{}", host, path);

    // Validate the normalized URL
    validate_normalized_url(&canonical)?;

    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::{normalize_repo_url, repository_identifier_from_parts};
    use std::path::Path;

    #[test]
    fn test_repository_identifier_prefers_normalized_remote_url() {
        assert_eq!(
            repository_identifier_from_parts(
                Some("git@github.com:linewell/git-ai-test.git"),
                Path::new("/work/local-copy"),
            )
            .as_deref(),
            Some("https://github.com/linewell/git-ai-test")
        );
    }

    #[test]
    fn test_repository_identifier_falls_back_to_namespaced_directory_name() {
        assert_eq!(
            repository_identifier_from_parts(None, Path::new("/work/git-ai-test")).as_deref(),
            Some("local/git-ai-test")
        );
    }

    #[test]
    fn test_repository_identifier_does_not_fallback_for_invalid_configured_remote() {
        assert_eq!(
            repository_identifier_from_parts(
                Some("not-a-valid-remote"),
                Path::new("/work/git-ai-test"),
            ),
            None
        );
    }

    #[test]
    fn test_normalize_repo_url_https() {
        assert_eq!(
            normalize_repo_url("https://github.com/user/repo").unwrap(),
            "https://github.com/user/repo"
        );
        assert_eq!(
            normalize_repo_url("https://github.com/user/repo.git").unwrap(),
            "https://github.com/user/repo"
        );
        assert_eq!(
            normalize_repo_url("https://github.com/user/repo/").unwrap(),
            "https://github.com/user/repo"
        );
        assert_eq!(
            normalize_repo_url("https://gitlab.com/group/subgroup/repo.git/").unwrap(),
            "https://gitlab.com/group/subgroup/repo"
        );
    }

    #[test]
    fn test_normalize_repo_url_ssh() {
        assert_eq!(
            normalize_repo_url("git@github.com:user/repo.git").unwrap(),
            "https://github.com/user/repo"
        );
        assert_eq!(
            normalize_repo_url("ssh://git@github.com/user/repo.git").unwrap(),
            "https://github.com/user/repo"
        );
        assert_eq!(
            normalize_repo_url("alice@github.com:org/repo").unwrap(),
            "https://github.com/org/repo"
        );
        assert_eq!(
            normalize_repo_url("git@gitlab.com:group/subgroup/repo").unwrap(),
            "https://gitlab.com/group/subgroup/repo"
        );
        assert_eq!(
            normalize_repo_url("git@bitbucket.org:user/repo.git").unwrap(),
            "https://bitbucket.org/user/repo"
        );
    }

    #[test]
    fn test_normalize_repo_url_git_protocol() {
        assert_eq!(
            normalize_repo_url("git://github.com/user/repo.git").unwrap(),
            "https://github.com/user/repo"
        );
    }

    #[test]
    fn test_normalize_repo_url_http_upgrade() {
        assert_eq!(
            normalize_repo_url("http://github.com/user/repo").unwrap(),
            "https://github.com/user/repo"
        );
        assert_eq!(
            normalize_repo_url("https://token@github.com/user/repo").unwrap(),
            "https://github.com/user/repo"
        );
    }

    #[test]
    fn test_normalize_repo_url_invalid() {
        assert!(normalize_repo_url("not-a-url").is_err());
        assert!(normalize_repo_url("https://").is_err());
        assert!(normalize_repo_url("ftp://example.com/repo").is_err());
        assert!(normalize_repo_url("git@github.com").is_err()); // missing :path
    }

    #[test]
    fn test_normalize_repo_url_ssh_scp_edge_cases() {
        // SSH URL with leading slash in path
        assert_eq!(
            normalize_repo_url("git@github.com:/user/repo").unwrap(),
            "https://github.com/user/repo"
        );

        // SSH URL with multiple path segments
        assert_eq!(
            normalize_repo_url("git@gitlab.example.com:group/subgroup/nested/repo").unwrap(),
            "https://gitlab.example.com/group/subgroup/nested/repo"
        );
    }

    #[test]
    fn test_normalize_repo_url_empty_or_invalid_ssh() {
        // Missing path after colon
        let result = normalize_repo_url("git@github.com:");
        assert!(result.is_err());

        // Empty string
        let result = normalize_repo_url("");
        assert!(result.is_err());

        // Only whitespace
        let result = normalize_repo_url("   ");
        assert!(result.is_err());
    }

    #[test]
    fn test_normalize_repo_url_with_credentials() {
        // HTTPS with user credentials should strip them
        assert_eq!(
            normalize_repo_url("https://user:pass@github.com/user/repo").unwrap(),
            "https://github.com/user/repo"
        );

        // HTTPS with token
        assert_eq!(
            normalize_repo_url("https://oauth2:token123@gitlab.com/user/repo").unwrap(),
            "https://gitlab.com/user/repo"
        );
    }

    #[test]
    fn test_normalize_repo_url_with_port() {
        // HTTPS with custom port
        assert_eq!(
            normalize_repo_url("https://github.com:443/user/repo").unwrap(),
            "https://github.com/user/repo"
        );

        // SSH URL with port
        assert_eq!(
            normalize_repo_url("ssh://git@github.com:22/user/repo.git").unwrap(),
            "https://github.com/user/repo"
        );
    }

    #[test]
    fn test_normalize_repo_url_no_path() {
        // URL with no path (just host)
        let result = normalize_repo_url("https://github.com");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("path"));

        // URL with only slash
        let result = normalize_repo_url("https://github.com/");
        assert!(result.is_err());
    }

    #[test]
    fn test_normalize_repo_url_complex_paths() {
        // Multiple .git suffixes (strips all at the end)
        assert_eq!(
            normalize_repo_url("https://github.com/user/repo.git.git").unwrap(),
            "https://github.com/user/repo"
        );

        // Path with underscores and dashes
        assert_eq!(
            normalize_repo_url("https://github.com/my-org/my_repo-123").unwrap(),
            "https://github.com/my-org/my_repo-123"
        );

        // Path with dots (not .git)
        assert_eq!(
            normalize_repo_url("https://github.com/user/repo.v2").unwrap(),
            "https://github.com/user/repo.v2"
        );

        // Nested paths
        assert_eq!(
            normalize_repo_url("https://gitlab.com/group/subgroup/project.git").unwrap(),
            "https://gitlab.com/group/subgroup/project"
        );
    }

    #[test]
    fn test_validate_normalized_url() {
        use super::validate_normalized_url;

        // Valid HTTPS URL with path
        assert!(validate_normalized_url("https://github.com/user/repo").is_ok());

        // Missing HTTPS scheme
        assert!(validate_normalized_url("http://github.com/user/repo").is_err());

        // No path
        assert!(validate_normalized_url("https://github.com").is_err());
        assert!(validate_normalized_url("https://github.com/").is_err());
    }

    #[test]
    fn test_normalize_ssh_url_edge_cases() {
        use super::normalize_ssh_url;

        // Valid SSH path with trailing slash
        assert_eq!(
            normalize_ssh_url("github.com", "user/repo/").unwrap(),
            "https://github.com/user/repo"
        );

        // Empty host
        assert!(normalize_ssh_url("", "user/repo").is_err());

        // Empty path
        assert!(normalize_ssh_url("github.com", "").is_err());

        // Path with .git suffix
        assert_eq!(
            normalize_ssh_url("gitlab.com", "group/repo.git").unwrap(),
            "https://gitlab.com/group/repo"
        );
    }

    #[test]
    fn test_normalize_repo_url_whitespace_handling() {
        // Leading/trailing whitespace
        assert_eq!(
            normalize_repo_url("  https://github.com/user/repo  ").unwrap(),
            "https://github.com/user/repo"
        );

        // Whitespace around SSH URL
        assert_eq!(
            normalize_repo_url("  git@github.com:user/repo.git  ").unwrap(),
            "https://github.com/user/repo"
        );
    }

    #[test]
    fn test_normalize_repo_url_unsupported_schemes() {
        assert!(normalize_repo_url("ftp://example.com/repo").is_err());
        assert!(normalize_repo_url("file:///local/path").is_err());
        assert!(normalize_repo_url("svn://example.com/repo").is_err());
    }
}
