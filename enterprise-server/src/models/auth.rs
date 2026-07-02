use serde::{Deserialize, Serialize};

/// POST /worker/oauth/device/code request body
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCodeRequest {
    // Client sends empty JSON `{}`
}

/// POST /worker/oauth/token request body (3 grant types)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    #[serde(default)]
    pub device_code: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub install_nonce: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
}

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
    pub key: String,                 // Only returned on creation
    pub key_prefix: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}
