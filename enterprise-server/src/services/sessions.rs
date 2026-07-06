//! Browser web session service.

use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::auth::jwt;
use crate::error::AppError;

pub const WEB_SESSION_COOKIE: &str = "web_session";
const WEB_SESSION_TOKEN_BYTES: usize = 32;
const WEB_SESSION_TTL_DAYS: i64 = 30;

pub fn generate_web_session_token() -> String {
    use rand::Rng;

    let mut bytes = [0u8; WEB_SESSION_TOKEN_BYTES];
    rand::thread_rng().fill(&mut bytes[..]);
    hex::encode(bytes)
}

pub async fn create_web_session(pool: &PgPool, user_id: Uuid) -> Result<String, AppError> {
    let session_token = generate_web_session_token();
    let session_token_hash = jwt::hash_token(&session_token);
    let expires_at = Utc::now() + Duration::days(WEB_SESSION_TTL_DAYS);

    sqlx::query(
        "INSERT INTO web_sessions (user_id, session_token_hash, expires_at) VALUES ($1, $2, $3)",
    )
    .bind(user_id)
    .bind(&session_token_hash)
    .bind(expires_at)
    .execute(pool)
    .await
    .map_err(AppError::Database)?;

    Ok(session_token)
}

pub async fn load_web_session_user(
    pool: &PgPool,
    session_token: &str,
) -> Result<Option<Uuid>, AppError> {
    if session_token.is_empty() {
        return Ok(None);
    }

    let session_token_hash = jwt::hash_token(session_token);
    let row: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE web_sessions \
         SET last_seen_at = now() \
         WHERE session_token_hash = $1 \
           AND revoked_at IS NULL \
           AND expires_at > now() \
         RETURNING user_id",
    )
    .bind(&session_token_hash)
    .fetch_optional(pool)
    .await
    .map_err(AppError::Database)?;

    Ok(row.map(|r| r.0))
}

pub async fn revoke_web_session(pool: &PgPool, session_token: &str) -> Result<(), AppError> {
    if session_token.is_empty() {
        return Ok(());
    }

    let session_token_hash = jwt::hash_token(session_token);

    sqlx::query(
        "UPDATE web_sessions \
         SET revoked_at = now() \
         WHERE session_token_hash = $1 \
           AND revoked_at IS NULL",
    )
    .bind(&session_token_hash)
    .execute(pool)
    .await
    .map_err(AppError::Database)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_session_tokens_are_hex_and_unique() {
        let first = generate_web_session_token();
        let second = generate_web_session_token();

        assert_eq!(first.len(), WEB_SESSION_TOKEN_BYTES * 2);
        assert_ne!(first, second);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
