use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

use crate::config::AppConfig;
use crate::error::AppError;
use crate::routes::AppState;

const REDIS_RATE_LIMIT_SCRIPT: &str = r#"
local current = redis.call('INCR', KEYS[1])
if current == 1 then
  redis.call('EXPIRE', KEYS[1], ARGV[1])
end
return current
"#;

/// Rate limit tier configuration: (max_requests, window_seconds)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitTier {
    pub max_requests: u32,
    pub window_seconds: u64,
}

impl RateLimitTier {
    pub const fn new(max_requests: u32, window_seconds: u64) -> Self {
        Self {
            max_requests,
            window_seconds,
        }
    }
}

/// Redis-backed fixed-window rate limiter.
/// Falls back to in-memory counters when Redis is unavailable.
#[derive(Clone)]
pub struct RateLimiter {
    redis: Option<redis::aio::ConnectionManager>,
    /// Per-tier counters: tier_name -> (key -> (count, window_start, window_seconds))
    counters: Arc<RwLock<HashMap<String, HashMap<String, (u32, Instant, u64)>>>>,
}

impl fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimiter")
            .field("redis", &self.redis.is_some())
            .finish_non_exhaustive()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            redis: None,
            counters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn with_redis(redis: redis::Client) -> Self {
        let redis = match tokio::time::timeout(
            Duration::from_secs(1),
            redis.get_connection_manager(),
        )
        .await
        {
            Ok(Ok(manager)) => Some(manager),
            Ok(Err(error)) => {
                tracing::warn!(
                    "Redis rate limit unavailable at startup; using in-memory counters: {}",
                    error
                );
                None
            }
            Err(_) => {
                tracing::warn!(
                    "Redis rate limit connection timed out at startup; using in-memory counters"
                );
                None
            }
        };

        Self {
            redis,
            counters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check if a request is allowed for the given tier and client key.
    /// Returns Ok(()) if allowed, Err if rate limited.
    pub async fn check(&self, tier: &str, key: &str, limit: RateLimitTier) -> Result<(), AppError> {
        if let Some(redis) = &self.redis {
            match self.check_redis(redis, tier, key, limit).await {
                Ok(count) => return check_count("redis", tier, key, count, limit),
                Err(error) => {
                    tracing::warn!(
                        "Redis rate limit unavailable; falling back to in-memory counters: {}",
                        error
                    );
                }
            }
        }

        self.check_in_memory(tier, key, limit).await
    }

    async fn check_redis(
        &self,
        redis: &redis::aio::ConnectionManager,
        tier: &str,
        key: &str,
        limit: RateLimitTier,
    ) -> Result<i64, redis::RedisError> {
        let redis_key = redis_rate_limit_key(tier, key, limit.window_seconds);
        let mut connection = redis.clone();
        redis::Script::new(REDIS_RATE_LIMIT_SCRIPT)
            .key(redis_key)
            .arg(limit.window_seconds.max(1))
            .invoke_async(&mut connection)
            .await
    }

    async fn check_in_memory(
        &self,
        tier: &str,
        key: &str,
        limit: RateLimitTier,
    ) -> Result<(), AppError> {
        let mut counters = self.counters.write().await;
        let now = Instant::now();
        let window_seconds = limit.window_seconds.max(1);
        let window_duration = Duration::from_secs(window_seconds);

        let tier_map = counters.entry(tier.to_string()).or_default();
        let entry = tier_map
            .entry(key.to_string())
            .or_insert((0, now, window_seconds));

        // Reset counter if window has expired
        if now.duration_since(entry.1) > window_duration {
            *entry = (0, now, window_seconds);
        } else {
            entry.2 = window_seconds;
        }

        entry.0 += 1;

        check_count("memory", tier, key, i64::from(entry.0), limit)
    }

    /// Clean up expired entries across all tiers (call periodically)
    pub async fn cleanup(&self) {
        let mut counters = self.counters.write().await;
        let now = Instant::now();
        for tier_map in counters.values_mut() {
            tier_map.retain(|_, (count, start, window_seconds)| {
                let window = Duration::from_secs((*window_seconds).max(1));
                *count > 0 && now.duration_since(*start) <= window * 2
            });
        }
    }
}

fn check_count(
    backend: &str,
    tier: &str,
    key: &str,
    count: i64,
    limit: RateLimitTier,
) -> Result<(), AppError> {
    if count > i64::from(limit.max_requests) {
        tracing::warn!(
            "Rate limit exceeded: backend={} tier={} key={} count={} limit={}",
            backend,
            tier,
            key,
            count,
            limit.max_requests
        );
        return Err(AppError::RateLimited(format!(
            "Rate limit exceeded. Maximum {} requests per {} seconds.",
            limit.max_requests, limit.window_seconds
        )));
    }

    Ok(())
}

fn redis_rate_limit_key(tier: &str, key: &str, window_seconds: u64) -> String {
    let window_start = current_window_start(window_seconds.max(1));
    format!("git-ai:rate-limit:{}:{}:{}", tier, key, window_start)
}

fn current_window_start(window_seconds: u64) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    (now / window_seconds) * window_seconds
}

