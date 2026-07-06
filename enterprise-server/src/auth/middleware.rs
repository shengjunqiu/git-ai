use axum::extract::{FromRequestParts, Request};
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::user::{AuthIdentity, AuthMethod, RequestHeaders};
use crate::routes::AppState;
use crate::services::sessions;

/// Extract auth identity from request (Bearer token or X-API-Key)
#[derive(Debug, Clone)]
pub struct AuthExtractor(pub AuthIdentity);

impl FromRequestParts<AppState> for AuthExtractor {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let identity = extract_auth_identity(parts, &state.config, &state.db).await?;
        Ok(AuthExtractor(identity))
    }
}

/// Optional auth - returns None if no auth provided
#[derive(Debug, Clone)]
pub struct OptionalAuth(pub Option<AuthIdentity>);

impl FromRequestParts<AppState> for OptionalAuth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let identity = extract_auth_identity(parts, &state.config, &state.db)
            .await
            .ok();
        Ok(OptionalAuth(identity))
    }
}

/// Admin guard - requires authentication AND admin/owner role
#[derive(Debug, Clone)]
pub struct AdminGuard(pub AuthIdentity);

impl FromRequestParts<AppState> for AdminGuard {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let identity = extract_auth_identity(parts, &state.config, &state.db).await?;
        if identity.is_admin() {
            Ok(AdminGuard(identity))
        } else {
            Err(AppError::Forbidden("Admin access required".into()))
        }
    }
}

/// Extract request headers
#[derive(Debug, Clone)]
pub struct HeaderExtractor(pub RequestHeaders);

impl FromRequestParts<AppState> for HeaderExtractor {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let headers = RequestHeaders {
            distinct_id: parts
                .headers
                .get("X-Distinct-ID")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
            author_identity: parts
                .headers
                .get("X-Author-Identity")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
            user_agent: parts
                .headers
                .get("User-Agent")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
        };
        Ok(HeaderExtractor(headers))
    }
}

