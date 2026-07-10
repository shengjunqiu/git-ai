//! Reports CLI login and logout state to the configured server.

use crate::api::client::{ApiClient, ApiContext};
use crate::api::types::ApiErrorResponse;
use crate::error::GitAiError;
use serde::{Deserialize, Serialize};

// test

/// Login state values understood by the client-status API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientStatusKind {
    LoggedIn,
    LoggedOut,
}

impl ClientStatusKind {
    /// Converts the enum to the server's wire-format value.
    fn as_str(self) -> &'static str {
        match self {
            Self::LoggedIn => "logged_in",
            Self::LoggedOut => "logged_out",
        }
    }
}

/// Client state and environment metadata sent with a status update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientStatusRequest {
    pub status: String,
    pub cli_version: String,
    pub os: String,
    pub arch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
}

impl ClientStatusRequest {
    /// Builds a status report using metadata from the current process.
    pub fn new(status: ClientStatusKind) -> Self {
        Self {
            status: status.as_str().to_string(),
            cli_version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            hostname: collect_hostname(),
        }
    }
}

/// Uploads a status update using the currently configured credentials.
pub fn upload_current_client_status(status: ClientStatusKind) -> Result<(), GitAiError> {
    // Status reporting should not hold up login or logout for long.
    let context = ApiContext::new(None).with_timeout(5);
    let client = ApiClient::new(context);
    client.upload_client_status(&ClientStatusRequest::new(status))
}

/// Uploads a status update with credentials obtained by the current login flow.
pub fn upload_client_status_with_token(
    base_url: String,
    access_token: String,
    status: ClientStatusKind,
) -> Result<(), GitAiError> {
    let context = ApiContext::with_auth(Some(base_url), access_token).with_timeout(5);
    let client = ApiClient::new(context);
    client.upload_client_status(&ClientStatusRequest::new(status))
}

impl ApiClient {
    /// Sends one client-status report to the server.
    pub fn upload_client_status(&self, request: &ClientStatusRequest) -> Result<(), GitAiError> {
        let response = self.context().post_json("/worker/client/status", request)?;
        let status_code = response.status_code;
        let body = response
            .as_str()
            .map_err(|e| GitAiError::Generic(format!("Failed to read response body: {}", e)))?;

        match status_code {
            200 => Ok(()),
            400 => {
                // Keep a useful error when the server body is not valid JSON.
                let error_response: ApiErrorResponse =
                    serde_json::from_str(body).unwrap_or_else(|_| ApiErrorResponse {
                        error: "Invalid request body".to_string(),
                        details: Some(serde_json::Value::String(body.to_string())),
                    });
                Err(GitAiError::Generic(format!(
                    "Bad Request: {}",
                    error_response.error
                )))
            }
            401 => Err(GitAiError::Generic("Unauthorized".to_string())),
            500 => Err(GitAiError::Generic(format!(
                "Internal Server Error: {}",
                body
            ))),
            _ => Err(GitAiError::Generic(format!(
                "Unexpected status code {}: {}",
                status_code, body
            ))),
        }
    }
}

/// Reads the hostname from platform variables, then falls back to `hostname`.
fn collect_hostname() -> Option<String> {
    if let Ok(hostname) = std::env::var("HOSTNAME")
        && !hostname.trim().is_empty()
    {
        return Some(hostname);
    }

    if let Ok(hostname) = std::env::var("COMPUTERNAME")
        && !hostname.trim().is_empty()
    {
        return Some(hostname);
    }

    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|hostname| hostname.trim().to_string())
        .filter(|hostname| !hostname.is_empty())
}
