use crate::api::client::ApiClient;
use crate::api::types::{
    ApiErrorResponse, CAPromptStoreReadResponse, CasUploadRequest, CasUploadResponse,
};
use crate::error::GitAiError;

/// CAS API endpoints
impl ApiClient {
    /// Upload CAS objects to the server
    ///
    /// # Arguments
    /// * `request` - The CAS upload request containing objects to upload
    ///
    /// # Returns
    /// * `Ok(CasUploadResponse)` - Success response
    /// * `Err(GitAiError)` - Error response
    pub fn upload_cas(&self, request: CasUploadRequest) -> Result<CasUploadResponse, GitAiError> {
        let response = self.context().post_json("/worker/cas/upload", &request)?;
        let status_code = response.status_code;

        let body = response
            .as_str()
            .map_err(|e| GitAiError::Generic(format!("Failed to read response body: {}", e)))?;

        match status_code {
            200 => {
                let cas_response: CasUploadResponse =
                    serde_json::from_str(body).map_err(GitAiError::JsonError)?;
                Ok(cas_response)
            }
            400 => {
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
                let error_response: ApiErrorResponse =
                    serde_json::from_str(body).unwrap_or_else(|_| ApiErrorResponse {
                        error: "Administrator authorization is required for Git tracking uploads"
                            .to_string(),
                        details: Some(serde_json::Value::String(body.to_string())),
                    });
                Err(GitAiError::UploadForbidden(error_response.error))
            }
            500 => {
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

    /// Read CAS objects by hash from the server
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
        // Validate all hashes are hex-only before building the URL to prevent
        // injection via crafted hash values in the query string.
        for hash in hashes {
            if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(GitAiError::Generic(format!(
                    "CAS hash contains non-hex characters: {}",
                    hash
                )));
            }
        }

        let query = hashes.join(",");
        let endpoint = format!("/worker/cas/?hashes={}", query);
        let response = self.context().get(&endpoint)?;
        let status_code = response.status_code;

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
                // All hashes not found — return empty response gracefully
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

    /// Test that CAS hash validation rejects non-hex characters
    #[test]
    fn test_cas_hash_validation_rejects_non_hex() {
        let ctx = ApiContext::without_auth(Some("https://example.com".to_string()));
        let client = ApiClient::new(ctx);

        // Non-hex characters should be rejected
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

    /// Test that CAS hash validation accepts valid hex hashes
    #[test]
    fn test_cas_hash_validation_accepts_hex() {
        // We can't test the actual HTTP call, but we can verify the validation
        // by checking that hex-only hashes don't trigger the validation error.
        // The actual HTTP call will fail (no server), but it should fail with
        // a network error, not a validation error.
        let ctx = ApiContext::without_auth(Some("https://nonexistent.invalid".to_string()));
        let client = ApiClient::new(ctx);

        let result = client.read_ca_prompt_store(&["abc123", "def456"]);
        // The validation should pass; the HTTP call will fail
        if let Err(GitAiError::Generic(msg)) = result {
            // Should NOT be the hash validation error
            assert!(
                !msg.contains("non-hex characters"),
                "Hex hashes should not trigger validation error, got: {}",
                msg
            );
        }
    }

    /// Test that CAS upload request serializes correctly
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

    /// Test that empty CAS upload request is valid
    #[test]
    fn test_cas_upload_request_empty() {
        let request = CasUploadRequest { objects: vec![] };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"objects\":[]"));
    }
}
