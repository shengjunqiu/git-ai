use axum::extract::State;
use axum::response::Json;
use serde_json::{json, Value};

use crate::routes::AppState;

/// Health check endpoint
pub async fn health_check(State(_state): State<AppState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "git-ai-enterprise-server",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Readiness check endpoint (verifies DB connection)
pub async fn readiness_check(State(state): State<AppState>) -> Result<Json<Value>, crate::error::AppError> {
    sqlx::query("SELECT 1")
        .execute(&state.db)
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("Database unreachable: {}", e)))?;

    Ok(Json(json!({
        "status": "ready",
        "checks": {
            "database": "ok"
        }
    })))
}
