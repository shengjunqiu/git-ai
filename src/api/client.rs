use crate::auth::types::StoredCredentials;
use crate::auth::{CredentialStore, OAuthClient};
use crate::config;
use crate::error::GitAiError;
use crate::git::repository::{exec_git, parse_git_var_identity};
use crate::http;
use once_cell::sync::Lazy;
use std::sync::Mutex;
use url::Url;

/// Global mutex to prevent multiple threads from refreshing simultaneously.
/// This provides in-process synchronization to avoid thundering herd issues.
/// Cross-process synchronization is handled by re-reading the credential store
/// before refreshing and on the next API context construction.
static REFRESH_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn credentials_server_matches(
    credentials: &StoredCredentials,
    target_base_url: &str,
    configured_base_url: &str,
) -> bool {
    let target_base_url = config::normalize_api_base_url(target_base_url);
    match credentials.server_url.as_deref() {
        Some(server_url) => config::normalize_api_base_url(server_url) == target_base_url,
        // Legacy credentials predate server binding. Only trust them for the
        // currently configured server, which matches the old global behavior.
        None => config::normalize_api_base_url(configured_base_url) == target_base_url,
    }
}

fn load_credentials_for_server(
    store: &CredentialStore,
    target_base_url: &str,
) -> Option<StoredCredentials> {
    let mut credentials = store.load().ok()??;
    let configured_base_url = config::Config::fresh().api_base_url().to_string();
    if !credentials_server_matches(&credentials, target_base_url, &configured_base_url) {
        return None;
    }

    // Bind legacy credentials to the configured server on first use. Failure
    // to persist the migration should not discard an otherwise usable token.
    if credentials.server_url.is_none() {
        credentials.server_url = Some(config::normalize_api_base_url(target_base_url));
        let _ = store.store(&credentials);
    }

    Some(credentials)
}

/// Attempt to load stored credentials and refresh if needed.
/// Returns None on any failure (not logged in, expired, refresh failed).
/// Uses in-process Mutex for thread safety during token refresh.
fn try_load_auth_token(base_url: &str) -> Option<String> {
    let store = CredentialStore::new();
    let creds = load_credentials_for_server(&store, base_url)?;

    // If refresh token expired, can't authenticate
    if creds.is_refresh_token_expired() {
        return None;
    }

    // Fast path: if access token is valid (with 5 min buffer), use it directly
    if !creds.is_access_token_expired(300) {
        return Some(creds.access_token);
    }

    // Need to refresh - acquire mutex to prevent thundering herd within this process
    // If mutex is poisoned (previous panic), we return None gracefully
    let _guard = REFRESH_LOCK.lock().ok()?;

    // Re-check credentials after acquiring lock - another thread may have refreshed
    let creds = load_credentials_for_server(&store, base_url)?;

    if creds.is_refresh_token_expired() {
        return None;
    }

    // Check again if access token is now valid (another thread may have refreshed)
    if !creds.is_access_token_expired(300) {
        return Some(creds.access_token);
    }

    // Still expired - we need to refresh
    let client = OAuthClient::with_base_url(base_url).ok()?;
    match client.refresh_access_token(&creds.refresh_token) {
        Ok(new_creds) => {
            // Store refreshed credentials (ignore errors - we still have the token)
            let _ = store.store(&new_creds);
            Some(new_creds.access_token)
        }
        Err(error) => {
            tracing::warn!(%error, "OAuth access token refresh failed");
            None
        }
    }
    // Mutex guard is automatically released when _guard is dropped
}

/// Resolve the git author identity without requiring a Repository instance.
///
/// Runs `git var GIT_COMMITTER_IDENT` to get the current user's identity,
/// respecting the full git precedence chain (env vars > config > system defaults).
/// Returns `None` if the identity cannot be determined.
fn resolve_git_identity() -> Option<String> {
    let args = vec!["var".to_string(), "GIT_COMMITTER_IDENT".to_string()];
    if let Ok(output) = exec_git(&args)
        && let Ok(stdout) = String::from_utf8(output.stdout)
    {
        let identity = parse_git_var_identity(&stdout);
        if let Some(formatted) = identity.formatted() {
            return Some(formatted);
        }
    }
    None
}

