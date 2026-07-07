//! Registration validation and organization lookup helpers.

use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize)]
pub struct RegisterableOrganization {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegisterableDepartment {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
}

pub fn email_domain(email: &str) -> Result<String, AppError> {
    let trimmed = email.trim();
    let (local, domain) = trimmed
        .rsplit_once('@')
        .ok_or_else(|| AppError::BadRequest("Invalid email address".into()))?;

    let local = local.trim();
    let domain = domain.trim().to_ascii_lowercase();
    if local.is_empty() || local.contains('@') || domain.is_empty() || !domain.contains('.') {
        return Err(AppError::BadRequest("Invalid email address".into()));
    }

    Ok(domain)
}

pub async fn list_registerable_organizations(
    pool: &PgPool,
    email: &str,
) -> Result<Vec<RegisterableOrganization>, AppError> {
    let domain = email_domain(email)?;

    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT o.id, o.name, o.slug \
         FROM organization_domains od \
         JOIN organizations o ON o.id = od.org_id \
         WHERE od.domain = $1 AND od.verified = true \
         ORDER BY o.name",
    )
    .bind(&domain)
    .fetch_all(pool)
    .await
    .map_err(AppError::Database)?;

    Ok(rows
        .into_iter()
        .map(|(id, name, slug)| RegisterableOrganization { id, name, slug })
        .collect())
}

pub async fn list_departments_for_org(
    pool: &PgPool,
    org_id: Uuid,
) -> Result<Vec<RegisterableDepartment>, AppError> {
    let rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, name, slug \
         FROM departments \
         WHERE org_id = $1 \
         ORDER BY name",
    )
    .bind(org_id)
    .fetch_all(pool)
    .await
    .map_err(AppError::Database)?;

    Ok(rows
        .into_iter()
        .map(|(id, name, slug)| RegisterableDepartment { id, name, slug })
        .collect())
}

pub async fn find_org_id_by_slug(pool: &PgPool, slug: &str) -> Result<Uuid, AppError> {
    let org_id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM organizations WHERE slug = $1")
        .bind(slug)
        .fetch_optional(pool)
        .await
        .map_err(AppError::Database)?;

    org_id.ok_or_else(|| {
        AppError::BadRequest(format!(
            "Organization '{}' is not configured for registration",
            slug
        ))
    })
}

pub async fn find_department_id_by_slug(
    pool: &PgPool,
    org_id: Uuid,
    slug: &str,
) -> Result<Uuid, AppError> {
    let department_id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM departments WHERE org_id = $1 AND slug = $2")
            .bind(org_id)
            .bind(slug)
            .fetch_optional(pool)
            .await
            .map_err(AppError::Database)?;

    department_id.ok_or_else(|| {
        AppError::BadRequest(format!(
            "Department '{}' is not configured for registration",
            slug
        ))
    })
}

pub async fn validate_org_domain(pool: &PgPool, email: &str, org_id: Uuid) -> Result<(), AppError> {
    let domain = email_domain(email)?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS( \
            SELECT 1 \
            FROM organization_domains \
            WHERE org_id = $1 \
              AND domain = $2 \
              AND verified = true \
         )",
    )
    .bind(org_id)
    .bind(&domain)
    .fetch_one(pool)
    .await
    .map_err(AppError::Database)?;

    if !exists {
        return Err(AppError::Forbidden(
            "Email domain is not allowed for this organization".into(),
        ));
    }

    Ok(())
}

pub async fn validate_department(
    pool: &PgPool,
    org_id: Uuid,
    department_id: Uuid,
) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS( \
            SELECT 1 \
            FROM departments \
            WHERE id = $1 AND org_id = $2 \
         )",
    )
    .bind(department_id)
    .bind(org_id)
    .fetch_one(pool)
    .await
    .map_err(AppError::Database)?;

    if !exists {
        return Err(AppError::BadRequest(
            "Department does not belong to the selected organization".into(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_domain_lowercases_domain() {
        assert_eq!(email_domain("Alice@Linewell.COM").unwrap(), "linewell.com");
    }

    #[test]
    fn email_domain_rejects_invalid_email() {
        assert!(email_domain("not-an-email").is_err());
        assert!(email_domain("alice@localhost").is_err());
        assert!(email_domain("@linewell.com").is_err());
        assert!(email_domain("alice@linewell.com@example.com").is_err());
    }

    #[test]
    fn email_domain_trims_whitespace() {
        assert_eq!(
            email_domain("  Alice@Linewell.COM  ").unwrap(),
            "linewell.com"
        );
    }
}
