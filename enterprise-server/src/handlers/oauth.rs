use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use chrono::{Duration, Utc};

use crate::auth::jwt;
use crate::error::AppError;
use crate::models::user::{DeviceCodeResponse, OAuthError, TokenResponse};
use crate::models::auth::TokenRequest;
use crate::routes::AppState;

/// POST /worker/oauth/device/code — Start device authorization flow
pub async fn device_code(
    State(state): State<AppState>,
) -> Result<axum::response::Response, AppError> {
    let (device_code, user_code) = jwt::generate_device_codes();
    let verification_uri = format!("{}/verify", state.config.base_url);
    let expires_at = Utc::now() + Duration::seconds(900);

    sqlx::query(
        "INSERT INTO oauth_devices (device_code, user_code, verification_uri, expires_at) VALUES ($1, $2, $3, $4)"
    )
    .bind(&device_code)
    .bind(&user_code)
    .bind(&verification_uri)
    .bind(expires_at)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let response = DeviceCodeResponse {
        device_code,
        user_code,
        verification_uri,
        verification_uri_complete: None,
        expires_in: 900,
        interval: 5,
    };

    Ok(Json(serde_json::to_value(response).unwrap()).into_response())
}

/// POST /worker/oauth/token — Token exchange (3 grant types)
pub async fn token(
    State(state): State<AppState>,
    Json(req): Json<TokenRequest>,
) -> Result<axum::response::Response, AppError> {
    if req.client_id.as_deref() != Some("git-ai-cli") {
        let err = OAuthError::invalid_grant("Invalid client_id");
        return Ok((StatusCode::BAD_REQUEST, Json(serde_json::to_value(err).unwrap())).into_response());
    }

    match req.grant_type.as_str() {
        "urn:ietf:params:oauth:grant-type:device_code" => {
            handle_device_code_grant(&state, req.device_code.as_deref()).await
        }
        "refresh_token" => {
            handle_refresh_token_grant(&state, req.refresh_token.as_deref()).await
        }
        "install_nonce" => {
            handle_install_nonce_grant(&state, req.install_nonce.as_deref()).await
        }
        _ => {
            let err = OAuthError::invalid_grant("Unsupported grant_type");
            Ok((StatusCode::BAD_REQUEST, Json(serde_json::to_value(err).unwrap())).into_response())
        }
    }
}