/// API client context with optional authentication
#[derive(Clone)]
pub struct ApiContext {
    /// Base URL for the API (e.g., `https://app.com`)
    pub base_url: String,
    /// Optional authentication token
    pub auth_token: Option<String>,
    /// Optional API key for X-API-Key header
    pub api_key: Option<String>,
    /// Optional git author identity for X-Author-Identity header (only sent when API key is set)
    pub author_identity: Option<String>,
    /// Request timeout in seconds
    pub timeout_secs: Option<u64>,
}

impl std::fmt::Debug for ApiContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiContext")
            .field("base_url", &self.base_url)
            .field(
                "auth_token",
                &self.auth_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("author_identity", &self.author_identity)
            .field("timeout_secs", &self.timeout_secs)
            .finish()
    }
}

impl ApiContext {
    /// Get the default API base URL from config
    /// Uses Config::fresh() to support runtime config updates (daemon mode)
    fn default_base_url() -> String {
        config::Config::fresh().api_base_url().to_string()
    }

    /// Create a GET request with common headers (User-Agent, X-Distinct-ID)
    /// Use this for all HTTP GET requests to ensure consistent headers.
    /// The returned (Agent, Request) pair uses the system's native certificate store.
    pub fn http_get(url: &str, timeout_secs: Option<u64>) -> (ureq::Agent, ureq::Request) {
        let agent = http::build_agent(timeout_secs);
        let request = agent
            .get(url)
            .set(
                "User-Agent",
                &format!("git-ai/{}", env!("CARGO_PKG_VERSION")),
            )
            .set("X-Distinct-ID", &config::get_or_create_distinct_id());
        (agent, request)
    }

    /// Create a POST request with common headers (User-Agent, X-Distinct-ID)
    /// Use this for all HTTP POST requests to ensure consistent headers.
    /// The returned (Agent, Request) pair uses the system's native certificate store.
    pub fn http_post(url: &str, timeout_secs: Option<u64>) -> (ureq::Agent, ureq::Request) {
        let agent = http::build_agent(timeout_secs);
        let request = agent
            .post(url)
            .set(
                "User-Agent",
                &format!("git-ai/{}", env!("CARGO_PKG_VERSION")),
            )
            .set("X-Distinct-ID", &config::get_or_create_distinct_id());
        (agent, request)
    }

    /// Create a new API context, automatically using stored credentials if available
    /// If base_url is None, uses api_base_url from config (which can be set via config file, env var, or defaults)
    /// Uses Config::fresh() to support runtime config updates (daemon mode)
    pub fn new(base_url: Option<String>) -> Self {
        let cfg = config::Config::fresh();
        let api_key = cfg.api_key().map(|s| s.to_string());
        let author_identity = if api_key.is_some() {
            resolve_git_identity()
        } else {
            None
        };
        let base_url =
            config::normalize_api_base_url(&base_url.unwrap_or_else(Self::default_base_url));
        let auth_token = try_load_auth_token(&base_url);
        Self {
            base_url,
            auth_token,
            api_key,
            author_identity,
            timeout_secs: Some(30),
        }
    }

    /// Create a new API context explicitly without authentication
    /// Use this when you need to ensure no auth token is sent
    /// Uses Config::fresh() to support runtime config updates (daemon mode)
    #[allow(dead_code)]
    pub fn without_auth(base_url: Option<String>) -> Self {
        let cfg = config::Config::fresh();
        let api_key = cfg.api_key().map(|s| s.to_string());
        let author_identity = if api_key.is_some() {
            resolve_git_identity()
        } else {
            None
        };
        Self {
            base_url: config::normalize_api_base_url(
                &base_url.unwrap_or_else(Self::default_base_url),
            ),
            auth_token: None,
            api_key,
            author_identity,
            timeout_secs: Some(30),
        }
    }

