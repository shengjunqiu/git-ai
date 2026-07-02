use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::AppError;
use crate::routes::AppState;

/// Rate limit tier configuration: (max_requests, window_seconds)
#[derive(Debug, Clone, Copy)]
pub struct RateLimitTier {
    pub max_requests: u32,
    pub window_seconds: u64,
}

impl RateLimitTier {
    pub const fn new(max_requests: u32, window_seconds: u64) -> Self {
        Self { max_requests, window_seconds }
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

/// In-memory rate limiter using sliding window counters.
/// Falls back to in-memory when Redis is unavailable.
/// Each tier maintains its own counter map keyed by client identifier.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    /// Per-tier counters: tier_name -> (key -> (count, window_start))
    counters: Arc<RwLock<HashMap<String, HashMap<String, (u32, std::time::Instant)>>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            counters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check if a request is allowed for the given tier and client key.
    /// Returns Ok(()) if allowed, Err if rate limited.
    pub async fn check(&self, tier: &str, key: &str, limit: RateLimitTier) -> Result<(), AppError> {
        let mut counters = self.counters.write().await;
        let now = std::time::Instant::now();
        let window_duration = std::time::Duration::from_secs(limit.window_seconds);

        let tier_map = counters.entry(tier.to_string()).or_default();
        let entry = tier_map.entry(key.to_string()).or_insert((0, now));

        // Reset counter if window has expired
        if now.duration_since(entry.1) > window_duration {
            *entry = (0, now);
        }

        entry.0 += 1;

        if entry.0 > limit.max_requests {
            tracing::warn!(
                "Rate limit exceeded: tier={} key={} count={} limit={}",
                tier, key, entry.0, limit.max_requests
            );
            return Err(AppError::RateLimited(format!(
                "Rate limit exceeded. Maximum {} requests per {} seconds.",
                limit.max_requests, limit.window_seconds
            )));
        }

        Ok(())
    }

    /// Clean up expired entries across all tiers (call periodically)
    pub async fn cleanup(&self) {
        let mut counters = self.counters.write().await;
        let now = std::time::Instant::now();
        for (tier_name, tier_map) in counters.iter_mut() {
            let window = match tier_name.as_str() {
                "metrics" => std::time::Duration::from_secs(tiers::METRICS.window_seconds),
                "cas_upload" => std::time::Duration::from_secs(tiers::CAS_UPLOAD.window_seconds),
                "cas_read" => std::time::Duration::from_secs(tiers::CAS_READ.window_seconds),
                "oauth" => std::time::Duration::from_secs(tiers::OAUTH.window_seconds),
                "admin" => std::time::Duration::from_secs(tiers::ADMIN.window_seconds),
                _ => std::time::Duration::from_secs(tiers::DEFAULT.window_seconds),
            };
            tier_map.retain(|_, (count, start)| {
                *count > 0 && now.duration_since(*start) <= window * 2
            });
        }
    }
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
    if let Some(ip) = request.headers().get("X-Forwarded-For")
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
    let (tier_name, tier_limit) = tier_for_path(&path);
    let client_key = extract_client_key(&request);

    state.rate_limiter.check(tier_name, &client_key, tier_limit).await?;

    Ok(next.run(request).await)
}
