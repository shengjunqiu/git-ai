use axum::extract::State;
use axum::response::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::middleware::{AuthExtractor, DashboardAuth, HeaderExtractor};
use crate::error::AppError;
use crate::routes::AppState;
use crate::services::client_status::{ClientStatus, ClientStatusMetadata};

#[derive(Debug, Deserialize)]
pub struct ClientStatusRequest {
    pub status: String,
    pub cli_version: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub hostname: Option<String>,
}

/// POST /worker/client/status — CLI login/logout status upload.
pub async fn update_client_status(
    State(state): State<AppState>,
    auth: AuthExtractor,
    headers: HeaderExtractor,
    Json(req): Json<ClientStatusRequest>,
) -> Result<Json<Value>, AppError> {
    let status = ClientStatus::parse(&req.status)?;

    crate::services::client_status::record_status(
        &state.db,
        auth.0.user_id,
        auth.0.org_id,
        status,
        ClientStatusMetadata {
            distinct_id: headers.0.distinct_id,
            cli_version: req.cli_version,
            os: req.os,
            arch: req.arch,
            hostname: req.hostname,
        },
    )
    .await?;

    Ok(Json(json!({ "ok": true, "status": status.as_str() })))
}

/// GET /api/v1/client/status — Current dashboard user's CLI status.
pub async fn current_client_status(
    State(state): State<AppState>,
    auth: DashboardAuth,
) -> Result<Json<Value>, AppError> {
    let Some(status) =
        crate::services::client_status::get_status(&state.db, auth.0.user_id).await?
    else {
        return Ok(Json(json!({
            "detected": false,
            "status": "unknown",
            "status_label": "未检测到",
        })));
    };

    let status_label = match status.status.as_str() {
        "logged_in" => "已登录",
        "logged_out" => "已登出",
        _ => "未检测到",
    };

    Ok(Json(json!({
        "detected": true,
        "device_key": status.device_key,
        "status": status.status,
        "status_label": status_label,
        "last_status_at": status.last_status_at,
        "last_seen_at": status.last_seen_at,
        "cli_version": status.cli_version,
        "os": status.os,
        "arch": status.arch,
        "hostname": status.hostname,
        "device_count": status.device_count,
        "devices": status.devices,
    })))
}