    /// Create a new API context with authentication
    /// If base_url is None, uses api_base_url from config (which can be set via config file, env var, or defaults)
    /// Uses Config::fresh() to support runtime config updates (daemon mode)
    #[allow(dead_code)]
    pub fn with_auth(base_url: Option<String>, auth_token: String) -> Self {
        let cfg = config::Config::fresh();
        let api_key = cfg.api_key().map(|s| s.to_string());
        let author_identity = if api_key.is_some() {
            resolve_git_identity()
        } else {
            None
        };
        Self {
            base_url: config::normalize_api_base_url(
                &base_url.unwrap_or_else(Self::default_base_url),
            ),
            auth_token: Some(auth_token),
            api_key,
            author_identity,
            timeout_secs: Some(30),
        }
    }

    /// Set a custom timeout
    pub fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = Some(timeout_secs);
        self
    }

    /// Build the full URL for an endpoint
    fn build_url(&self, endpoint: &str) -> Result<String, GitAiError> {
        let base = Url::parse(&self.base_url)
            .map_err(|e| GitAiError::Generic(format!("Invalid base URL: {}", e)))?;
        let url = base
            .join(endpoint)
            .map_err(|e| GitAiError::Generic(format!("Invalid endpoint URL: {}", e)))?;
        Ok(url.to_string())
    }

    /// Make a POST request with JSON body
    pub fn post_json<T: serde::Serialize>(
        &self,
        endpoint: &str,
        body: &T,
    ) -> Result<http::Response, GitAiError> {
        let url = self.build_url(endpoint)?;
        let body_json = serde_json::to_string(body).map_err(GitAiError::JsonError)?;

        let (_agent, mut request) = Self::http_post(&url, self.timeout_secs);
        request = request.set("Content-Type", "application/json");

        if let Some(api_key) = &self.api_key {
            request = request.set("X-API-Key", api_key);
            if let Some(identity) = &self.author_identity {
                request = request.set("X-Author-Identity", identity);
            }
        }
        if let Some(token) = &self.auth_token {
            request = request.set("Authorization", &format!("Bearer {}", token));
        }

        http::send_with_body(request, &body_json)
            .map_err(|e| GitAiError::Generic(format!("HTTP request failed: {}", e)))
    }

    /// Make a GET request
    pub fn get(&self, endpoint: &str) -> Result<http::Response, GitAiError> {
        let url = self.build_url(endpoint)?;

        let (_agent, mut request) = Self::http_get(&url, self.timeout_secs);

        if let Some(api_key) = &self.api_key {
            request = request.set("X-API-Key", api_key);
            if let Some(identity) = &self.author_identity {
                request = request.set("X-Author-Identity", identity);
            }
        }
        if let Some(token) = &self.auth_token {
            request = request.set("Authorization", &format!("Bearer {}", token));
        }

        http::send(request).map_err(|e| GitAiError::Generic(format!("HTTP request failed: {}", e)))
    }
}

/// API client wrapper
#[derive(Debug, Clone)]
pub struct ApiClient {
    context: ApiContext,
}

impl ApiClient {
    /// Create a new API client with the given context
    pub fn new(context: ApiContext) -> Self {
        Self { context }
    }

    /// Get a reference to the API context
    pub fn context(&self) -> &ApiContext {
        &self.context
    }

    /// Get a mutable reference to the API context
    #[allow(dead_code)]
    pub fn context_mut(&mut self) -> &mut ApiContext {
        &mut self.context
    }

    /// Check if user is logged in (has an auth token)
    pub fn is_logged_in(&self) -> bool {
        self.context.auth_token.is_some()
    }

