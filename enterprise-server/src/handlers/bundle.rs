use axum::extract::State;
use axum::response::Json;
use serde_json::Value;
use uuid::Uuid;

use crate::auth::middleware::AuthExtractor;
use crate::error::AppError;
use crate::models::bundle::CreateBundleRequest;
use crate::routes::AppState;

/// POST /api/bundles — Create a share bundle
pub async fn create_bundle(
    State(state): State<AppState>,
    auth: AuthExtractor,
    Json(req): Json<CreateBundleRequest>,
) -> Result<Json<Value>, AppError> {
    if req.title.is_empty() {
        return Err(AppError::BadRequest("Title must have at least 1 character".into()));
    }
    if req.data.prompts.is_empty() {
        return Err(AppError::BadRequest("At least one prompt is required".into()));
    }

    let bundle_id = Uuid::new_v4();
    let share_url = format!("{}/bundle/{}", state.config.base_url, bundle_id);
    let data_json = serde_json::to_value(&req.data)
        .map_err(|e| AppError::BadRequest(format!("Invalid bundle data: {}", e)))?;

    sqlx::query(
        "INSERT INTO bundles (id, user_id, title, data, share_url) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(bundle_id)
    .bind(auth.0.user_id)
    .bind(&req.title)
    .bind(&data_json)
    .bind(&share_url)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Store prompts
    for (hash, content) in &req.data.prompts {
        sqlx::query(
            "INSERT INTO bundle_prompts (bundle_id, hash, content) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING"
        )
        .bind(bundle_id)
        .bind(hash)
        .bind(content)
        .execute(&state.db)
        .await
        .ok();
    }

    // Store files
    for (file_path, file_record) in &req.data.files {
        let annotations_json = serde_json::to_value(&file_record.annotations).ok();
        sqlx::query(
            "INSERT INTO bundle_files (bundle_id, file_path, annotations, diff, base_content) VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING"
        )
        .bind(bundle_id)
        .bind(file_path)
        .bind(&annotations_json)
        .bind(&file_record.diff)
        .bind(&file_record.base_content)
        .execute(&state.db)
        .await
        .ok();
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "id": bundle_id.to_string(),
        "url": share_url,
    })))
}
