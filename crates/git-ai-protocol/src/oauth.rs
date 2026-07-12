use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceCodeRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub code_verifier: Option<String>,
    #[serde(default)]
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    pub expires_in: u32,
    pub interval: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub refresh_token: String,
    pub refresh_expires_in: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OAuthError {
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

impl OAuthError {
    pub fn authorization_pending() -> Self {
        Self {
            error: "authorization_pending".into(),
            error_description: None,
        }
    }

    pub fn slow_down() -> Self {
        Self {
            error: "slow_down".into(),
            error_description: None,
        }
    }

    pub fn access_denied() -> Self {
        Self {
            error: "access_denied".into(),
            error_description: None,
        }
    }

    pub fn expired_token() -> Self {
        Self {
            error: "expired_token".into(),
            error_description: None,
        }
    }

    pub fn invalid_grant(description: &str) -> Self {
        Self {
            error: "invalid_grant".into(),
            error_description: Some(description.into()),
        }
    }
}
