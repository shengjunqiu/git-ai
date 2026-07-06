use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub personal_org_id: Option<Uuid>,
    pub password_hash: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub default_org_id: Option<Uuid>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationDomain {
    pub id: Uuid,
    pub org_id: Uuid,
    pub domain: String,
    pub verified: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Department {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgMember {
    pub user_id: Uuid,
    pub org_id: Uuid,
    pub department_id: Option<Uuid>,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: Uuid,
    pub user_id: Uuid,
    pub org_id: Option<Uuid>,
    pub key_prefix: String,
    pub key_hash: String,
    pub name: Option<String>,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub session_token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationCode {
    pub code_hash: String,
    pub user_id: Uuid,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthDevice {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub client_id: String,
    pub expires_at: DateTime<Utc>,
    pub interval_seconds: i32,
    pub user_id: Option<Uuid>,
    pub authorized_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallNonce {
    pub nonce: String,
    pub user_id: Uuid,
    pub used: bool,
    pub created_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
}

/// JWT Claims matching git-ai client expectations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,                // user UUID
    pub email: String,
    pub name: String,
    pub personal_org_id: Option<String>,
    pub orgs: Vec<JwtOrg>,
    pub iat: i64,
    pub exp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtOrg {
    pub org_id: String,
    pub org_name: String,
    pub org_slug: String,
    pub role: String,
}

/// Token response matching git-ai client expectations
/// Client expects: expires_in: u64, refresh_expires_in: u64
/// Server uses i64 for DB compatibility but must serialize as positive numbers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,         // always "Bearer"
    pub expires_in: i64,            // seconds (3600) — client deserializes as u64, compatible with JSON numbers
    pub refresh_token: String,
    pub refresh_expires_in: i64,    // seconds (7776000 ≈ 90 days) — client deserializes as u64
}

/// Device code response matching git-ai client expectations
/// Client expects: expires_in: u32, interval: u32
/// Server uses i64 for DB compatibility but must serialize as positive numbers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    pub expires_in: i64,            // seconds (900) — client deserializes as u32, compatible for values < 2^32
    pub interval: i64,              // seconds (5) — client deserializes as u32, compatible for values < 2^32
}

/// OAuth error response (RFC 8628)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthError {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
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

/// Identity extracted from auth middleware
#[derive(Debug, Clone)]
pub struct AuthIdentity {
    pub user_id: Uuid,
    pub email: String,
    pub name: String,
    pub org_id: Option<Uuid>,
    pub org_slug: Option<String>,
    pub role: Option<String>,
    pub scopes: Vec<String>,
    pub auth_method: AuthMethod,
}

impl AuthIdentity {
    /// Returns true if the user has an admin-level role (owner or admin) in any organization,
    /// or if the API key has the "admin" scope.
    pub fn is_admin(&self) -> bool {
        // Check org role for Bearer token auth
        if let Some(role) = &self.role {
            if role == "owner" || role == "admin" {
                return true;
            }
        }
        // Check API key scopes for admin access
        if self.scopes.contains(&"admin".to_string()) {
            return true;
        }
        false
    }

    /// Returns the list of organization IDs the user belongs to.
    /// For admin users this returns their org_id (admin sees all data within their org).
    /// For non-admin users this also returns their org_id (non-admin data is further filtered by user_id).
    pub fn data_scope_org_ids(&self) -> Option<Vec<Uuid>> {
        self.org_id.map(|id| vec![id])
    }

    /// Returns the user_id for data filtering.
    /// For admin users this returns None (admin sees all users' data within their org).
    /// For non-admin users this returns Some(user_id) (only their own data).
    pub fn data_scope_user_id(&self) -> Option<Uuid> {
        if self.is_admin() {
            return None; // Admin sees all users' data within the org
        }
        Some(self.user_id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AuthMethod {
    BearerToken,
    ApiKey,
    WebSession,
}

/// Request headers extracted by middleware
#[derive(Debug, Clone, Default)]
pub struct RequestHeaders {
    pub distinct_id: Option<String>,
    pub author_identity: Option<String>,
    pub user_agent: Option<String>,
}
