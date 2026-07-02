//! Telemetry proxy handlers
//!
//! Proxies Sentry and PostHog telemetry to their respective backends,
//! supporting enterprise self-hosted deployments.

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;

use crate::error::AppError;
use crate::routes::AppState;

/// POST /worker/telemetry/sentry/{project_id}/store/ — Sentry event proxy
///
/// Receives Sentry envelope events and forwards them to the configured
/// enterprise Sentry DSN. Supports dual-DSN (OSS + Enterprise).
pub async fn sentry_proxy(
    State(state): State<AppState>,
    Path(project_id): Path<String>,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    tracing::debug!(
        "Sentry event received: project_id={}, size={} bytes",
        project_id,
        body.len()
    );

    // If a Sentry DSN is configured, forward the event
    if !state.config.sentry_dsn.is_empty() {
        let sentry_url = format!(
            "{}/api/{}/store/",
            state.config.sentry_dsn.trim_end_matches('/'),
            project_id
        );

        let client = reqwest::Client::new();
        let response = client
            .post(&sentry_url)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(body.to_vec())
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        match response {
            Ok(resp) => {
                let status = resp.status();
                tracing::debug!("Sentry forward response: {}", status);
                return Ok(StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK).into_response());
            }
            Err(e) => {
                tracing::warn!("Sentry forward failed: {}", e);
                // Still return 200 to client so it doesn't retry
            }
        }
    } else {
        tracing::debug!("No Sentry DSN configured, event dropped");
    }

    // Always return 200 to prevent client retries
    Ok(StatusCode::OK.into_response())
}

/// POST /worker/telemetry/posthog/capture/ — PostHog event proxy
///
/// Receives PostHog analytics events and forwards them to the configured
/// PostHog host.
pub async fn posthog_proxy(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Response, AppError> {
    tracing::debug!("PostHog event received");

    if !state.config.posthog_host.is_empty() && !state.config.posthog_api_key.is_empty() {
        let url = format!(
            "{}/capture/",
            state.config.posthog_host.trim_end_matches('/')
        );

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header(header::CONTENT_TYPE, "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        match response {
            Ok(resp) => {
                let status = resp.status();
                tracing::debug!("PostHog forward response: {}", status);
                return Ok(StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK).into_response());
            }
            Err(e) => {
                tracing::warn!("PostHog forward failed: {}", e);
            }
        }
    } else {
        tracing::debug!("No PostHog configured, event dropped");
    }

    Ok(StatusCode::OK.into_response())
}
