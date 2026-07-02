//! Phase 6: CI/CD and alert event ingestion handlers
//!
//! Receives CI/CD pipeline events and production alert events for
//! full lifecycle tracking of AI-generated code.

use axum::extract::State;
use axum::response::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthExtractor;
use crate::error::AppError;
use crate::routes::AppState;

// ================================================================
// CI/CD Events
// ================================================================

#[derive(Debug, Deserialize)]
pub struct CiEventRequest {
    pub event_type: String,             // "ci_run", "deployment", "pr_review"
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    pub org_slug: Option<String>,
    pub repo_url: String,
    pub commit_sha: String,
    pub deployment_env: Option<String>, // "production", "staging", "development"
    pub status: Option<String>,         // "success", "failure", "running"
    pub deployer: Option<String>,
    pub ci_platform: Option<String>,    // "github_actions", "gitlab_ci", "jenkins", etc.
    pub metadata: Option<serde_json::Value>,
}

/// POST /api/v1/ci-events — Receive CI/CD lifecycle events
pub async fn create_ci_event(
    State(state): State<AppState>,
    auth: AuthExtractor,
    Json(req): Json<CiEventRequest>,
) -> Result<Json<Value>, AppError> {
    let valid_types = ["ci_run", "deployment", "pr_review"];
    if !valid_types.contains(&req.event_type.as_str()) {
        return Err(AppError::BadRequest(format!(
            "event_type must be one of: {}", valid_types.join(", ")
        )));
    }

    let id = Uuid::new_v4();

    // Resolve org_id from slug if provided
    let org_id: Option<Uuid> = if let Some(slug) = &req.org_slug {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM organizations WHERE slug = $1"
        )
        .bind(slug)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?
    } else {
        auth.0.org_id
    };

    sqlx::query(
        r#"INSERT INTO ci_events (id, org_id, event_type, timestamp, repo_url, commit_sha,
           deployment_env, status, deployer, ci_platform, metadata)
        VALUES ($1, $2, $3, COALESCE($4, now()), $5, $6, $7, $8, $9, $10, $11)"#
    )
    .bind(id)
    .bind(org_id)
    .bind(&req.event_type)
    .bind(req.timestamp)
    .bind(&req.repo_url)
    .bind(&req.commit_sha)
    .bind(&req.deployment_env)
    .bind(&req.status)
    .bind(&req.deployer)
    .bind(&req.ci_platform)
    .bind(&req.metadata)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Audit log
    crate::services::audit::log_action(
        &state.db, Some(auth.0.user_id), org_id,
        "ci_event.create", Some("ci_event"), Some(&id.to_string()),
        Some(json!({
            "event_type": req.event_type,
            "commit_sha": req.commit_sha,
            "repo_url": req.repo_url,
        })),
        None, None,
    ).await.ok();

    Ok(Json(json!({
        "id": id.to_string(),
        "event_type": req.event_type,
        "commit_sha": req.commit_sha,
    })))
}

// ================================================================
// Alert Events
// ================================================================

#[derive(Debug, Deserialize)]
pub struct AlertEventRequest {
    pub event_type: Option<String>,     // defaults to "alert"
    pub alert_source: String,           // "pagerduty", "datadog", "grafana", "custom"
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    pub org_slug: Option<String>,
    pub repo_url: String,
    pub commit_sha: String,
    pub severity: Option<String>,       // "info", "warning", "critical"
    pub description: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// POST /api/v1/alert-events — Receive production alert events
pub async fn create_alert_event(
    State(state): State<AppState>,
    auth: AuthExtractor,
    Json(req): Json<AlertEventRequest>,
) -> Result<Json<Value>, AppError> {
    let valid_sources = ["pagerduty", "datadog", "grafana", "custom", "sentry", "opsgenie"];
    if !valid_sources.contains(&req.alert_source.as_str()) {
        return Err(AppError::BadRequest(format!(
            "alert_source must be one of: {}", valid_sources.join(", ")
        )));
    }

    let severity = req.severity.as_deref().unwrap_or("info");
    if !["info", "warning", "critical"].contains(&severity) {
        return Err(AppError::BadRequest("severity must be 'info', 'warning', or 'critical'".into()));
    }

    let id = Uuid::new_v4();

    let org_id: Option<Uuid> = if let Some(slug) = &req.org_slug {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM organizations WHERE slug = $1"
        )
        .bind(slug)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?
    } else {
        auth.0.org_id
    };

    sqlx::query(
        r#"INSERT INTO alert_events (id, org_id, alert_source, event_type, timestamp,
           repo_url, commit_sha, severity, description, metadata)
        VALUES ($1, $2, $3, COALESCE($4, 'alert'), COALESCE($5, now()), $6, $7, $8, $9, $10)"#
    )
    .bind(id)
    .bind(org_id)
    .bind(&req.alert_source)
    .bind(&req.event_type)
    .bind(req.timestamp)
    .bind(&req.repo_url)
    .bind(&req.commit_sha)
    .bind(severity)
    .bind(&req.description)
    .bind(&req.metadata)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Audit log
    crate::services::audit::log_action(
        &state.db, Some(auth.0.user_id), org_id,
        "alert_event.create", Some("alert_event"), Some(&id.to_string()),
        Some(json!({
            "alert_source": req.alert_source,
            "severity": severity,
            "commit_sha": req.commit_sha,
        })),
        None, None,
    ).await.ok();

    Ok(Json(json!({
        "id": id.to_string(),
        "alert_source": req.alert_source,
        "severity": severity,
        "commit_sha": req.commit_sha,
    })))
}
