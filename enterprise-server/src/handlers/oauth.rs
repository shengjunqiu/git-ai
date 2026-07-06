use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};

use crate::auth::jwt;
use crate::error::AppError;
use crate::models::auth::TokenRequest;
use crate::models::user::{DeviceCodeResponse, OAuthError};
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

/// POST /worker/oauth/token — Token exchange
pub async fn token(
    State(state): State<AppState>,
    Json(req): Json<TokenRequest>,
) -> Result<axum::response::Response, AppError> {
    if req.client_id.as_deref() != Some("git-ai-cli") {
        let err = OAuthError::invalid_grant("Invalid client_id");
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(serde_json::to_value(err).unwrap()),
        )
            .into_response());
    }

    match req.grant_type.as_str() {
        "urn:ietf:params:oauth:grant-type:device_code" => {
            handle_device_code_grant(&state, req.device_code.as_deref()).await
        }
        "refresh_token" => handle_refresh_token_grant(&state, req.refresh_token.as_deref()).await,
        "install_nonce" => handle_install_nonce_grant(&state, req.install_nonce.as_deref()).await,
        "authorization_code" => {
            handle_authorization_code_grant(
                &state,
                req.code.as_deref(),
                req.code_verifier.as_deref(),
                req.redirect_uri.as_deref(),
            )
            .await
        }
        _ => {
            let err = OAuthError::invalid_grant("Unsupported grant_type");
            Ok((
                StatusCode::BAD_REQUEST,
                Json(serde_json::to_value(err).unwrap()),
            )
                .into_response())
        }
    }
}

async fn token_response(
    state: &AppState,
    user_id: uuid::Uuid,
) -> Result<axum::response::Response, AppError> {
    let response = crate::services::tokens::generate_token_response(state, user_id).await?;
    Ok(Json(response).into_response())
}

async fn handle_device_code_grant(
    state: &AppState,
    device_code: Option<&str>,
) -> Result<axum::response::Response, AppError> {
    let device_code =
        device_code.ok_or_else(|| AppError::BadRequest("device_code is required".into()))?;

    let row: Option<(chrono::DateTime<Utc>, Option<uuid::Uuid>)> =
        sqlx::query_as("SELECT expires_at, user_id FROM oauth_devices WHERE device_code = $1")
            .bind(device_code)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?;

    let (expires_at, user_id) = match row {
        Some(r) => r,
        None => {
            let err = OAuthError::expired_token();
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(serde_json::to_value(err).unwrap()),
            )
                .into_response());
        }
    };

    if expires_at < Utc::now() {
        let err = OAuthError::expired_token();
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(serde_json::to_value(err).unwrap()),
        )
            .into_response());
    }

    let user_id = match user_id {
        Some(uid) => uid,
        None => {
            let err = OAuthError::authorization_pending();
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(serde_json::to_value(err).unwrap()),
            )
                .into_response());
        }
    };

    // Clean up used device code
    sqlx::query("DELETE FROM oauth_devices WHERE device_code = $1")
        .bind(device_code)
        .execute(&state.db)
        .await
        .ok();

    token_response(state, user_id).await
}

async fn handle_refresh_token_grant(
    state: &AppState,
    refresh_token: Option<&str>,
) -> Result<axum::response::Response, AppError> {
    let refresh_token =
        refresh_token.ok_or_else(|| AppError::BadRequest("refresh_token is required".into()))?;
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
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(serde_json::to_value(err).unwrap()),
            )
                .into_response());
        }
    };

    if expires_at < Utc::now() {
        let err = OAuthError::invalid_grant("Refresh token expired");
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(serde_json::to_value(err).unwrap()),
        )
            .into_response());
    }

    // Revoke old refresh token
    sqlx::query("UPDATE refresh_tokens SET revoked_at = now() WHERE token_hash = $1")
        .bind(&token_hash)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

    token_response(state, user_id).await
}

async fn handle_install_nonce_grant(
    state: &AppState,
    install_nonce: Option<&str>,
) -> Result<axum::response::Response, AppError> {
    let nonce =
        install_nonce.ok_or_else(|| AppError::BadRequest("install_nonce is required".into()))?;

    let row: Option<(uuid::Uuid, bool)> =
        sqlx::query_as("SELECT user_id, used FROM install_nonces WHERE nonce = $1")
            .bind(nonce)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?;

    let (user_id, used) = match row {
        Some(r) => r,
        None => {
            let err = OAuthError::invalid_grant("Invalid install nonce");
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(serde_json::to_value(err).unwrap()),
            )
                .into_response());
        }
    };

    if used {
        let err = OAuthError::invalid_grant("Install nonce already used");
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(serde_json::to_value(err).unwrap()),
        )
            .into_response());
    }

    // Mark nonce as used
    sqlx::query("UPDATE install_nonces SET used = true, used_at = now() WHERE nonce = $1")
        .bind(nonce)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

    token_response(state, user_id).await
}

