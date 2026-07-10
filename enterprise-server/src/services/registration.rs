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
    pub code: String,
    pub name: String,
    pub slug: String,
    pub parent_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy)]
pub struct RegistrationScope {
    pub org_id: Uuid,
    pub department_id: Uuid,
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
    let rows: Vec<(Uuid, String, String, String, Option<Uuid>)> = sqlx::query_as(
        "SELECT id, code, name, slug, parent_id \
         FROM departments \
         WHERE org_id = $1 \
         ORDER BY code, name",
    )
    .bind(org_id)
    .fetch_all(pool)
    .await
    .map_err(AppError::Database)?;

    Ok(rows
        .into_iter()
        .map(|(id, code, name, slug, parent_id)| RegisterableDepartment {
            id,
            code,
            name,
            slug,
            parent_id,
        })
        .collect())
}

pub async fn resolve_and_validate_registration_scope(
    pool: &PgPool,
    email: &str,
    org_id: Option<Uuid>,
    org_slug: Option<&str>,
    department_id: Option<Uuid>,
    department_slug: Option<&str>,
) -> Result<RegistrationScope, AppError> {
    let domain = email_domain(email)?;
    let org_slug = if org_id.is_some() {
        None
    } else {
        Some(required_scope_value(org_slug, "Organization is required")?)
    };
    let department_slug = if department_id.is_some() {
        None
    } else {
        Some(required_scope_value(
            department_slug,
            "Department is required",
        )?)
    };

    let row: (Option<Uuid>, Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT o.id, d.id, od.org_id \
         FROM (SELECT 1) input \
         LEFT JOIN organizations o \
           ON (($1::uuid IS NOT NULL AND o.id = $1) \
            OR ($1::uuid IS NULL AND o.slug = $2)) \
         LEFT JOIN organization_domains od \
           ON od.org_id = o.id \
          AND od.domain = $3 \
          AND od.verified = true \
         LEFT JOIN departments d \
           ON d.org_id = o.id \
          AND (($4::uuid IS NOT NULL AND d.id = $4) \
            OR ($4::uuid IS NULL AND d.slug = $5))",
    )
    .bind(org_id)
    .bind(org_slug.as_deref())
    .bind(&domain)
    .bind(department_id)
    .bind(department_slug.as_deref())
    .fetch_one(pool)
    .await
    .map_err(AppError::Database)?;

    let (resolved_org_id, resolved_department_id, verified_domain_org_id) = row;

    if resolved_org_id.is_none() {
        if let Some(org_slug) = org_slug.as_deref() {
            return Err(AppError::BadRequest(format!(
                "Organization '{}' is not configured for registration",
                org_slug
            )));
        }
    }

    if resolved_department_id.is_none() {
        if let Some(department_slug) = department_slug.as_deref() {
            return Err(AppError::BadRequest(format!(
                "Department '{}' is not configured for registration",
                department_slug
            )));
        }
    }

    let Some(org_id) = resolved_org_id else {
        return Err(AppError::Forbidden(
            "Email domain is not allowed for this organization".into(),
        ));
    };

    if verified_domain_org_id.is_none() {
        return Err(AppError::Forbidden(
            "Email domain is not allowed for this organization".into(),
        ));
    }

    let Some(department_id) = resolved_department_id else {
        return Err(AppError::BadRequest(
            "Department does not belong to the selected organization".into(),
        ));
    };

    Ok(RegistrationScope {
        org_id,
        department_id,
    })
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

fn required_scope_value(value: Option<&str>, message: &str) -> Result<String, AppError> {
    let value = value.map(str::trim).unwrap_or_default();
    if value.is_empty() {
        Err(AppError::BadRequest(message.into()))
    } else {
        Ok(value.to_string())
    }
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
