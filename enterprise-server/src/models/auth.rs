use serde::{Deserialize, Serialize};

pub use git_ai_protocol::oauth::TokenRequest;

/// Request to create an API key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub org_id: Option<uuid::Uuid>,
    pub scopes: Option<Vec<String>>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub user_id: Option<uuid::Uuid>,
}

/// API key response (includes plaintext key only on creation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyResponse {
    pub id: uuid::Uuid,
    pub key: String, // Only returned on creation
    pub key_prefix: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_request_deserializes_authorization_code_grant() {
        let request: TokenRequest = serde_json::from_str(
            r#"{
                "grant_type": "authorization_code",
                "client_id": "git-ai-cli",
                "code": "code-123",
                "code_verifier": "verifier-123",
                "redirect_uri": "http://127.0.0.1:49152/callback"
            }"#,
        )
        .unwrap();

        assert_eq!(request.grant_type, "authorization_code");
        assert_eq!(request.client_id.as_deref(), Some("git-ai-cli"));
        assert_eq!(request.code.as_deref(), Some("code-123"));
        assert_eq!(request.code_verifier.as_deref(), Some("verifier-123"));
        assert_eq!(
            request.redirect_uri.as_deref(),
            Some("http://127.0.0.1:49152/callback")
        );
        assert!(request.device_code.is_none());
    }

    #[test]
    fn token_request_keeps_legacy_device_grant_fields_optional() {
        let request: TokenRequest = serde_json::from_str(
            r#"{
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
                "client_id": "git-ai-cli",
                "device_code": "device-123"
            }"#,
        )
        .unwrap();

        assert_eq!(request.device_code.as_deref(), Some("device-123"));
        assert!(request.code.is_none());
        assert!(request.code_verifier.is_none());
        assert!(request.redirect_uri.is_none());
    }
}
