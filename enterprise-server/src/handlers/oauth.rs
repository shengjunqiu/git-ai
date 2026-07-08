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
    let consumed: Option<(uuid::Uuid,)> = sqlx::query_as(
        "DELETE FROM oauth_devices \
         WHERE device_code = $1 \
           AND expires_at > now() \
           AND user_id IS NOT NULL \
         RETURNING user_id",
    )
    .bind(device_code)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?;

    if let Some((user_id,)) = consumed {
        return token_response(state, user_id).await;
    }

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

    if user_id.is_none() {
        let err = OAuthError::authorization_pending();
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(serde_json::to_value(err).unwrap()),
        )
            .into_response());
    }

    let err = OAuthError::expired_token();
    Ok((
        StatusCode::BAD_REQUEST,
        Json(serde_json::to_value(err).unwrap()),
    )
        .into_response())
}

async fn handle_refresh_token_grant(
    state: &AppState,
    refresh_token: Option<&str>,
) -> Result<axum::response::Response, AppError> {
    let refresh_token =
        refresh_token.ok_or_else(|| AppError::BadRequest("refresh_token is required".into()))?;
    let token_hash = jwt::hash_token(refresh_token);

    let consumed: Option<(uuid::Uuid,)> = sqlx::query_as(
        "UPDATE refresh_tokens \
         SET revoked_at = now() \
         WHERE token_hash = $1 \
           AND revoked_at IS NULL \
           AND expires_at > now() \
         RETURNING user_id",
    )
    .bind(&token_hash)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?;

    match consumed {
        Some((user_id,)) => token_response(state, user_id).await,
        None => {
            let err = OAuthError::invalid_grant("Invalid or revoked refresh token");
            Ok((
                StatusCode::BAD_REQUEST,
                Json(serde_json::to_value(err).unwrap()),
            )
                .into_response())
        }
    }
}