    /// Check if an API key is configured
    pub fn has_api_key(&self) -> bool {
        self.context.api_key.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_credentials(server_url: Option<&str>) -> StoredCredentials {
        StoredCredentials {
            server_url: server_url.map(str::to_string),
            access_token: "access-token".to_string(),
            refresh_token: "refresh-token".to_string(),
            access_token_expires_at: i64::MAX,
            refresh_token_expires_at: i64::MAX,
        }
    }

    // ============= ApiContext Tests =============

    #[test]
    fn test_credentials_only_match_their_issuing_server() {
        let credentials = test_credentials(Some("https://server-a.example.com/"));

        assert!(credentials_server_matches(
            &credentials,
            "https://server-a.example.com",
            "https://server-b.example.com"
        ));
        assert!(!credentials_server_matches(
            &credentials,
            "https://server-b.example.com",
            "https://server-b.example.com"
        ));
    }

    #[test]
    fn test_legacy_credentials_only_match_configured_server() {
        let credentials = test_credentials(None);

        assert!(credentials_server_matches(
            &credentials,
            "https://configured.example.com/",
            "https://configured.example.com"
        ));
        assert!(!credentials_server_matches(
            &credentials,
            "https://other.example.com",
            "https://configured.example.com"
        ));
    }

    #[test]
    fn test_api_context_without_auth() {
        let ctx = ApiContext::without_auth(Some("https://example.com".to_string()));
        assert!(ctx.auth_token.is_none());
        assert_eq!(ctx.base_url, "https://example.com");
    }

    #[test]
    fn test_api_context_with_auth() {
        let ctx = ApiContext::with_auth(
            Some("https://example.com".to_string()),
            "test_token".to_string(),
        );
        assert_eq!(ctx.auth_token, Some("test_token".to_string()));
        assert_eq!(ctx.base_url, "https://example.com");
    }

    #[test]
    fn test_api_context_with_timeout() {
        let ctx =
            ApiContext::without_auth(Some("https://example.com".to_string())).with_timeout(60);
        assert_eq!(ctx.timeout_secs, Some(60));
    }

    #[test]
    fn test_api_context_default_timeout() {
        let ctx = ApiContext::without_auth(Some("https://example.com".to_string()));
        assert_eq!(ctx.timeout_secs, Some(30));
    }

    // ============= ApiClient Tests =============

    #[test]
    fn test_api_client_is_logged_in_true() {
        let ctx =
            ApiContext::with_auth(Some("https://example.com".to_string()), "token".to_string());
        let client = ApiClient::new(ctx);
        assert!(client.is_logged_in());
    }

    #[test]
    fn test_api_client_is_logged_in_false() {
        let ctx = ApiContext::without_auth(Some("https://example.com".to_string()));
        let client = ApiClient::new(ctx);
        assert!(!client.is_logged_in());
    }

    #[test]
    fn test_api_client_context_access() {
        let ctx =
            ApiContext::with_auth(Some("https://example.com".to_string()), "token".to_string());
        let client = ApiClient::new(ctx);
        assert_eq!(client.context().base_url, "https://example.com");
    }

    // ============= URL Building Tests =============

    #[test]
    fn test_build_url_simple() {
        let ctx = ApiContext::without_auth(Some("https://example.com".to_string()));
        let url = ctx.build_url("/api/test").unwrap();
        assert_eq!(url, "https://example.com/api/test");
    }

    #[test]
    fn test_build_url_with_trailing_slash() {
        let ctx = ApiContext::without_auth(Some("https://example.com/".to_string()));
        let url = ctx.build_url("api/test").unwrap();
        assert_eq!(url, "https://example.com/api/test");
    }

    #[test]
    fn test_build_url_invalid_base() {
        let ctx = ApiContext::without_auth(Some("not-a-url".to_string()));
        let result = ctx.build_url("/api/test");
        assert!(result.is_err());
    }

    // ============= Mutex Thread Safety Tests =============

    #[test]
    fn test_mutex_is_accessible() {
        // Simple test to verify the mutex can be locked
        let guard = REFRESH_LOCK.lock();
        assert!(guard.is_ok());
        // Guard drops here, releasing the lock
    }

    #[test]
    fn test_concurrent_access_to_mutex() {
        // Test that multiple threads can safely contend for the mutex
        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = vec![];

        for _ in 0..5 {
            let counter_clone = counter.clone();
            let handle = std::thread::spawn(move || {
                if let Ok(_guard) = REFRESH_LOCK.lock() {
                    counter_clone.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // All threads should have acquired the lock sequentially
        let final_count = counter.load(Ordering::SeqCst);
        assert_eq!(final_count, 5);
    }

    // ============= Metrics/CAS Upload URL & Auth Tests =============

    /// Test that the metrics upload endpoint URL is correctly constructed
    #[test]
    fn test_metrics_upload_url() {
        let ctx = ApiContext::without_auth(Some("https://enterprise.example.com".to_string()));
        let url = ctx.build_url("/worker/metrics/upload").unwrap();
        assert_eq!(url, "https://enterprise.example.com/worker/metrics/upload");
    }

    /// Test that the CAS upload endpoint URL is correctly constructed
    #[test]
    fn test_cas_upload_url() {
        let ctx = ApiContext::without_auth(Some("https://enterprise.example.com".to_string()));
        let url = ctx.build_url("/worker/cas/upload").unwrap();
        assert_eq!(url, "https://enterprise.example.com/worker/cas/upload");
    }

    /// Test that the CAS read endpoint URL is correctly constructed
    #[test]
    fn test_cas_read_url() {
        let ctx = ApiContext::without_auth(Some("https://enterprise.example.com".to_string()));
        let url = ctx.build_url("/worker/cas/?hashes=abc123,def456").unwrap();
        assert_eq!(
            url,
            "https://enterprise.example.com/worker/cas/?hashes=abc123,def456"
        );
    }

    /// Test that an ApiContext with Bearer token includes auth header fields
    #[test]
    fn test_context_with_bearer_token_has_auth() {
        let ctx = ApiContext::with_auth(
            Some("https://enterprise.example.com".to_string()),
            "my_access_token".to_string(),
        );
        assert!(ctx.auth_token.is_some());
        assert_eq!(ctx.auth_token.as_deref(), Some("my_access_token"));
    }

    /// Test that an ApiContext with API key includes X-API-Key header field
    #[test]
    fn test_context_with_api_key_has_key() {
        let mut ctx = ApiContext::without_auth(Some("https://enterprise.example.com".to_string()));
        ctx.api_key = Some("my-api-key".to_string());
        assert!(ctx.api_key.is_some());
        assert_eq!(ctx.api_key.as_deref(), Some("my-api-key"));
    }

    /// Test that both Bearer token AND API key can coexist
    /// (this is the case when user is both logged in and has an API key)
    #[test]
    fn test_context_with_both_auth_methods() {
        let mut ctx = ApiContext::with_auth(
            Some("https://enterprise.example.com".to_string()),
            "access_token".to_string(),
        );
        ctx.api_key = Some("my-api-key".to_string());
        assert!(ctx.auth_token.is_some());
        assert!(ctx.api_key.is_some());
    }

    /// Test that the enterprise server URL format works for default endpoints
    #[test]
    fn test_enterprise_server_url_format() {
        // Common enterprise server URL patterns
        let urls = vec![
            "https://git-ai.internal.company.com",
            "https://api.gitai.example.com:8443",
            "http://localhost:3000",
        ];
        for url in urls {
            let ctx = ApiContext::without_auth(Some(url.to_string()));
            let metrics_url = ctx.build_url("/worker/metrics/upload").unwrap();
            assert!(
                metrics_url.starts_with(url),
                "Metrics URL should start with base: {}",
                url
            );
            assert!(metrics_url.ends_with("/worker/metrics/upload"));

            let cas_url = ctx.build_url("/worker/cas/upload").unwrap();
            assert!(
                cas_url.starts_with(url),
                "CAS URL should start with base: {}",
                url
            );
            assert!(cas_url.ends_with("/worker/cas/upload"));
        }
    }
}
