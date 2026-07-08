//! Audit logging service
//!
//! Records user actions for compliance and security monitoring.

use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AuditPayload {
    pub user_id: Option<Uuid>,
    pub org_id: Option<Uuid>,
    pub action: String,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub details: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

/// Record an audit log entry
pub async fn log_action(
    pool: &PgPool,
    user_id: Option<Uuid>,
    org_id: Option<Uuid>,
    action: &str,
    resource_type: Option<&str>,
    resource_id: Option<&str>,
    details: Option<serde_json::Value>,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO audit_log (user_id, org_id, action, resource_type, resource_id, details, ip_address, user_agent)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#
    )
    .bind(user_id)
    .bind(org_id)
    .bind(action)
    .bind(resource_type)
    .bind(resource_id)
    .bind(details)
    .bind(ip_address)
    .bind(user_agent)
    .execute(pool)
    .await?;

    Ok(())
}

pub fn spawn_log_action(pool: PgPool, payload: AuditPayload) {
    tokio::spawn(async move {
        let action = payload.action.clone();
        if let Err(error) = log_action(
            &pool,
            payload.user_id,
            payload.org_id,
            &payload.action,
            payload.resource_type.as_deref(),
            payload.resource_id.as_deref(),
            payload.details,
            payload.ip_address.as_deref(),
            payload.user_agent.as_deref(),
        )
        .await
        {
            tracing::warn!(?error, action = %action, "failed to write audit log");
        }
    });
}
