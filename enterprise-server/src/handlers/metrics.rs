use axum::extract::State;
use axum::response::Json;
use serde_json::Value;

use crate::auth::middleware::{AuthExtractor, HeaderExtractor};
use crate::models::metrics::MetricsBatch;
use crate::routes::AppState;

/// POST /worker/metrics/upload — Batch upload metrics events
///
/// Supports partial success: HTTP 200 with errors array for failed events.
/// Client only retries on 400/401/500, not on partial success.
pub async fn upload_metrics(
    State(state): State<AppState>,
    auth: AuthExtractor,
    headers: HeaderExtractor,
    Json(batch): Json<MetricsBatch>,
) -> Json<Value> {
    let event_count = batch.events.len();

    tracing::info!(
        "Metrics upload: {} events, distinct_id={:?}",
        event_count,
        headers.0.distinct_id,
    );

    let response = crate::services::metrics::process_metrics_batch(
        &state.db,
        batch.events,
        Some(auth.0.user_id),
        headers.0.distinct_id,
    )
    .await;

    let success_count = event_count - response.errors.len();
    tracing::info!(
        "Metrics upload result: {} success, {} errors",
        success_count,
        response.errors.len(),
    );

    Json(serde_json::to_value(response).unwrap_or_else(|_| {
        serde_json::json!({ "errors": [] })
    }))
}
