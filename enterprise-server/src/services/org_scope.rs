//! Organization scope helpers for authenticated users.

use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct OrgScope {
    pub org_id: Uuid,
    pub org_slug: String,
    pub department_id: Option<Uuid>,
    pub role: String,
}

pub async fn preferred_org_scope(
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<Option<OrgScope>, AppError> {
    let row: Option<(Uuid, String, Option<Uuid>, String)> = sqlx::query_as(
        "SELECT om.org_id, o.slug, om.department_id, om.role \
         FROM org_members om \
         JOIN organizations o ON o.id = om.org_id \
         JOIN users u ON u.id = om.user_id \
         WHERE om.user_id = $1 \
         ORDER BY CASE \
             WHEN u.default_org_id IS NOT NULL AND om.org_id = u.default_org_id THEN 0 \
             WHEN u.default_org_id IS NULL AND u.personal_org_id IS NOT NULL AND om.org_id <> u.personal_org_id THEN 0 \
             ELSE 1 \
         END, o.created_at \
         LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::Database)?;

    Ok(row.map(|(org_id, org_slug, department_id, role)| OrgScope {
        org_id,
        org_slug,
        department_id,
        role,
    }))
}

pub async fn org_scope_for_org(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    org_id: Uuid,
) -> Result<Option<OrgScope>, AppError> {
    let row: Option<(Uuid, String, Option<Uuid>, String)> = sqlx::query_as(
        "SELECT om.org_id, o.slug, om.department_id, om.role \
         FROM org_members om \
         JOIN organizations o ON o.id = om.org_id \
         WHERE om.user_id = $1 AND om.org_id = $2",
    )
    .bind(user_id)
    .bind(org_id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::Database)?;

    Ok(row.map(|(org_id, org_slug, department_id, role)| OrgScope {
        org_id,
        org_slug,
        department_id,
        role,
    }))
}