async fn handle_authorization_code_grant(
    state: &AppState,
    code: Option<&str>,
    code_verifier: Option<&str>,
    redirect_uri: Option<&str>,
) -> Result<axum::response::Response, AppError> {
    let code = code.ok_or_else(|| AppError::BadRequest("code is required".into()))?;
    let code_verifier =
        code_verifier.ok_or_else(|| AppError::BadRequest("code_verifier is required".into()))?;
    let redirect_uri =
        redirect_uri.ok_or_else(|| AppError::BadRequest("redirect_uri is required".into()))?;
    let code_hash = jwt::hash_token(code);

    let row: Option<(
        uuid::Uuid,
        String,
        String,
        String,
        String,
        chrono::DateTime<Utc>,
        Option<chrono::DateTime<Utc>>,
    )> = sqlx::query_as(
        "SELECT user_id, client_id, redirect_uri, code_challenge, code_challenge_method, expires_at, consumed_at \
         FROM authorization_codes \
         WHERE code_hash = $1",
    )
    .bind(&code_hash)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?;

    let Some((
        user_id,
        client_id,
        stored_redirect_uri,
        code_challenge,
        method,
        expires_at,
        consumed_at,
    )) = row
    else {
        return oauth_error("Invalid authorization code");
    };

    if client_id != "git-ai-cli" {
        return oauth_error("Invalid client_id");
    }
    if stored_redirect_uri != redirect_uri {
        return oauth_error("redirect_uri does not match authorization request");
    }
    if method != "S256" {
        return oauth_error("Unsupported code_challenge_method");
    }
    if expires_at < Utc::now() {
        return oauth_error("Authorization code expired");
    }
    if consumed_at.is_some() {
        return oauth_error("Authorization code already used");
    }
    if pkce_challenge(code_verifier) != code_challenge {
        return oauth_error("Invalid code_verifier");
    }

    let consumed: Option<(uuid::Uuid,)> = sqlx::query_as(
        "UPDATE authorization_codes \
         SET consumed_at = now() \
         WHERE code_hash = $1 \
           AND consumed_at IS NULL \
           AND expires_at > now() \
         RETURNING user_id",
    )
    .bind(&code_hash)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?;

    if consumed.is_none() {
        return oauth_error("Authorization code already used or expired");
    }

    crate::services::audit::log_action(
        &state.db,
        Some(user_id),
        None,
        "token.exchange",
        Some("authorization_code"),
        Some(&code_hash),
        Some(serde_json::json!({"grant_type": "authorization_code"})),
        None,
        None,
    )
    .await
    .ok();

    token_response(state, user_id).await
}

fn oauth_error(message: &str) -> Result<axum::response::Response, AppError> {
    let err = OAuthError::invalid_grant(message);
    Ok((
        StatusCode::BAD_REQUEST,
        Json(serde_json::to_value(err).unwrap()),
    )
        .into_response())
}

fn pkce_challenge(code_verifier: &str) -> String {
    let digest = Sha256::digest(code_verifier.as_bytes());
    base64url_no_pad(&digest)
}

fn base64url_no_pad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity((bytes.len() * 4 + 2) / 3);
    let mut i = 0;

    while i + 3 <= bytes.len() {
        let chunk =
            ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | (bytes[i + 2] as u32);
        output.push(ALPHABET[((chunk >> 18) & 0x3f) as usize] as char);
        output.push(ALPHABET[((chunk >> 12) & 0x3f) as usize] as char);
        output.push(ALPHABET[((chunk >> 6) & 0x3f) as usize] as char);
        output.push(ALPHABET[(chunk & 0x3f) as usize] as char);
        i += 3;
    }

    match bytes.len() - i {
        1 => {
            let chunk = (bytes[i] as u32) << 16;
            output.push(ALPHABET[((chunk >> 18) & 0x3f) as usize] as char);
            output.push(ALPHABET[((chunk >> 12) & 0x3f) as usize] as char);
        }
        2 => {
            let chunk = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
            output.push(ALPHABET[((chunk >> 18) & 0x3f) as usize] as char);
            output.push(ALPHABET[((chunk >> 12) & 0x3f) as usize] as char);
            output.push(ALPHABET[((chunk >> 6) & 0x3f) as usize] as char);
        }
        _ => {}
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

        assert_eq!(pkce_challenge(verifier), challenge);
    }

    #[test]
    fn pkce_challenge_rejects_wrong_verifier() {
        let expected_challenge = pkce_challenge("correct-verifier");
        assert_ne!(pkce_challenge("wrong-verifier"), expected_challenge);
    }

    #[test]
    fn base64url_encoder_omits_padding() {
        assert_eq!(base64url_no_pad(b"f"), "Zg");
        assert_eq!(base64url_no_pad(b"fo"), "Zm8");
        assert_eq!(base64url_no_pad(b"foo"), "Zm9v");
    }
}
