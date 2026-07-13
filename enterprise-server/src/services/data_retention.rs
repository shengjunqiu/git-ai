//! Data retention policy management and CAS access audit service
//!
//! Provides CRUD for per-organization data retention policies,
//! periodic purge of expired data, and CAS access audit logging.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

// ================================================================
// Data retention policy management
// ================================================================

/// Get the retention policy for an organization (returns defaults if none set)
pub async fn get_retention_policy(
    pool: &PgPool,
    org_id: Uuid,
) -> Result<serde_json::Value, AppError> {
    let row: Option<(Uuid, Uuid, i32, i32, i32, i32, i32, bool, chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        r#"SELECT id, org_id, metrics_retention_days, cas_retention_days, audit_retention_days,
                  ci_events_retention_days, alerts_retention_days, auto_purge, created_at, updated_at
        FROM data_retention_policies WHERE org_id = $1"#
    )
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Database(e))?;

    Ok(match row {
        Some((id, oid, metrics, cas, audit, ci, alerts, auto_purge, created, updated)) => {
            serde_json::json!({
                "id": id.to_string(),
                "org_id": oid.to_string(),
                "metrics_retention_days": metrics,
                "cas_retention_days": cas,
                "audit_retention_days": audit,
                "ci_events_retention_days": ci,
                "alerts_retention_days": alerts,
                "auto_purge": auto_purge,
                "created_at": created,
                "updated_at": updated,
            })
        }
        None => {
            // Return defaults
            serde_json::json!({
                "org_id": org_id.to_string(),
                "metrics_retention_days": 365,
                "cas_retention_days": 365,
                "audit_retention_days": 730,
                "ci_events_retention_days": 365,
                "alerts_retention_days": 365,
                "auto_purge": false,
            })
        }
    })
}

/// Upsert a retention policy for an organization
pub async fn upsert_retention_policy(
    pool: &PgPool,
    org_id: Uuid,
    metrics_days: Option<i32>,
    cas_days: Option<i32>,
    audit_days: Option<i32>,
    ci_days: Option<i32>,
    alerts_days: Option<i32>,
    auto_purge: Option<bool>,
) -> Result<serde_json::Value, AppError> {
    sqlx::query(
        r#"INSERT INTO data_retention_policies (org_id, metrics_retention_days, cas_retention_days,
           audit_retention_days, ci_events_retention_days, alerts_retention_days, auto_purge)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (org_id) DO UPDATE SET
            metrics_retention_days = EXCLUDED.metrics_retention_days,
            cas_retention_days = EXCLUDED.cas_retention_days,
            audit_retention_days = EXCLUDED.audit_retention_days,
            ci_events_retention_days = EXCLUDED.ci_events_retention_days,
            alerts_retention_days = EXCLUDED.alerts_retention_days,
            auto_purge = EXCLUDED.auto_purge,
            updated_at = now()"#,
    )
    .bind(org_id)
    .bind(metrics_days.unwrap_or(365))
    .bind(cas_days.unwrap_or(365))
    .bind(audit_days.unwrap_or(730))
    .bind(ci_days.unwrap_or(365))
    .bind(alerts_days.unwrap_or(365))
    .bind(auto_purge.unwrap_or(false))
    .execute(pool)
    .await
    .map_err(|e| AppError::Database(e))?;

    get_retention_policy(pool, org_id).await
}

/// Purge expired data based on retention policies
/// Should be called periodically (e.g., daily via cron or automation)
pub async fn purge_expired_data(pool: &PgPool) -> Result<serde_json::Value, AppError> {
    // Get all orgs with auto_purge enabled
    let orgs: Vec<(Uuid, i32, i32, i32, i32, i32)> = sqlx::query_as(
        r#"SELECT org_id, metrics_retention_days, cas_retention_days,
                  audit_retention_days, ci_events_retention_days, alerts_retention_days
        FROM data_retention_policies WHERE auto_purge = true"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Database(e))?;

    let mut purged = serde_json::json!({
        "metrics_events": 0,
        "ci_events": 0,
        "alert_events": 0,
        "cas_access_log": 0,
    });

    for (org_id, metrics_days, _cas_days, audit_days, ci_days, alerts_days) in &orgs {
        // Purge old metrics events
        let result = sqlx::query(
            "DELETE FROM metrics_events WHERE org_id = $1 AND created_at < now() - ($2 || ' days')::interval"
        )
        .bind(org_id)
        .bind(metrics_days.to_string())
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e))?;
        purged["metrics_events"] = serde_json::json!(
            purged["metrics_events"].as_i64().unwrap_or(0) + result.rows_affected() as i64
        );

        // Purge old CI events
        let result = sqlx::query(
            "DELETE FROM ci_events WHERE org_id = $1 AND created_at < now() - ($2 || ' days')::interval"
        )
        .bind(org_id)
        .bind(ci_days.to_string())
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e))?;
        purged["ci_events"] = serde_json::json!(
            purged["ci_events"].as_i64().unwrap_or(0) + result.rows_affected() as i64
        );

        // Purge old alert events
        let result = sqlx::query(
            "DELETE FROM alert_events WHERE org_id = $1 AND created_at < now() - ($2 || ' days')::interval"
        )
        .bind(org_id)
        .bind(alerts_days.to_string())
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e))?;
        purged["alert_events"] = serde_json::json!(
            purged["alert_events"].as_i64().unwrap_or(0) + result.rows_affected() as i64
        );

        // Purge old CAS access log entries
        let result = sqlx::query(
            "DELETE FROM cas_access_log WHERE org_id = $1 AND created_at < now() - ($2 || ' days')::interval"
        )
        .bind(org_id)
        .bind(audit_days.to_string())
        .execute(pool)
        .await
        .map_err(|e| AppError::Database(e))?;
        purged["cas_access_log"] = serde_json::json!(
            purged["cas_access_log"].as_i64().unwrap_or(0) + result.rows_affected() as i64
        );
    }

    Ok(purged)
}

// ================================================================
// CAS access audit logging
// ================================================================

/// Log a CAS access event for audit purposes
pub async fn log_cas_access(
    pool: &PgPool,
    user_id: Option<Uuid>,
    org_id: Option<Uuid>,
    api_key_id: Option<Uuid>,
    cas_hash: &str,
    access_method: &str, // "api", "dashboard", "ide_plugin"
    purpose: Option<&str>,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
) -> Result<(), sqlx::Error> {
    let valid_methods = ["api", "dashboard", "ide_plugin"];
    let method = if valid_methods.contains(&access_method) {
        access_method
    } else {
        "api"
    };

    sqlx::query(
        r#"INSERT INTO cas_access_log (user_id, org_id, api_key_id, cas_hash, access_method, purpose, ip_address, user_agent)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#
    )
    .bind(user_id)
    .bind(org_id)
    .bind(api_key_id)
    .bind(cas_hash)
    .bind(method)
    .bind(purpose)
    .bind(ip_address)
    .bind(user_agent)
    .execute(pool)
    .await?;

    Ok(())
}
