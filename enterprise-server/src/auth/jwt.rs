use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::user::{JwtClaims, JwtOrg};

/// Create a new JWT access token
pub fn create_access_token(
    user_id: &Uuid,
    email: &str,
    name: &str,
    personal_org_id: Option<&Uuid>,
    orgs: Vec<JwtOrg>,
    config: &AppConfig,
) -> Result<String, AppError> {
    let now = Utc::now();
    let claims = JwtClaims {
        sub: user_id.to_string(),
        email: email.to_string(),
        name: name.to_string(),
        personal_org_id: personal_org_id.map(|id| id.to_string()),
        orgs,
        iat: now.timestamp(),
        exp: (now + Duration::hours(1)).timestamp(),
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(format!("Failed to encode JWT: {}", e)))
}

/// Validate a JWT access token and return claims
pub fn validate_access_token(token: &str, config: &AppConfig) -> Result<JwtClaims, AppError> {
    let mut validation = Validation::default();
    validation.validate_exp = true;

    decode::<JwtClaims>(
        token,
        &DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|e| AppError::Unauthorized(format!("Invalid token: {}", e)))
}

/// Create a refresh token (random 64-byte hex string)
pub fn generate_refresh_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 64];
    rng.fill(&mut bytes[..]);
    hex::encode(bytes)
}

/// Hash a refresh token for storage
pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Generate a device code and user code for OAuth Device Flow
pub fn generate_device_codes() -> (String, String) {
    use rand::Rng;
    let mut rng = rand::thread_rng();

    // Device code: 40 hex chars
    let mut device_bytes = [0u8; 20];
    rng.fill(&mut device_bytes[..]);
    let device_code = hex::encode(device_bytes);

    // User code: 8 chars uppercase alphanumeric (XXXX-XXXX format)
    let chars: Vec<char> = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789".chars().collect();
    let user_code: String = (0..8)
        .map(|_| chars[rng.gen_range(0..chars.len())])
        .collect();

    (device_code, user_code)
}

/// Generate an API key with prefix for identification
pub fn generate_api_key() -> (String, String, String) {
    use rand::Rng;
    // API key: "gai_" + 66 hex chars from 33 random bytes
    let mut bytes = [0u8; 33];
    rand::thread_rng().fill(&mut bytes[..]);
    let key = format!("gai_{}", hex::encode(bytes));

    // Prefix: first 8 chars for identification
    let prefix = key[..8].to_string();

    // Hash for storage
    let hash = hash_token(&key);

    (key, prefix, hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_validate_token() {
        let config = AppConfig {
            database_url: String::new(),
            database_max_connections: 20,
            database_min_connections: 1,
            database_acquire_timeout_seconds: 5,
            redis_url: String::new(),
            jwt_secret: "test-secret-key".into(),
            s3_endpoint: String::new(),
            s3_bucket: String::new(),
            s3_access_key: String::new(),
            s3_secret_key: String::new(),
            s3_region: String::new(),
            cas_upload_concurrency: 8,
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
            base_url: String::new(),
            sentry_dsn: String::new(),
            posthog_host: String::new(),
            posthog_api_key: String::new(),
        };

        let user_id = Uuid::new_v4();
        let token = create_access_token(
            &user_id,
            "test@example.com",
            "Test User",
            None,
            vec![],
            &config,
        )
        .unwrap();

        let claims = validate_access_token(&token, &config).unwrap();
        assert_eq!(claims.sub, user_id.to_string());
        assert_eq!(claims.email, "test@example.com");
    }

    #[test]
    fn test_generate_device_codes() {
        let (device_code, user_code) = generate_device_codes();
        assert_eq!(device_code.len(), 40);
        assert_eq!(user_code.len(), 8);
    }

    #[test]
    fn test_generate_api_key() {
        let (key, prefix, hash) = generate_api_key();
        assert!(key.starts_with("gai_"));
        assert_eq!(prefix.len(), 8);
        assert_eq!(hash.len(), 64); // SHA256 hex
    }
}