/// Helper: get user info and generate token response
async fn generate_token_response(state: &AppState, user_id: uuid::Uuid) -> Result<axum::response::Response, AppError> {
    let user_row: (String, String, Option<uuid::Uuid>) = sqlx::query_as(
        "SELECT email, name, personal_org_id FROM users WHERE id = $1"
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Get org memberships
    let org_rows: Vec<(uuid::Uuid, String)> = sqlx::query_as(
        "SELECT org_id, role FROM org_members WHERE user_id = $1"
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let mut orgs = Vec::new();
    for (org_id, role) in org_rows {
        let org_name_row: Option<(String, String)> = sqlx::query_as(
            "SELECT name, slug FROM organizations WHERE id = $1"
        )
        .bind(org_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

        if let Some((org_name, org_slug)) = org_name_row {
            orgs.push(crate::models::user::JwtOrg {
                org_id: org_id.to_string(),
                org_name,
                org_slug,
                role,
            });
        }
    }

    let access_token = jwt::create_access_token(
        &user_id,
        &user_row.0,
        &user_row.1,
        user_row.2.as_ref(),
        orgs,
        &state.config,
    )?;

    let refresh_token_str = jwt::generate_refresh_token();
    let refresh_token_hash = jwt::hash_token(&refresh_token_str);
    let refresh_expires_at = Utc::now() + Duration::seconds(7776000);

    sqlx::query(
        "INSERT INTO refresh_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)"
    )
    .bind(user_id)
    .bind(&refresh_token_hash)
    .bind(refresh_expires_at)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let response = TokenResponse {
        access_token,
        token_type: "Bearer".into(),
        expires_in: 3600,
        refresh_token: refresh_token_str,
        refresh_expires_in: 7776000,
    };

    Ok(Json(serde_json::to_value(response).unwrap()).into_response())
}

async fn handle_device_code_grant(
    state: &AppState,
    device_code: Option<&str>,
) -> Result<axum::response::Response, AppError> {
    let device_code = device_code.ok_or_else(|| AppError::BadRequest("device_code is required".into()))?;

    let row: Option<(chrono::DateTime<Utc>, Option<uuid::Uuid>)> = sqlx::query_as(
        "SELECT expires_at, user_id FROM oauth_devices WHERE device_code = $1"
    )
    .bind(device_code)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let (expires_at, user_id) = match row {
        Some(r) => r,
        None => {
            let err = OAuthError::expired_token();
            return Ok((StatusCode::BAD_REQUEST, Json(serde_json::to_value(err).unwrap())).into_response());
        }
    };

    if expires_at < Utc::now() {
        let err = OAuthError::expired_token();
        return Ok((StatusCode::BAD_REQUEST, Json(serde_json::to_value(err).unwrap())).into_response());
    }

    let user_id = match user_id {
        Some(uid) => uid,
        None => {
            let err = OAuthError::authorization_pending();
            return Ok((StatusCode::BAD_REQUEST, Json(serde_json::to_value(err).unwrap())).into_response());
        }
    };

    // Clean up used device code
    sqlx::query("DELETE FROM oauth_devices WHERE device_code = $1")
        .bind(device_code)
        .execute(&state.db)
        .await
        .ok();

    generate_token_response(state, user_id).await
}

async fn handle_refresh_token_grant(
    state: &AppState,
    refresh_token: Option<&str>,
) -> Result<axum::response::Response, AppError> {
    let refresh_token = refresh_token.ok_or_else(|| AppError::BadRequest("refresh_token is required".into()))?;
    let token_hash = jwt::hash_token(refresh_token);

    let row: Option<(uuid::Uuid, chrono::DateTime<Utc>)> = sqlx::query_as(
        "SELECT user_id, expires_at FROM refresh_tokens WHERE token_hash = $1 AND revoked_at IS NULL"
    )
    .bind(&token_hash)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let (user_id, expires_at) = match row {
        Some(r) => r,
        None => {
            let err = OAuthError::invalid_grant("Invalid or revoked refresh token");
            return Ok((StatusCode::BAD_REQUEST, Json(serde_json::to_value(err).unwrap())).into_response());
        }
    };

    if expires_at < Utc::now() {
        let err = OAuthError::invalid_grant("Refresh token expired");
        return Ok((StatusCode::BAD_REQUEST, Json(serde_json::to_value(err).unwrap())).into_response());
    }

    // Revoke old refresh token
    sqlx::query("UPDATE refresh_tokens SET revoked_at = now() WHERE token_hash = $1")
        .bind(&token_hash)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

    generate_token_response(state, user_id).await
}

async fn handle_install_nonce_grant(
    state: &AppState,
    install_nonce: Option<&str>,
) -> Result<axum::response::Response, AppError> {
    let nonce = install_nonce.ok_or_else(|| AppError::BadRequest("install_nonce is required".into()))?;

    let row: Option<(uuid::Uuid, bool)> = sqlx::query_as(
        "SELECT user_id, used FROM install_nonces WHERE nonce = $1"
    )
    .bind(nonce)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let (user_id, used) = match row {
        Some(r) => r,
        None => {
            let err = OAuthError::invalid_grant("Invalid install nonce");
            return Ok((StatusCode::BAD_REQUEST, Json(serde_json::to_value(err).unwrap())).into_response());
        }
    };

    if used {
        let err = OAuthError::invalid_grant("Install nonce already used");
        return Ok((StatusCode::BAD_REQUEST, Json(serde_json::to_value(err).unwrap())).into_response());
    }

    // Mark nonce as used
    sqlx::query("UPDATE install_nonces SET used = true, used_at = now() WHERE nonce = $1")
        .bind(nonce)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

    generate_token_response(state, user_id).await
}
