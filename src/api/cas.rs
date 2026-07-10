//! Client-side bindings for the content-addressable storage (CAS) API.
//!
//! CAS payloads are addressed by their content hash. This module only translates
//! between the shared HTTP client and the CAS request/response types; authentication,
//! base-URL handling, and transport errors remain centralized in [`ApiContext`].
//!
//! [`ApiContext`]: crate::api::client::ApiContext

//

use crate::api::client::ApiClient;
use crate::api::types::{
    ApiErrorResponse, CAPromptStoreReadResponse, CasUploadRequest, CasUploadResponse,
};
use crate::error::GitAiError;

/// CAS-specific extensions to the shared API client.
impl ApiClient {
    /// Uploads a batch of content-addressed objects to the server.
    ///
    /// A successful HTTP response can still contain per-object failures in
    /// [`CasUploadResponse`]. Batch sizing and the decision to retry failed objects
    /// are intentionally left to the caller.
    ///
    /// # Arguments
    /// * `request` - The CAS upload request containing objects to upload
    ///
    /// # Returns
    /// * `Ok(CasUploadResponse)` - Success response
    /// * `Err(GitAiError)` - Error response
    pub fn upload_cas(&self, request: CasUploadRequest) -> Result<CasUploadResponse, GitAiError> {
        // ApiContext adds the configured credentials and common request headers.
        let response = self.context().post_json("/worker/cas/upload", &request)?;
        let status_code = response.status_code;

        // Read the body once before dispatching on the status so both successful
        // payloads and server-provided error details use the same response data.
        let body = response
            .as_str()
            .map_err(|e| GitAiError::Generic(format!("Failed to read response body: {}", e)))?;

        match status_code {
            200 => {
                // The response reports the outcome of each object independently;
                // callers use it to remove only successful objects from local queues.
                let cas_response: CasUploadResponse =
                    serde_json::from_str(body).map_err(GitAiError::JsonError)?;
                Ok(cas_response)
            }
            400 => {
                // Some compatible servers may return a non-JSON error page. Keep a
                // stable user-facing message while retaining the raw body in details.
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
            403 => {
                // Authorization denial is a durable administrator policy decision,
                // not a transient HTTP failure. The dedicated variant lets upload
                // workers avoid retry delays and keep the objects queued locally.
                let error_response: ApiErrorResponse =
                    serde_json::from_str(body).unwrap_or_else(|_| ApiErrorResponse {
                        error: "Administrator authorization is required for Git tracking uploads"
                            .to_string(),
                        details: Some(serde_json::Value::String(body.to_string())),
                    });
                Err(GitAiError::UploadForbidden(error_response.error))
            }
            500 => {
                // As with 400 responses, tolerate legacy or proxy-generated bodies
                // that do not follow the structured API error schema.
                let error_response: ApiErrorResponse =
                    serde_json::from_str(body).unwrap_or_else(|_| ApiErrorResponse {
                        error: "Internal server error".to_string(),
                        details: None,
                    });
                Err(GitAiError::Generic(format!(
                    "Internal Server Error: {}",
                    error_response.error
                )))
            }
            _ => Err(GitAiError::Generic(format!(
                "Unexpected status code {}: {}",
                status_code, body
            ))),
        }
    }

    /// Reads a batch of CAS objects by content hash.
    ///
    /// Missing objects are represented as data rather than transport failures: an
    /// all-missing `404` response becomes an empty successful result, while mixed
    /// batches are described by the per-hash statuses returned with `200`.
    ///
    /// # Arguments
    /// * `hashes` - Slice of CAS hashes to fetch (max 100 per call)
    ///
    /// # Returns
    /// * `Ok(CAPromptStoreReadResponse)` - Response with results for each hash
    /// * `Err(GitAiError)` - On network or server errors
    pub fn read_ca_prompt_store(
        &self,
        hashes: &[&str],
    ) -> Result<CAPromptStoreReadResponse, GitAiError> {
        // Hashes are interpolated into a comma-separated query value rather than
        // passed through a URL encoder. Restricting them to their canonical hex
        // alphabet prevents `&`, `=`, or `,` from changing the query structure.
        for hash in hashes {
            if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(GitAiError::Generic(format!(
                    "CAS hash contains non-hex characters: {}",
                    hash
                )));
            }
        }

        // The server accepts a comma-separated batch (currently at most 100 hashes;
        // callers are responsible for chunking larger collections).
        let query = hashes.join(",");
        let endpoint = format!("/worker/cas/?hashes={}", query);
        let response = self.context().get(&endpoint)?;
        let status_code = response.status_code;

        // Preserve the body for either response deserialization or diagnostics on
        // an unexpected status code.
        let body = response
            .as_str()
            .map_err(|e| GitAiError::Generic(format!("Failed to read response body: {}", e)))?;

        match status_code {
            200 => {
                let cas_response: CAPromptStoreReadResponse =
                    serde_json::from_str(body).map_err(GitAiError::JsonError)?;
                Ok(cas_response)
            }
            404 => {
                // An all-missing batch is a cache miss, not an operational error.
                // This allows callers to continue to their local-database fallback.
                Ok(CAPromptStoreReadResponse {
                    results: Vec::new(),
                    success_count: 0,
                    failure_count: hashes.len(),
                })
            }
            _ => Err(GitAiError::Generic(format!(
                "CAS read failed with status {}: {}",
                status_code, body
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::client::{ApiClient, ApiContext};
    use crate::api::types::{CasObject, CasUploadRequest};
    use std::collections::HashMap;

    /// Query delimiters and other non-hex characters must not reach the HTTP layer.
    #[test]
    fn test_cas_hash_validation_rejects_non_hex() {
        let ctx = ApiContext::without_auth(Some("https://example.com".to_string()));
        let client = ApiClient::new(ctx);

        let result = client.read_ca_prompt_store(&["abc123", "not-hex!"]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            GitAiError::Generic(msg) => {
                assert!(
                    msg.contains("non-hex characters"),
                    "Error should mention non-hex characters, got: {}",
                    msg
                );
            }
            _ => panic!("Expected Generic error, got: {:?}", err),
        }
    }

    /// Valid hashes must pass local validation even if the subsequent request fails.
    #[test]
    fn test_cas_hash_validation_accepts_hex() {
        // `.invalid` is reserved for names that cannot resolve, so this isolates the
        // validation assertion without depending on a live CAS server.
        let ctx = ApiContext::without_auth(Some("https://nonexistent.invalid".to_string()));
        let client = ApiClient::new(ctx);

        let result = client.read_ca_prompt_store(&["abc123", "def456"]);
        if let Err(GitAiError::Generic(msg)) = result {
            assert!(
                !msg.contains("non-hex characters"),
                "Hex hashes should not trigger validation error, got: {}",
                msg
            );
        }
    }

    /// CAS uploads must preserve the hash, content, and metadata wire fields.
    #[test]
    fn test_cas_upload_request_serialization() {
        let mut metadata = HashMap::new();
        metadata.insert("kind".to_string(), "prompt".to_string());
        metadata.insert("api_version".to_string(), "v1".to_string());

        let request = CasUploadRequest {
            objects: vec![CasObject {
                content: serde_json::json!({"messages": [{"role": "user", "content": "test"}]}),
                hash: "a1b2c3d4e5f6".to_string(),
                metadata,
            }],
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("a1b2c3d4e5f6"));
        assert!(json.contains("prompt"));
    }

    /// Empty batches remain serializable for callers that use them as probes.
    #[test]
    fn test_cas_upload_request_empty() {
        let request = CasUploadRequest { objects: vec![] };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"objects\":[]"));
    }
}
