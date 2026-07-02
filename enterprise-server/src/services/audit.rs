//! Audit logging service
//!
//! Records user actions for compliance and security monitoring.

use sqlx::PgPool;
use uuid::Uuid;

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