const ADMIN_WRITE_MAX_REQUESTS: u32 = 60;

fn should_bypass_rate_limit(path: &str) -> bool {
    matches!(path, "/health" | "/ready")
}

/// Determine rate limit tier from request path
fn tier_for_request(
    config: &AppConfig,
    method: &Method,
    path: &str,
) -> (&'static str, RateLimitTier) {
    if path.starts_with("/worker/metrics") {
        (
            "metrics",
            RateLimitTier::new(
                config.rate_limit_metrics_max_requests,
                config.rate_limit_metrics_window_seconds,
            ),
        )
    } else if path.starts_with("/worker/cas/upload") {
        (
            "cas_upload",
            RateLimitTier::new(
                config.rate_limit_cas_upload_max_requests,
                config.rate_limit_cas_upload_window_seconds,
            ),
        )
    } else if path.starts_with("/worker/cas") {
        (
            "cas_read",
            RateLimitTier::new(
                config.rate_limit_cas_read_max_requests,
                config.rate_limit_cas_read_window_seconds,
            ),
        )
    } else if path.starts_with("/worker/oauth") {
        (
            "oauth",
            RateLimitTier::new(
                config.rate_limit_oauth_max_requests,
                config.rate_limit_oauth_window_seconds,
            ),
        )
    } else if path.starts_with("/api/admin") && is_write_method(method) {
        (
            "admin_write",
            RateLimitTier::new(
                config
                    .rate_limit_admin_max_requests
                    .min(ADMIN_WRITE_MAX_REQUESTS),
                config.rate_limit_admin_window_seconds,
            ),
        )
    } else if path.starts_with("/api/admin") {
        (
            "admin",
            RateLimitTier::new(
                config.rate_limit_admin_max_requests,
                config.rate_limit_admin_window_seconds,
            ),
        )
    } else if is_auth_path(path) {
        (
            "auth",
            RateLimitTier::new(
                config.rate_limit_auth_max_requests,
                config.rate_limit_auth_window_seconds,
            ),
        )
    } else {
        (
            "default",
            RateLimitTier::new(
                config.rate_limit_default_max_requests,
                config.rate_limit_default_window_seconds,
            ),
        )
    }
}

fn is_write_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

fn is_auth_path(path: &str) -> bool {
    matches!(path, "/login" | "/logout" | "/verify") || path.starts_with("/auth/")
}

/// Extract a client identifier from the request.
/// Prefers browser session user, then X-API-Key, Authorization header, then IP address.
async fn extract_client_key(parts: ClientKeyParts, state: &AppState) -> String {
    if let Some(user_id) =
        extract_browser_session_user_id(parts.web_session_token.as_deref(), state).await
    {
        return format!("user:{}", user_id);
    }

    if let Some(api_key) = parts.api_key {
        return format!("key:{}", &api_key[..api_key.len().min(8)]);
    }
    if let Some(auth) = parts.authorization {
        return format!("auth:{}", &auth[..auth.len().min(24)]);
    }
    if let Some(ip) = parts.forwarded_ip {
        return format!("ip:{}", ip);
    }
    "anonymous".to_string()
}

async fn extract_browser_session_user_id(
    session_token: Option<&str>,
    state: &AppState,
) -> Option<uuid::Uuid> {
    let session_token = session_token?;
    match crate::services::sessions::load_web_session_user(&state.db, &session_token).await {
        Ok(user_id) => user_id,
        Err(error) => {
            tracing::debug!(%error, "failed to load web session for rate limit key");
            None
        }
    }
}

