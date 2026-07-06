//! Metrics service - handles decoding, storing, and querying metrics events

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;
use crate::models::metrics::{DecodedMetricEvent, MetricEvent, MetricUploadError, MetricsUploadResponse};
use crate::pos_encoded::decode_event;

/// Process a batch of metrics events
pub async fn process_metrics_batch(
    pool: &PgPool,
    events: Vec<MetricEvent>,
    user_id: Option<Uuid>,
    distinct_id: Option<String>,
) -> MetricsUploadResponse {
    let mut errors = Vec::new();

    for (idx, event) in events.iter().enumerate() {
        match decode_event(event) {
            Ok(decoded) => {
                if let Err(e) = store_event(pool, &decoded, user_id, &distinct_id).await {
                    tracing::warn!("Failed to store metrics event at index {}: {}", idx, e);
                    errors.push(MetricUploadError {
                        index: idx,
                        error: format!("Storage error: {}", e),
                    });
                }
            }
            Err(e) => {
                tracing::warn!("Failed to decode metrics event at index {}: {}", idx, e);
                errors.push(MetricUploadError {
                    index: idx,
                    error: format!("Decode error: {}", e),
                });
            }
        }
    }

    MetricsUploadResponse { errors }
}

/// Store a decoded metrics event in the database
async fn store_event(
    pool: &PgPool,
    event: &DecodedMetricEvent,
    user_id: Option<Uuid>,
    distinct_id: &Option<String>,
) -> Result<(), AppError> {
    let org_id = if let Some(uid) = user_id {
        crate::services::org_scope::preferred_org_scope(pool, uid)
            .await?
            .map(|scope| scope.org_id)
    } else {
        None
    };

    let ai_additions_total = event
        .ai_additions
        .as_ref()
        .map(|v| v.iter().sum::<i32>())
        .unwrap_or(0);

    let raw_values_json = serde_json::to_value(&event.raw_values)
        .map_err(|e| AppError::Internal(format!("Failed to serialize raw_values: {}", e)))?;
    let raw_attrs_json = serde_json::to_value(&event.raw_attrs)
        .map_err(|e| AppError::Internal(format!("Failed to serialize raw_attrs: {}", e)))?;

    let tool_model_pairs_json = event
        .tool_model_pairs
        .as_ref()
        .map(|v| serde_json::to_value(v).unwrap_or(serde_json::Value::Null));

    let custom_attrs_json = event
        .custom_attributes
        .as_ref()
        .map(|v| serde_json::to_value(v).unwrap_or(serde_json::Value::Null));

    let effective_distinct_id = event.distinct_id.as_deref().or(distinct_id.as_deref());

    sqlx::query(
        r#"INSERT INTO metrics_events (
            event_type, timestamp, user_id, distinct_id, org_id,
            repo_url, author_email, tool, model, commit_sha,
            human_additions, ai_additions,
            git_diff_added_lines, git_diff_deleted_lines,
            tool_model_pairs, prompt_id, session_id, file_path,
            custom_attributes, raw_values, raw_attrs
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21)"#
    )
    .bind(event.event_type as i32)
    .bind(event.timestamp)
    .bind(user_id)
    .bind(effective_distinct_id)
    .bind(org_id)
    .bind(&event.repo_url)
    .bind(&event.author)
    .bind(&event.tool)
    .bind(&event.model)
    .bind(&event.commit_sha)
    .bind(event.human_additions)
    .bind(ai_additions_total)
    .bind(event.git_diff_added_lines)
    .bind(event.git_diff_deleted_lines)
    .bind(&tool_model_pairs_json)
    .bind(&event.prompt_id)
    .bind(&event.session_id)
    .bind(&event.file_path)
    .bind(&custom_attrs_json)
    .bind(&raw_values_json)
    .bind(&raw_attrs_json)
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e))?;

    Ok(())
}
