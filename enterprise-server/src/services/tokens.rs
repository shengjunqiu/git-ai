//! Token issuance service.

use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::auth::jwt;
use crate::error::AppError;
use crate::models::user::{JwtOrg, TokenResponse};
use crate::routes::AppState;

const ACCESS_TOKEN_TTL_SECONDS: i64 = 3600;
const REFRESH_TOKEN_TTL_SECONDS: i64 = 7776000;

/// Build the standard OAuth token response for a user and persist a new refresh token.
pub async fn generate_token_response(
    state: &AppState,
    user_id: Uuid,
) -> Result<TokenResponse, AppError> {
    let user_row: (String, String, Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT email, name, personal_org_id, default_org_id FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .map_err(AppError::Database)?;

    let org_rows: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT om.org_id, o.name, o.slug, om.role \
         FROM org_members om \
         JOIN organizations o ON o.id = om.org_id \
         WHERE om.user_id = $1 \
         ORDER BY CASE \
             WHEN $2::uuid IS NOT NULL AND om.org_id = $2 THEN 0 \
             WHEN $2::uuid IS NULL AND $3::uuid IS NOT NULL AND om.org_id <> $3 THEN 0 \
             ELSE 1 \
         END, o.created_at",
    )
    .bind(user_id)
    .bind(user_row.3)
    .bind(user_row.2)
    .fetch_all(&state.db)
    .await
    .map_err(AppError::Database)?;

    let orgs = org_rows
        .into_iter()
        .map(|(org_id, org_name, org_slug, role)| JwtOrg {
            org_id: org_id.to_string(),
            org_name,
            org_slug,
            role,
        })
        .collect();

    let access_token = jwt::create_access_token(
        &user_id,
        &user_row.0,
        &user_row.1,
        user_row.2.as_ref(),
        orgs,
        &state.config,
    )?;

    let refresh_token = jwt::generate_refresh_token();
    let refresh_token_hash = jwt::hash_token(&refresh_token);
    let refresh_expires_at = Utc::now() + Duration::seconds(REFRESH_TOKEN_TTL_SECONDS);

    sqlx::query("INSERT INTO refresh_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(&refresh_token_hash)
        .bind(refresh_expires_at)
        .execute(&state.db)
        .await
        .map_err(AppError::Database)?;

    Ok(TokenResponse {
        access_token,
        token_type: "Bearer".into(),
        expires_in: ACCESS_TOKEN_TTL_SECONDS,
        refresh_token,
        refresh_expires_in: REFRESH_TOKEN_TTL_SECONDS,
    })
}