async fn handle_install_nonce_grant(
    state: &AppState,
    install_nonce: Option<&str>,
) -> Result<axum::response::Response, AppError> {
    let nonce =
        install_nonce.ok_or_else(|| AppError::BadRequest("install_nonce is required".into()))?;

    let consumed: Option<(uuid::Uuid,)> = sqlx::query_as(
        "UPDATE install_nonces \
         SET used = true, used_at = now() \
         WHERE nonce = $1 \
           AND used = false \
         RETURNING user_id",
    )
    .bind(nonce)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?;

    match consumed {
        Some((user_id,)) => token_response(state, user_id).await,
        None => {
            let err = OAuthError::invalid_grant("Invalid install nonce");
            Ok((
                StatusCode::BAD_REQUEST,
                Json(serde_json::to_value(err).unwrap()),
            )
                .into_response())
        }
    }
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

    crate::services::audit::spawn_log_action(
        state.db.clone(),
        crate::services::audit::AuditPayload {
            user_id: Some(user_id),
            org_id: None,
            action: "token.exchange".to_string(),
            resource_type: Some("authorization_code".to_string()),
            resource_id: Some(code_hash),
            details: Some(serde_json::json!({"grant_type": "authorization_code"})),
            ip_address: None,
            user_agent: None,
        },
    );

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
    use sqlx::postgres::PgPoolOptions;
    use sqlx::PgPool;
    use uuid::Uuid;

    struct TestDatabase {
        state: AppState,
        admin_pool: PgPool,
        db_name: String,
    }

    impl TestDatabase {
        async fn new() -> anyhow::Result<Option<Self>> {
            let database_url = test_database_url();
            let db_name = unique_test_database_name();
            let admin_url = database_url_for_database(&database_url, "postgres")?;
            let test_url = database_url_for_database(&database_url, &db_name)?;

            let admin_pool = match PgPoolOptions::new()
                .max_connections(2)
                .connect(&admin_url)
                .await
            {
                Ok(pool) => pool,
                Err(error) => {
                    eprintln!(
                        "skipping oauth concurrency test: could not connect to admin database: {error}"
                    );
                    return Ok(None);
                }
            };

            if let Err(error) = create_database(&admin_pool, &db_name).await {
                eprintln!(
                    "skipping oauth concurrency test: could not create isolated database {db_name}: {error}"
                );
                return Ok(None);
            }

            let pool = PgPoolOptions::new()
                .max_connections(6)
                .connect(&test_url)
                .await?;
            crate::db::run_migrations(&pool).await?;

            let config = test_config(&test_url);
            let redis = redis::Client::open(config.redis_url.clone())?;
            let cas_store = crate::services::cas::CasStore::new(&config)?;
            let auth_password_limiter = crate::routes::auth_password_limiter(&config);
            let state = AppState {
                db: pool,
                redis,
                config,
                cas_store,
                rate_limiter: crate::services::rate_limit::RateLimiter::new(),
                auth_password_limiter,
            };

            Ok(Some(Self {
                state,
                admin_pool,
                db_name,
            }))
        }

        async fn cleanup(self) -> anyhow::Result<()> {
            self.state.db.close().await;
            drop_database(&self.admin_pool, &self.db_name).await?;
            self.admin_pool.close().await;
            Ok(())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refresh_token_concurrent_exchange_succeeds_once() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let user_id = insert_test_user(&db.state.db).await?;
        let refresh_token = "refresh-token-concurrency-test";
        let refresh_token_hash = jwt::hash_token(refresh_token);

        sqlx::query(
            "INSERT INTO refresh_tokens (user_id, token_hash, expires_at) \
             VALUES ($1, $2, now() + interval '1 hour')",
        )
        .bind(user_id)
        .bind(&refresh_token_hash)
        .execute(&db.state.db)
        .await?;

        let first = handle_refresh_token_grant(&db.state, Some(refresh_token));
        let second = handle_refresh_token_grant(&db.state, Some(refresh_token));
        let (first_response, second_response) = tokio::join!(first, second);
        let responses = [first_response?, second_response?];

        assert_single_success(&responses);

        let old_token_revoked: bool = sqlx::query_scalar(
            "SELECT revoked_at IS NOT NULL FROM refresh_tokens WHERE token_hash = $1",
        )
        .bind(&refresh_token_hash)
        .fetch_one(&db.state.db)
        .await?;
        assert!(old_token_revoked);

        assert_eq!(active_refresh_token_count(&db.state.db, user_id).await?, 1);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn install_nonce_concurrent_exchange_succeeds_once() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let user_id = insert_test_user(&db.state.db).await?;
        let nonce = "install-nonce-concurrency-test";

        sqlx::query("INSERT INTO install_nonces (nonce, user_id) VALUES ($1, $2)")
            .bind(nonce)
            .bind(user_id)
            .execute(&db.state.db)
            .await?;

        let first = handle_install_nonce_grant(&db.state, Some(nonce));
        let second = handle_install_nonce_grant(&db.state, Some(nonce));
        let (first_response, second_response) = tokio::join!(first, second);
        let responses = [first_response?, second_response?];

        assert_single_success(&responses);

        let nonce_used: bool =
            sqlx::query_scalar("SELECT used FROM install_nonces WHERE nonce = $1")
                .bind(nonce)
                .fetch_one(&db.state.db)
                .await?;
        assert!(nonce_used);
        assert_eq!(active_refresh_token_count(&db.state.db, user_id).await?, 1);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn device_code_concurrent_exchange_succeeds_once() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let user_id = insert_test_user(&db.state.db).await?;
        let device_code = "device-code-concurrency-test";

        sqlx::query(
            "INSERT INTO oauth_devices (
                device_code, user_code, verification_uri, expires_at, user_id, authorized_at
            ) VALUES ($1, $2, $3, now() + interval '15 minutes', $4, now())",
        )
        .bind(device_code)
        .bind("USER-CODE")
        .bind("http://localhost:8080/verify")
        .bind(user_id)
        .execute(&db.state.db)
        .await?;

        let first = handle_device_code_grant(&db.state, Some(device_code));
        let second = handle_device_code_grant(&db.state, Some(device_code));
        let (first_response, second_response) = tokio::join!(first, second);
        let responses = [first_response?, second_response?];

        assert_single_success(&responses);

        let device_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM oauth_devices WHERE device_code = $1")
                .bind(device_code)
                .fetch_one(&db.state.db)
                .await?;
        assert_eq!(device_rows, 0);
        assert_eq!(active_refresh_token_count(&db.state.db, user_id).await?, 1);

        db.cleanup().await?;
        Ok(())
    }

    fn assert_single_success(responses: &[axum::response::Response]) {
        let success_count = responses
            .iter()
            .filter(|response| response.status() == StatusCode::OK)
            .count();
        let bad_request_count = responses
            .iter()
            .filter(|response| response.status() == StatusCode::BAD_REQUEST)
            .count();

        assert_eq!(success_count, 1, "exactly one exchange should succeed");
        assert_eq!(
            bad_request_count, 1,
            "the concurrent loser should receive a grant error"
        );
    }

    async fn insert_test_user(pool: &PgPool) -> anyhow::Result<Uuid> {
        let user_id = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, email, name) VALUES ($1, $2, $3)")
            .bind(user_id)
            .bind(format!("{user_id}@example.com"))
            .bind("OAuth Test User")
            .execute(pool)
            .await?;
        Ok(user_id)
    }

    async fn active_refresh_token_count(pool: &PgPool, user_id: Uuid) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT COUNT(*) FROM refresh_tokens WHERE user_id = $1 AND revoked_at IS NULL",
        )
        .bind(user_id)
        .fetch_one(pool)
        .await?)
    }

    fn test_config(database_url: &str) -> crate::config::AppConfig {
        crate::config::AppConfig {
            database_url: database_url.to_string(),
            database_max_connections: 20,
            database_min_connections: 1,
            database_acquire_timeout_seconds: 5,
            redis_url: "redis://127.0.0.1:6379".to_string(),
            jwt_secret: "oauth-concurrency-test-secret".to_string(),
            s3_endpoint: "http://localhost:9000".to_string(),
            s3_bucket: "git-ai-cas".to_string(),
            s3_access_key: "minioadmin".to_string(),
            s3_secret_key: "minioadmin".to_string(),
            s3_region: "us-east-1".to_string(),
            cas_upload_concurrency: 8,
            auth_password_concurrency: 8,
            metrics_write_rollups: true,
            dashboard_use_rollups: false,
            rate_limit_metrics_max_requests: 60,
            rate_limit_metrics_window_seconds: 60,
            rate_limit_cas_upload_max_requests: 30,
            rate_limit_cas_upload_window_seconds: 60,
            rate_limit_cas_read_max_requests: 100,
            rate_limit_cas_read_window_seconds: 60,
            rate_limit_oauth_max_requests: 600,
            rate_limit_oauth_window_seconds: 60,
            rate_limit_auth_max_requests: 300,
            rate_limit_auth_window_seconds: 60,
            rate_limit_admin_max_requests: 30,
            rate_limit_admin_window_seconds: 60,
            rate_limit_default_max_requests: 300,
            rate_limit_default_window_seconds: 60,
            base_url: "http://localhost:8080".to_string(),
            sentry_dsn: String::new(),
            posthog_host: String::new(),
            posthog_api_key: String::new(),
        }
    }

    fn test_database_url() -> String {
        dotenvy::dotenv().ok();
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://gitai:gitai@localhost:5433/gitai_enterprise".into())
    }

    fn unique_test_database_name() -> String {
        format!("git_ai_oauth_test_{}", Uuid::new_v4().simple())
    }

    fn database_url_for_database(database_url: &str, database: &str) -> anyhow::Result<String> {
        let mut url = url::Url::parse(database_url)?;
        url.set_path(database);
        Ok(url.to_string())
    }

    async fn create_database(pool: &PgPool, db_name: &str) -> anyhow::Result<()> {
        sqlx::query(&format!("CREATE DATABASE {}", quote_ident(db_name)))
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn drop_database(pool: &PgPool, db_name: &str) -> anyhow::Result<()> {
        sqlx::query(&format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            quote_ident(db_name)
        ))
        .execute(pool)
        .await?;
        Ok(())
    }

    fn quote_ident(identifier: &str) -> String {
        format!("\"{}\"", identifier.replace('"', "\"\""))
    }

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