struct ClientKeyParts {
    web_session_token: Option<String>,
    api_key: Option<String>,
    authorization: Option<String>,
    forwarded_ip: Option<String>,
}

fn client_key_parts(request: &Request) -> ClientKeyParts {
    ClientKeyParts {
        web_session_token: cookie_value(request, crate::services::sessions::WEB_SESSION_COOKIE),
        api_key: header_value(request, "X-API-Key"),
        authorization: header_value(request, "Authorization"),
        forwarded_ip: header_value(request, "X-Forwarded-For")
            .or_else(|| header_value(request, "X-Real-IP")),
    }
}

fn header_value(request: &Request, name: &str) -> Option<String> {
    request
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
}

fn cookie_value(request: &Request, name: &str) -> Option<String> {
    let cookie_header = request.headers().get("Cookie")?;
    let cookie_str = cookie_header.to_str().ok()?;

    cookie_str.split(';').find_map(|cookie| {
        let cookie = cookie.trim();
        let (cookie_name, cookie_value) = cookie.split_once('=')?;
        if cookie_name == name {
            Some(cookie_value.to_string())
        } else {
            None
        }
    })
}

/// Rate limiting middleware using AppState's RateLimiter
pub async fn rate_limit_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let path = request.uri().path().to_string();
    if should_bypass_rate_limit(&path) {
        return Ok(next.run(request).await);
    }

    let (tier_name, tier_limit) = tier_for_request(&state.config, request.method(), &path);
    let key_parts = client_key_parts(&request);
    let client_key = extract_client_key(key_parts, &state).await;

    state
        .rate_limiter
        .check(tier_name, &client_key, tier_limit)
        .await?;

    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn in_memory_limiter_enforces_limit() -> anyhow::Result<()> {
        let limiter = RateLimiter::new();
        let limit = RateLimitTier::new(1, 60);
        let key = format!("memory-{}", Uuid::new_v4());

        limiter.check("test", &key, limit).await?;
        let result = limiter.check("test", &key, limit).await;

        assert!(matches!(result, Err(AppError::RateLimited(_))));
        Ok(())
    }

    #[tokio::test]
    async fn redis_limiter_shares_counts_across_instances() -> anyhow::Result<()> {
        let Some(client) = redis_test_client().await? else {
            return Ok(());
        };
        let first = RateLimiter::with_redis(client.clone()).await;
        let second = RateLimiter::with_redis(client).await;
        let limit = RateLimitTier::new(1, 60);
        let key = format!("redis-shared-{}", Uuid::new_v4());

        first.check("test", &key, limit).await?;
        let result = second.check("test", &key, limit).await;

        assert!(matches!(result, Err(AppError::RateLimited(_))));
        Ok(())
    }

    #[tokio::test]
    async fn redis_failure_falls_back_to_in_memory_limit() -> anyhow::Result<()> {
        let client = redis::Client::open("redis://127.0.0.1:1/")?;
        let limiter = RateLimiter::with_redis(client).await;
        let limit = RateLimitTier::new(1, 60);
        let key = format!("redis-fallback-{}", Uuid::new_v4());

        limiter.check("test", &key, limit).await?;
        let result = limiter.check("test", &key, limit).await;

        assert!(matches!(result, Err(AppError::RateLimited(_))));
        Ok(())
    }

    #[test]
    fn health_and_readiness_bypass_rate_limits() {
        assert!(should_bypass_rate_limit("/health"));
        assert!(should_bypass_rate_limit("/ready"));
        assert!(!should_bypass_rate_limit("/worker/metrics/upload"));
        assert!(!should_bypass_rate_limit("/api/admin/users/list"));
    }

    #[test]
    fn tier_for_path_assigns_configured_limits() {
        let config = test_config();

        assert_path_tier(
            &config,
            Method::POST,
            "/worker/oauth/device/code",
            "oauth",
            RateLimitTier::new(601, 61),
        );
        assert_path_tier(
            &config,
            Method::POST,
            "/worker/oauth/token",
            "oauth",
            RateLimitTier::new(601, 61),
        );
        assert_path_tier(
            &config,
            Method::POST,
            "/auth/login",
            "auth",
            RateLimitTier::new(301, 31),
        );
        assert_path_tier(
            &config,
            Method::POST,
            "/auth/register",
            "auth",
            RateLimitTier::new(301, 31),
        );
        assert_path_tier(
            &config,
            Method::GET,
            "/auth/organizations",
            "auth",
            RateLimitTier::new(301, 31),
        );
        assert_path_tier(
            &config,
            Method::GET,
            "/login",
            "auth",
            RateLimitTier::new(301, 31),
        );
        assert_path_tier(
            &config,
            Method::GET,
            "/verify",
            "auth",
            RateLimitTier::new(301, 31),
        );
        assert_path_tier(
            &config,
            Method::POST,
            "/worker/metrics/upload",
            "metrics",
            RateLimitTier::new(60, 10),
        );
        assert_path_tier(
            &config,
            Method::GET,
            "/api/admin/users/list",
            "admin",
            RateLimitTier::new(300, 60),
        );
        assert_path_tier(
            &config,
            Method::POST,
            "/api/admin/departments",
            "admin_write",
            RateLimitTier::new(60, 60),
        );
        assert_path_tier(
            &config,
            Method::DELETE,
            "/api/admin/departments/00000000-0000-0000-0000-000000000000",
            "admin_write",
            RateLimitTier::new(60, 60),
        );
        assert_path_tier(
            &config,
            Method::GET,
            "/api/other",
            "default",
            RateLimitTier::new(300, 300),
        );
    }

    fn assert_path_tier(
        config: &AppConfig,
        method: Method,
        path: &str,
        expected_name: &'static str,
        expected_limit: RateLimitTier,
    ) {
        let (name, limit) = tier_for_request(config, &method, path);
        assert_eq!(name, expected_name);
        assert_eq!(limit, expected_limit);
    }

    fn test_config() -> AppConfig {
        AppConfig {
            database_url: String::new(),
            database_max_connections: 20,
            database_min_connections: 1,
            database_acquire_timeout_seconds: 5,
            redis_url: String::new(),
            jwt_secret: "test-secret".to_string(),
            s3_endpoint: String::new(),
            s3_bucket: String::new(),
            s3_access_key: String::new(),
            s3_secret_key: String::new(),
            s3_region: String::new(),
            cas_upload_concurrency: 8,
            auth_password_concurrency: 8,
            metrics_rollup_write_mode: crate::config::MetricsRollupWriteMode::Sync,
            metrics_rollup_worker_enabled: false,
            metrics_rollup_worker_interval_seconds: 5,
            metrics_rollup_worker_batch_size: 100,
            dashboard_use_rollups: true,
            rate_limit_metrics_max_requests: 60,
            rate_limit_metrics_window_seconds: 10,
            rate_limit_cas_upload_max_requests: 30,
            rate_limit_cas_upload_window_seconds: 20,
            rate_limit_cas_read_max_requests: 100,
            rate_limit_cas_read_window_seconds: 25,
            rate_limit_oauth_max_requests: 601,
            rate_limit_oauth_window_seconds: 61,
            rate_limit_auth_max_requests: 301,
            rate_limit_auth_window_seconds: 31,
            rate_limit_admin_max_requests: 300,
            rate_limit_admin_window_seconds: 60,
            rate_limit_default_max_requests: 300,
            rate_limit_default_window_seconds: 300,
            base_url: String::new(),
            sentry_dsn: String::new(),
            posthog_host: String::new(),
            posthog_api_key: String::new(),
        }
    }

    async fn redis_test_client() -> anyhow::Result<Option<redis::Client>> {
        dotenvy::dotenv().ok();
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        let client = redis::Client::open(redis_url)?;
        let mut connection = match client.get_multiplexed_async_connection().await {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("skipping Redis rate limit test: could not connect to Redis: {error}");
                return Ok(None);
            }
        };

        let ping: redis::RedisResult<String> =
            redis::cmd("PING").query_async(&mut connection).await;
        match ping {
            Ok(_) => Ok(Some(client)),
            Err(error) => {
                eprintln!("skipping Redis rate limit test: Redis ping failed: {error}");
                Ok(None)
            }
        }
    }
}