async fn extract_auth_identity(
    parts: &mut Parts,
    config: &crate::config::AppConfig,
    pool: &sqlx::PgPool,
) -> Result<AuthIdentity, AppError> {
    // Try Bearer token first
    if let Some(auth_header) = parts.headers.get("Authorization") {
        let auth_str = auth_header.to_str().map_err(|_| {
            AppError::Unauthorized("Invalid Authorization header".into())
        })?;

        if let Some(token) = auth_str.strip_prefix("Bearer ") {
            let claims = crate::auth::jwt::validate_access_token(token, config)?;
            let user_id = Uuid::parse_str(&claims.sub)
                .map_err(|_| AppError::Unauthorized("Invalid user ID in token".into()))?;

            let org_row: Option<(Uuid,)> = sqlx::query_as(
                "SELECT org_id FROM org_members WHERE user_id = $1 LIMIT 1"
            )
            .bind(user_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Database(e))?;

            return Ok(AuthIdentity {
                user_id,
                email: claims.email,
                name: claims.name,
                org_id: org_row.map(|r| r.0),
                org_slug: claims.orgs.first().map(|o| o.org_slug.clone()),
                role: claims.orgs.first().map(|o| o.role.clone()),
                scopes: vec![
                    "metrics:write".into(),
                    "cas:write".into(),
                    "cas:read".into(),
                    "reports:write".into(),
                ],
                auth_method: AuthMethod::BearerToken,
            });
        }
    }

    // Try Cookie (access_token cookie for browser dashboard access)
    if let Some(cookie_header) = parts.headers.get("Cookie") {
        let cookie_str = cookie_header.to_str().unwrap_or("");
        for cookie in cookie_str.split(';') {
            let cookie = cookie.trim();
            if let Some(token) = cookie.strip_prefix("access_token=") {
                if !token.is_empty() {
                    let claims = crate::auth::jwt::validate_access_token(token, config)?;
                    let user_id = Uuid::parse_str(&claims.sub)
                        .map_err(|_| AppError::Unauthorized("Invalid user ID in token".into()))?;

                    let org_row: Option<(Uuid,)> = sqlx::query_as(
                        "SELECT org_id FROM org_members WHERE user_id = $1 LIMIT 1"
                    )
                    .bind(user_id)
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| AppError::Database(e))?;

                    return Ok(AuthIdentity {
                        user_id,
                        email: claims.email,
                        name: claims.name,
                        org_id: org_row.map(|r| r.0),
                        org_slug: claims.orgs.first().map(|o| o.org_slug.clone()),
                        role: claims.orgs.first().map(|o| o.role.clone()),
                        scopes: vec![
                            "metrics:write".into(),
                            "cas:write".into(),
                            "cas:read".into(),
                            "reports:write".into(),
                        ],
                        auth_method: AuthMethod::BearerToken,
                    });
                }
            }
        }
    }

    // Try X-API-Key header or api_key cookie
    let api_key_value = if let Some(api_key_header) = parts.headers.get("X-API-Key") {
        Some(api_key_header.to_str().ok().unwrap_or("").to_string())
    } else if let Some(cookie_header) = parts.headers.get("Cookie") {
        let cookie_str = cookie_header.to_str().unwrap_or("");
        cookie_str
            .split(';')
            .find_map(|c| {
                let c = c.trim();
                c.strip_prefix("api_key=").map(|v| v.to_string())
            })
    } else {
        None
    };

    if let Some(api_key) = api_key_value {
        if !api_key.is_empty() {
            let key_hash = crate::auth::jwt::hash_token(&api_key);

            let row: Option<(Uuid, Option<Uuid>, Vec<String>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
                "SELECT user_id, org_id, scopes, expires_at \
                 FROM api_keys WHERE key_hash = $1 AND revoked_at IS NULL"
            )
            .bind(&key_hash)
            .fetch_optional(pool)
            .await
            .map_err(|e| AppError::Database(e))?;

            if let Some((user_id, org_id, scopes, expires_at)) = row {
                if let Some(expires) = expires_at {
                    if expires < chrono::Utc::now() {
                        return Err(AppError::Unauthorized("API Key has expired".into()));
                    }
                }

                let user_row: Option<(Uuid, String, String)> = sqlx::query_as(
                    "SELECT id, email, name FROM users WHERE id = $1"
                )
                .bind(user_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| AppError::Database(e))?;

                if let Some((_id, email, name)) = user_row {
                    // Look up the user's role in org_members for the API key's org
                    // This is critical: we must use the org_id from the API key to get
                    // the correct role (e.g., "member" in linewell.com vs "owner" in personal org)
                    let role_row: Option<(String,)> = if let Some(key_org_id) = org_id {
                        sqlx::query_as(
                            "SELECT role FROM org_members WHERE user_id = $1 AND org_id = $2"
                        )
                        .bind(user_id)
                        .bind(key_org_id)
                        .fetch_optional(pool)
                        .await
                        .map_err(|e| AppError::Database(e))?
                    } else {
                        // No org_id on API key, fall back to first org membership
                        sqlx::query_as(
                            "SELECT role FROM org_members WHERE user_id = $1 LIMIT 1"
                        )
                        .bind(user_id)
                        .fetch_optional(pool)
                        .await
                        .map_err(|e| AppError::Database(e))?
                    };

                    // Use org_members role if available, otherwise fall back to "api_key"
                    let role = role_row.map(|r| r.0).unwrap_or_else(|| "api_key".into());

                    return Ok(AuthIdentity {
                        user_id,
                        email,
                        name,
                        org_id,
                        org_slug: None,
                        role: Some(role),
                        scopes,
                        auth_method: AuthMethod::ApiKey,
                    });
                }
            }
        }
    }

    Err(AppError::Unauthorized("Authentication required".into()))
}

/// Extract the browser web session user without changing API Bearer/API key auth semantics.
pub async fn extract_web_session_user(
    parts: &Parts,
    pool: &sqlx::PgPool,
) -> Result<Option<Uuid>, AppError> {
    let Some(session_token) = cookie_value(parts, sessions::WEB_SESSION_COOKIE) else {
        return Ok(None);
    };

    sessions::load_web_session_user(pool, &session_token).await
}

fn cookie_value(parts: &Parts, name: &str) -> Option<String> {
    let cookie_header = parts.headers.get("Cookie")?;
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

/// Middleware to add request ID for tracing
pub async fn request_id_middleware(request: Request, next: Next) -> Response {
    let request_id = uuid::Uuid::new_v4().to_string();
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("X-Request-Id", request_id.parse().unwrap());
    response
}
