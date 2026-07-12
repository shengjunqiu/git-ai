//! Metrics API endpoints

use crate::api::client::ApiClient;
use crate::api::types::ApiErrorResponse;
use crate::error::GitAiError;
use crate::metrics::MetricsBatch;
use crate::observability::log_error;
pub use git_ai_protocol::metrics::{
    MetricUploadError as MetricsUploadError, MetricsUploadResponse,
};

/// Retry delay in seconds: single retry after 60s
const RETRY_DELAYS_SECS: [u64; 1] = [60];

/// Upload metrics batch with retry logic.
///
/// Returns Ok(()) on success (200 response, even with partial errors).
/// Returns Err on failure after all retries exhausted.
///
/// Partial errors (200 + errors array) are logged to Sentry but not retried,
/// since validation errors won't succeed on retry.
pub fn upload_metrics_with_retry(
    client: &ApiClient,
    batch: &MetricsBatch,
    operation: &str,
) -> Result<(), GitAiError> {
    // First attempt (no delay), then retry with delays
    for (attempt, delay_secs) in std::iter::once(&0u64)
        .chain(RETRY_DELAYS_SECS.iter())
        .enumerate()
    {
        if attempt > 0 {
            eprintln!(
                "[metrics] Retrying upload after {}s delay (attempt {}/{})",
                delay_secs,
                attempt + 1,
                RETRY_DELAYS_SECS.len() + 1
            );
            std::thread::sleep(std::time::Duration::from_secs(*delay_secs));
        }

        match client.upload_metrics(batch) {
            Ok(response) => {
                // 200 response - log any validation errors to Sentry
                for error in &response.errors {
                    log_error(
                        &GitAiError::Generic(format!(
                            "Metrics {} error at index {}: {}",
                            operation, error.index, error.error
                        )),
                        Some(serde_json::json!({
                            "operation": operation,
                            "error_index": error.index
                        })),
                    );
                }
                return Ok(());
            }
            Err(e) => {
                // Administrator authorization is a durable policy decision, not a
                // transient transport failure. Return immediately so the daemon can
                // persist the events locally instead of sleeping for 60 seconds.
                if matches!(e, GitAiError::UploadForbidden(_)) {
                    eprintln!("[metrics] Upload blocked by administrator authorization");
                    return Err(e);
                }

                // Non-200 - will retry if attempts remain
                if attempt == RETRY_DELAYS_SECS.len() {
                    eprintln!("[metrics] All retries exhausted, giving up");
                    return Err(e);
                }
                eprintln!("[metrics] Upload failed: {}, will retry...", e);
            }
        }
    }

    Err(GitAiError::Generic(
        "All upload retries exhausted".to_string(),
    ))
}

/// Metrics API endpoints
impl ApiClient {
    /// Upload metrics batch to the server (max 250 events)
    ///
    /// # Arguments
    /// * `batch` - The metrics batch to upload
    ///
    /// # Returns
    /// * `Ok(MetricsUploadResponse)` - Response with errors (empty = all success)
    /// * `Err(GitAiError)` - Request failed
    pub fn upload_metrics(
        &self,
        batch: &MetricsBatch,
    ) -> Result<MetricsUploadResponse, GitAiError> {
        let response = self.context().post_json("/worker/metrics/upload", batch)?;
        let status_code = response.status_code;

        let body = response
            .as_str()
            .map_err(|e| GitAiError::Generic(format!("Failed to read response body: {}", e)))?;

        match status_code {
            200 => {
                let metrics_response: MetricsUploadResponse =
                    serde_json::from_str(body).map_err(GitAiError::JsonError)?;
                Ok(metrics_response)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_successful_indices() {
        let response = MetricsUploadResponse {
            errors: vec![
                MetricsUploadError {
                    index: 1,
                    error: "error".to_string(),
                },
                MetricsUploadError {
                    index: 3,
                    error: "error".to_string(),
                },
            ],
        };

        let successful = response.successful_indices(5);
        assert_eq!(successful, vec![0, 2, 4]);
    }

    #[test]
    fn test_successful_indices_empty_errors() {
        let response = MetricsUploadResponse { errors: vec![] };
        let successful = response.successful_indices(3);
        assert_eq!(successful, vec![0, 1, 2]);
    }

    #[test]
    fn test_successful_indices_all_errors() {
        let response = MetricsUploadResponse {
            errors: vec![
                MetricsUploadError {
                    index: 0,
                    error: "error".to_string(),
                },
                MetricsUploadError {
                    index: 1,
                    error: "error".to_string(),
                },
            ],
        };
        let successful = response.successful_indices(2);
        assert!(successful.is_empty());
    }

    #[test]
    fn upload_forbidden_error_is_identifiable_without_string_matching() {
        let error = GitAiError::UploadForbidden(
            "Developer is not authorized to upload Git tracking data".to_string(),
        );

        assert!(matches!(error, GitAiError::UploadForbidden(_)));
        assert!(error.to_string().contains("Upload forbidden"));
    }
}
