use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

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
#[derive(Debug, Clone, Copy)]
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

/// Well-known rate limit tiers
pub mod tiers {
    use super::RateLimitTier;
    pub const METRICS: RateLimitTier = RateLimitTier::new(60, 60);
    pub const CAS_UPLOAD: RateLimitTier = RateLimitTier::new(30, 60);
    pub const CAS_READ: RateLimitTier = RateLimitTier::new(100, 60);
    pub const OAUTH: RateLimitTier = RateLimitTier::new(10, 60);
    pub const ADMIN: RateLimitTier = RateLimitTier::new(30, 60);
    pub const DEFAULT: RateLimitTier = RateLimitTier::new(120, 60);
}

/// Redis-backed fixed-window rate limiter.
/// Falls back to in-memory counters when Redis is unavailable.
#[derive(Clone)]
pub struct RateLimiter {
    redis: Option<redis::aio::ConnectionManager>,
    /// Per-tier counters: tier_name -> (key -> (count, window_start))
    counters: Arc<RwLock<HashMap<String, HashMap<String, (u32, Instant)>>>>,
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
        let redis =
            match tokio::time::timeout(Duration::from_secs(1), redis.get_connection_manager())
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
        let window_duration = Duration::from_secs(limit.window_seconds.max(1));

        let tier_map = counters.entry(tier.to_string()).or_default();
        let entry = tier_map.entry(key.to_string()).or_insert((0, now));

        // Reset counter if window has expired
        if now.duration_since(entry.1) > window_duration {
            *entry = (0, now);
        }

        entry.0 += 1;

        check_count("memory", tier, key, i64::from(entry.0), limit)
    }

    /// Clean up expired entries across all tiers (call periodically)
    pub async fn cleanup(&self) {
        let mut counters = self.counters.write().await;
        let now = Instant::now();
        for (tier_name, tier_map) in counters.iter_mut() {
            let window = match tier_name.as_str() {
                "metrics" => Duration::from_secs(tiers::METRICS.window_seconds),
                "cas_upload" => Duration::from_secs(tiers::CAS_UPLOAD.window_seconds),
                "cas_read" => Duration::from_secs(tiers::CAS_READ.window_seconds),
                "oauth" => Duration::from_secs(tiers::OAUTH.window_seconds),
                "admin" => Duration::from_secs(tiers::ADMIN.window_seconds),
                _ => Duration::from_secs(tiers::DEFAULT.window_seconds),
            };
            tier_map
                .retain(|_, (count, start)| *count > 0 && now.duration_since(*start) <= window * 2);
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

fn should_bypass_rate_limit(path: &str) -> bool {
    matches!(path, "/health" | "/ready")
}

/// Determine rate limit tier from request path
fn tier_for_path(path: &str) -> (&'static str, RateLimitTier) {
    if path.starts_with("/worker/metrics") {
        ("metrics", tiers::METRICS)
    } else if path.starts_with("/worker/cas/upload") {
        ("cas_upload", tiers::CAS_UPLOAD)
    } else if path.starts_with("/worker/cas") {
        ("cas_read", tiers::CAS_READ)
    } else if path.starts_with("/worker/oauth") {
        ("oauth", tiers::OAUTH)
    } else if path.starts_with("/api/admin") {
        ("admin", tiers::ADMIN)
    } else {
        ("default", tiers::DEFAULT)
    }
}

/// Extract a client identifier from the request.
/// Prefers X-API-Key prefix, falls back to Authorization header, then IP address.
fn extract_client_key(request: &Request) -> String {
    // Try X-API-Key prefix
    if let Some(api_key) = request.headers().get("X-API-Key") {
        if let Ok(key_str) = api_key.to_str() {
            return format!("key:{}", &key_str[..key_str.len().min(8)]);
        }
    }
    // Try Authorization (just the first 16 chars as identifier)
    if let Some(auth) = request.headers().get("Authorization") {
        if let Ok(auth_str) = auth.to_str() {
            return format!("auth:{}", &auth_str[..auth_str.len().min(24)]);
        }
    }
    // Fall back to X-Forwarded-For or X-Real-IP
    if let Some(ip) = request
        .headers()
        .get("X-Forwarded-For")
        .or_else(|| request.headers().get("X-Real-IP"))
    {
        if let Ok(ip_str) = ip.to_str() {
            return format!("ip:{}", ip_str);
        }
    }
    // Last resort: anonymous
    "anonymous".to_string()
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

    let (tier_name, tier_limit) = tier_for_path(&path);
    let client_key = extract_client_key(&request);

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
