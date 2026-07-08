use axum::extract::State;
use axum::response::Json;
use serde_json::Value;

use crate::auth::middleware::{AuthExtractor, HeaderExtractor};
use crate::error::AppError;
use crate::models::metrics::MetricsBatch;
use crate::routes::AppState;

const MAX_METRICS_BATCH_EVENTS: usize = 500;

/// POST /worker/metrics/upload — Batch upload metrics events
///
/// Supports partial success: HTTP 200 with errors array for failed events.
/// Client only retries on 400/401/500, not on partial success.
pub async fn upload_metrics(
    State(state): State<AppState>,
    auth: AuthExtractor,
    headers: HeaderExtractor,
    Json(batch): Json<MetricsBatch>,
) -> Result<Json<Value>, AppError> {
    validate_metrics_batch_size(batch.events.len())?;

    let event_count = batch.events.len();

    tracing::info!(
        "Metrics upload: {} events, distinct_id={:?}",
        event_count,
        headers.0.distinct_id,
    );

    let distinct_id = headers.0.distinct_id.clone();
    let response = crate::services::metrics::process_metrics_batch(
        &state.db,
        batch.events,
        Some(auth.0.user_id),
        distinct_id.clone(),
    )
    .await;

    let success_count = event_count - response.errors.len();
    if success_count > 0 {
        if let Err(e) = crate::services::client_status::touch_last_seen(
            &state.db,
            auth.0.user_id,
            auth.0.org_id,
            distinct_id,
        )
        .await
        {
            tracing::warn!(%e, "failed to update client last_seen_at after metrics upload");
        }
    }

    tracing::info!(
        "Metrics upload result: {} success, {} errors",
        success_count,
        response.errors.len(),
    );

    Ok(Json(
        serde_json::to_value(response).unwrap_or_else(|_| serde_json::json!({ "errors": [] })),
    ))
}

fn validate_metrics_batch_size(event_count: usize) -> Result<(), AppError> {
    if event_count > MAX_METRICS_BATCH_EVENTS {
        return Err(AppError::BadRequest(format!(
            "Maximum {} events per batch",
            MAX_METRICS_BATCH_EVENTS
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_metrics_batch_size() {
        assert!(validate_metrics_batch_size(MAX_METRICS_BATCH_EVENTS).is_ok());
        assert!(matches!(
            validate_metrics_batch_size(MAX_METRICS_BATCH_EVENTS + 1),
            Err(AppError::BadRequest(_))
        ));
    }
}
