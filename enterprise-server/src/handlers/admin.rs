use axum::extract::{Path, Query, State};
use axum::response::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::jwt;
use crate::auth::middleware::{AdminGuard, AuthExtractor};
use crate::error::AppError;
use crate::models::auth::CreateApiKeyRequest;
use crate::routes::AppState;

// ================================================================
// User management
// ================================================================

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub email: String,
    pub name: String,
    pub org_id: Uuid,
    pub department_id: Uuid,
    #[serde(default)]
    pub generate_nonce: bool,
}

/// POST /api/admin/users — Create a new user
pub async fn create_user(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<Value>, AppError> {
    let email = req.email.trim();
    let name = req.name.trim();
    if email.is_empty() {
        return Err(AppError::BadRequest("Email is required".into()));
    }
    if name.is_empty() {
        return Err(AppError::BadRequest("Name is required".into()));
    }
    crate::services::registration::validate_department(
        &state.db,
        req.org_id,
        req.department_id,
    )
    .await?;

    let user_id = Uuid::new_v4();
    let mut tx = state.db.begin().await.map_err(AppError::Database)?;

    // Create user
    sqlx::query("INSERT INTO users (id, email, name, default_org_id) VALUES ($1, $2, $3, $4)")
        .bind(user_id)
        .bind(email)
        .bind(name)
        .bind(req.org_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::Database(e))?;

    // Add user to the selected organization and department.
    sqlx::query(
        "INSERT INTO org_members (user_id, org_id, department_id, role) \
         VALUES ($1, $2, $3, 'member')",
    )
        .bind(user_id)
        .bind(req.org_id)
        .bind(req.department_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::Database(e))?;

    // Generate install nonce if requested
    let install_nonce = if req.generate_nonce {
        let nonce = {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let bytes: [u8; 16] = rng.gen();
            hex::encode(bytes)
        };

        sqlx::query("INSERT INTO install_nonces (nonce, user_id) VALUES ($1, $2)")
            .bind(&nonce)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::Database(e))?;

        Some(nonce)
    } else {
        None
    };

    tx.commit().await.map_err(AppError::Database)?;

    // Audit log
    crate::services::audit::log_action(
        &state.db,
        Some(_auth.0.user_id),
        Some(req.org_id),
        "user.create",
        Some("user"),
        Some(&user_id.to_string()),
        Some(serde_json::json!({
            "email": email,
            "name": name,
            "org_id": req.org_id,
            "department_id": req.department_id,
        })),
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(json!({
        "id": user_id.to_string(),
        "email": email,
        "name": name,
        "personal_org_id": null,
        "default_org_id": req.org_id.to_string(),
        "department_id": req.department_id.to_string(),
        "install_nonce": install_nonce,
    })))
}

/// GET /api/admin/users/{id} — Get user details
pub async fn get_user(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(user_id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let row: Option<(
        Uuid,
        String,
        String,
        Option<Uuid>,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT id, email, name, personal_org_id, created_at, updated_at FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let (id, email, name, personal_org_id, created_at, updated_at) = match row {
        Some(r) => r,
        None => return Err(AppError::NotFound("User not found".into())),
    };

    // Get org memberships
    let org_rows: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT org_id, role FROM org_members WHERE user_id = $1")
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let orgs: Vec<Value> = org_rows
        .iter()
        .map(|(org_id, role)| json!({ "org_id": org_id.to_string(), "role": role }))
        .collect();

    Ok(Json(json!({
        "id": id.to_string(),
        "email": email,
        "name": name,
        "personal_org_id": personal_org_id,
        "orgs": orgs,
        "created_at": created_at,
        "updated_at": updated_at,
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub name: Option<String>,
    pub email: Option<String>,
}

/// PUT /api/admin/users/{id} — Update user
pub async fn update_user(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(user_id): Path<Uuid>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<Json<Value>, AppError> {
    if let Some(name) = &req.name {
        sqlx::query("UPDATE users SET name = $1, updated_at = now() WHERE id = $2")
            .bind(name)
            .bind(user_id)
            .execute(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?;
    }

    if let Some(email) = &req.email {
        sqlx::query("UPDATE users SET email = $1, updated_at = now() WHERE id = $2")
            .bind(email)
            .bind(user_id)
            .execute(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?;
    }

    Ok(Json(json!({ "success": true })))
}

/// DELETE /api/admin/users/{id} — Delete user
pub async fn delete_user(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(user_id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let result = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("User not found".into()));
    }

    crate::services::audit::log_action(
        &state.db,
        Some(_auth.0.user_id),
        None,
        "user.delete",
        Some("user"),
        Some(&user_id.to_string()),
        None,
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(json!({ "success": true })))
}

/// GET /api/admin/users/list — List all users
pub async fn list_users(
    State(state): State<AppState>,
    _auth: AdminGuard,
) -> Result<Json<Value>, AppError> {
    let rows: Vec<(
        Uuid,
        String,
        String,
        Option<Uuid>,
        chrono::DateTime<chrono::Utc>,
        Value,
    )> = sqlx::query_as(
        r#"SELECT
            u.id,
            u.email,
            u.name,
            u.personal_org_id,
            u.created_at,
            COALESCE(
                jsonb_agg(
                    jsonb_build_object(
                        'id', ak.id,
                        'key_prefix', ak.key_prefix,
                        'name', ak.name,
                        'created_at', ak.created_at,
                        'expires_at', ak.expires_at,
                        'last_used_at', ak.last_used_at
                    )
                    ORDER BY ak.created_at DESC
                ) FILTER (WHERE ak.id IS NOT NULL),
                '[]'::jsonb
            ) AS api_keys
        FROM users u
        LEFT JOIN api_keys ak ON ak.user_id = u.id AND ak.revoked_at IS NULL
        GROUP BY u.id, u.email, u.name, u.personal_org_id, u.created_at
        ORDER BY u.created_at DESC"#,
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let users: Vec<Value> = rows
        .iter()
        .map(|(id, email, name, personal_org_id, created_at, api_keys)| {
        json!({
            "id": id.to_string(),
            "email": email,
            "name": name,
            "personal_org_id": personal_org_id.map(|u| u.to_string()),
            "created_at": created_at,
            "api_keys": api_keys,
        })
        })
        .collect();

    Ok(Json(json!({ "users": users })))
}

// ================================================================
// Organization management
// ================================================================

#[derive(Debug, Deserialize)]
pub struct CreateOrganizationRequest {
    pub name: String,
    pub slug: String,
}

/// POST /api/admin/organizations — Create an organization
pub async fn create_organization(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Json(req): Json<CreateOrganizationRequest>,
) -> Result<Json<Value>, AppError> {
    let org_id = Uuid::new_v4();

    sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)")
        .bind(org_id)
        .bind(&req.name)
        .bind(&req.slug)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

    crate::services::audit::log_action(
        &state.db,
        Some(_auth.0.user_id),
        Some(org_id),
        "organization.create",
        Some("organization"),
        Some(&org_id.to_string()),
        Some(json!({"name": req.name, "slug": req.slug})),
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(json!({
        "id": org_id.to_string(),
        "name": req.name,
        "slug": req.slug,
    })))
}

/// GET /api/admin/organizations/{id} — Get organization details
pub async fn get_organization(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(org_id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let row: Option<(Uuid, String, String, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("SELECT id, name, slug, created_at FROM organizations WHERE id = $1")
    .bind(org_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let (id, name, slug, created_at) = match row {
        Some(r) => r,
        None => return Err(AppError::NotFound("Organization not found".into())),
    };

    // Get member count
    let member_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM org_members WHERE org_id = $1")
    .bind(org_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    Ok(Json(json!({
        "id": id.to_string(),
        "name": name,
        "slug": slug,
        "member_count": member_count.0,
        "created_at": created_at,
    })))
}

/// DELETE /api/admin/organizations/{id} — Delete organization
pub async fn delete_organization(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(org_id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let result = sqlx::query("DELETE FROM organizations WHERE id = $1")
        .bind(org_id)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Organization not found".into()));
    }

    crate::services::audit::log_action(
        &state.db,
        Some(_auth.0.user_id),
        Some(org_id),
        "organization.delete",
        Some("organization"),
        Some(&org_id.to_string()),
        None,
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(json!({ "success": true })))
}

/// GET /api/admin/organizations/list — List organizations.
/// Personal organizations are hidden by default; pass include_personal=true to include them.
#[derive(Debug, Deserialize)]
pub struct ListOrganizationsQuery {
    pub include_personal: Option<bool>,
}

pub async fn list_organizations(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Query(query): Query<ListOrganizationsQuery>,
) -> Result<Json<Value>, AppError> {
    let include_personal = query.include_personal.unwrap_or(false);
    let rows: Vec<(Uuid, String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT o.id, o.name, o.slug, o.created_at \
         FROM organizations o \
         WHERE ( \
             $1::bool = true \
             OR ( \
                 o.slug NOT LIKE 'personal-%' \
                 AND NOT EXISTS ( \
                     SELECT 1 \
                     FROM users u \
                     WHERE u.personal_org_id = o.id \
                       AND NOT EXISTS ( \
                           SELECT 1 \
                           FROM organization_domains od \
                           WHERE od.org_id = o.id \
                       ) \
                 ) \
             ) \
         ) \
         ORDER BY o.name",
    )
    .bind(include_personal)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let orgs: Vec<Value> = rows
        .iter()
        .map(|(id, name, slug, created_at)| {
        json!({
            "id": id.to_string(),
            "name": name,
            "slug": slug,
            "created_at": created_at,
        })
        })
        .collect();

    Ok(Json(json!({ "organizations": orgs })))
}

// ================================================================
// Department management
// ================================================================

#[derive(Debug, Deserialize)]
pub struct ListDepartmentsQuery {
    pub org_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDepartmentRequest {
    pub org_id: Uuid,
    pub name: String,
}

/// GET /api/admin/departments — List departments
pub async fn list_departments(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Query(query): Query<ListDepartmentsQuery>,
) -> Result<Json<Value>, AppError> {
    let rows: Vec<(
        Uuid,
        Uuid,
        String,
        String,
        chrono::DateTime<chrono::Utc>,
        String,
        String,
        i64,
    )> = sqlx::query_as(
        r#"SELECT
            d.id,
            d.org_id,
            d.name,
            d.slug,
            d.created_at,
            o.name AS org_name,
            o.slug AS org_slug,
            COUNT(om.user_id)::bigint AS member_count
        FROM departments d
        JOIN organizations o ON o.id = d.org_id
        LEFT JOIN org_members om ON om.org_id = d.org_id AND om.department_id = d.id
        WHERE ($1::uuid IS NULL OR d.org_id = $1)
        GROUP BY d.id, d.org_id, d.name, d.slug, d.created_at, o.name, o.slug
        ORDER BY o.name, d.name"#,
    )
    .bind(query.org_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let departments: Vec<Value> = rows
        .iter()
        .map(
            |(id, org_id, name, slug, created_at, org_name, org_slug, member_count)| {
                json!({
                    "id": id.to_string(),
                    "org_id": org_id.to_string(),
                    "name": name,
                    "slug": slug,
                    "org_name": org_name,
                    "org_slug": org_slug,
                    "member_count": member_count,
                    "created_at": created_at,
                })
            },
        )
        .collect();

    Ok(Json(json!({ "departments": departments })))
}

/// POST /api/admin/departments — Create a department
pub async fn create_department(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Json(req): Json<CreateDepartmentRequest>,
) -> Result<Json<Value>, AppError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("Department name is required".into()));
    }

    let dept_id = Uuid::new_v4();
    let slug = generate_department_slug(name, dept_id);

    sqlx::query("INSERT INTO departments (id, org_id, name, slug) VALUES ($1, $2, $3, $4)")
        .bind(dept_id)
        .bind(req.org_id)
        .bind(name)
        .bind(&slug)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

    crate::services::audit::log_action(
        &state.db,
        Some(_auth.0.user_id),
        Some(req.org_id),
        "department.create",
        Some("department"),
        Some(&dept_id.to_string()),
        Some(json!({"name": name, "slug": slug})),
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(json!({
        "id": dept_id.to_string(),
        "org_id": req.org_id.to_string(),
        "name": name,
        "slug": slug,
    })))
}

fn generate_department_slug(name: &str, dept_id: Uuid) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;

    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            previous_dash = false;
        } else if !slug.is_empty() && !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    let id = dept_id.to_string();
    let suffix = &id[..8];
    if slug.is_empty() {
        format!("dept-{}", suffix)
    } else {
        format!("{}-{}", slug, suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_uuid() -> Uuid {
        Uuid::parse_str("12345678-1234-5678-1234-567812345678").unwrap()
    }

    #[test]
    fn generate_department_slug_keeps_ascii_name_readable() {
        assert_eq!(
            generate_department_slug("R&D Center", fixed_uuid()),
            "r-d-center-12345678"
        );
    }

    #[test]
    fn generate_department_slug_falls_back_for_non_ascii_name() {
        assert_eq!(
            generate_department_slug("技术中心", fixed_uuid()),
            "dept-12345678"
        );
    }
}

/// DELETE /api/admin/departments/{id} — Delete a department
pub async fn delete_department(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(dept_id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let result = sqlx::query("DELETE FROM departments WHERE id = $1")
        .bind(dept_id)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Department not found".into()));
    }

    crate::services::audit::log_action(
        &state.db,
        Some(_auth.0.user_id),
        None,
        "department.delete",
        Some("department"),
        Some(&dept_id.to_string()),
        None,
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(json!({ "success": true })))
}

// ================================================================
// API Key management (existing)
// ================================================================

/// POST /api/admin/api-keys — Create an API key
pub async fn create_api_key(
    State(state): State<AppState>,
    auth: AdminGuard,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<Json<Value>, AppError> {
    let (key, prefix, hash) = jwt::generate_api_key();
    let key_id = Uuid::new_v4();
    let scopes = req.scopes.unwrap_or_else(|| {
        vec![
            "metrics:write".into(),
            "cas:write".into(),
            "cas:read".into(),
            "reports:write".into(),
        ]
    });
    // Use specified user_id if provided (admin creating key for another user), otherwise use authenticated user's id
    let target_user_id = req.user_id.unwrap_or(auth.0.user_id);

    sqlx::query(
        r#"INSERT INTO api_keys (id, user_id, org_id, key_prefix, key_hash, name, scopes, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#
    )
    .bind(key_id)
    .bind(target_user_id)
    .bind(req.org_id)
    .bind(&prefix)
    .bind(&hash)
    .bind(&req.name)
    .bind(&scopes)
    .bind(req.expires_at)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    crate::services::audit::log_action(
        &state.db,
        Some(auth.0.user_id),
        req.org_id,
        "api_key.create",
        Some("api_key"),
        Some(&key_id.to_string()),
        Some(json!({"name": req.name, "target_user_id": target_user_id.to_string()})),
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(json!({
        "id": key_id.to_string(),
        "key": key,
        "key_prefix": prefix,
        "name": req.name,
        "scopes": scopes,
    })))
}

/// GET /api/admin/api-keys — List API keys
pub async fn list_api_keys(
    State(state): State<AppState>,
    auth: AdminGuard,
) -> Result<Json<Value>, AppError> {
    // Admin can see all keys, no user_id filter
    let rows: Vec<(
        Uuid,
        String,
        Option<String>,
        Vec<String>,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        r#"SELECT id, key_prefix, name, scopes, created_at, expires_at, last_used_at
        FROM api_keys WHERE revoked_at IS NULL
        ORDER BY created_at DESC"#,
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let result: Vec<Value> = rows
        .iter()
        .map(|(id, prefix, name, scopes, created, expires, last_used)| {
        json!({
            "id": id.to_string(),
            "key_prefix": prefix,
            "name": name,
            "scopes": scopes,
            "created_at": created,
            "expires_at": expires,
            "last_used_at": last_used,
        })
        })
        .collect();

    Ok(Json(json!({ "api_keys": result })))
}

/// DELETE /api/admin/api-keys/{id} — Revoke an API key
pub async fn revoke_api_key(
    State(state): State<AppState>,
    auth: AdminGuard,
    Path(key_id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    // Admin can revoke any key, no user_id filter
    let result =
        sqlx::query("UPDATE api_keys SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL")
    .bind(key_id)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("API key not found".into()));
    }

    crate::services::audit::log_action(
        &state.db,
        Some(auth.0.user_id),
        None,
        "api_key.revoke",
        Some("api_key"),
        Some(&key_id.to_string()),
        None,
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(json!({ "success": true })))
}

// ================================================================
// Install nonce management (existing)
// ================================================================

#[derive(Debug, Deserialize)]
pub struct GenerateNonceRequest {
    pub user_id: Uuid,
}

/// POST /api/admin/install-nonces — Generate install nonce
pub async fn generate_install_nonce(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Json(req): Json<GenerateNonceRequest>,
) -> Result<Json<Value>, AppError> {
    let nonce = {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let bytes: [u8; 16] = rng.gen();
        hex::encode(bytes)
    };

    sqlx::query("INSERT INTO install_nonces (nonce, user_id) VALUES ($1, $2)")
        .bind(&nonce)
        .bind(req.user_id)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

    crate::services::audit::log_action(
        &state.db,
        Some(_auth.0.user_id),
        None,
        "install_nonce.generate",
        Some("install_nonce"),
        Some(&nonce),
        Some(json!({"target_user_id": req.user_id.to_string()})),
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(json!({
        "nonce": nonce,
        "user_id": req.user_id.to_string(),
    })))
}

// ================================================================
// Audit log
// ================================================================

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub user_id: Option<Uuid>,
    pub org_id: Option<Uuid>,
    pub action: Option<String>,
    pub limit: Option<i64>,
}

/// GET /api/v1/audit-log — Query audit log
pub async fn list_audit_log(
    State(state): State<AppState>,
    _auth: AdminGuard,
    axum::extract::Query(query): axum::extract::Query<AuditLogQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(100).min(1000);

    let rows: Vec<(i64, Option<Uuid>, Option<Uuid>, String, Option<String>, Option<String>, Option<serde_json::Value>, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        r#"SELECT id, user_id, org_id, action, resource_type, resource_id, details, ip_address, user_agent, created_at
        FROM audit_log
        WHERE ($1::uuid IS NULL OR user_id = $1)
          AND ($2::uuid IS NULL OR org_id = $2)
          AND ($3::text IS NULL OR action = $3)
        ORDER BY created_at DESC
        LIMIT $4"#
    )
    .bind(query.user_id)
    .bind(query.org_id)
    .bind(&query.action)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let entries: Vec<Value> = rows
        .iter()
        .map(
            |(
                id,
                user_id,
                org_id,
                action,
                resource_type,
                resource_id,
                details,
                ip_address,
                user_agent,
                created_at,
            )| {
        json!({
            "id": id,
            "user_id": user_id.map(|u| u.to_string()),
            "org_id": org_id.map(|u| u.to_string()),
            "action": action,
            "resource_type": resource_type,
            "resource_id": resource_id,
            "details": details,
            "ip_address": ip_address,
            "user_agent": user_agent,
            "created_at": created_at,
        })
            },
        )
        .collect();

    Ok(Json(json!({ "entries": entries, "count": entries.len() })))
}

// ================================================================
// Repository access rules (whitelist / blacklist)
// ================================================================

#[derive(Debug, Deserialize)]
pub struct CreateRepoAccessRuleRequest {
    pub org_id: Uuid,
    pub rule_type: String,   // "whitelist" or "blacklist"
    pub pattern: String,     // Glob pattern, e.g., "github.com/myorg/*"
    pub description: Option<String>,
}

/// POST /api/admin/repo-access-rules — Create a repository access rule
pub async fn create_repo_access_rule(
    State(state): State<AppState>,
    auth: AdminGuard,
    Json(req): Json<CreateRepoAccessRuleRequest>,
) -> Result<Json<Value>, AppError> {
    if req.rule_type != "whitelist" && req.rule_type != "blacklist" {
        return Err(AppError::BadRequest(
            "rule_type must be 'whitelist' or 'blacklist'".into(),
        ));
    }
    if req.pattern.is_empty() {
        return Err(AppError::BadRequest("pattern must not be empty".into()));
    }

    let rule_id = Uuid::new_v4();

    sqlx::query(
        r#"INSERT INTO repo_access_rules (id, org_id, rule_type, pattern, description, created_by)
        VALUES ($1, $2, $3, $4, $5, $6)"#,
    )
    .bind(rule_id)
    .bind(req.org_id)
    .bind(&req.rule_type)
    .bind(&req.pattern)
    .bind(&req.description)
    .bind(auth.0.user_id)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    crate::services::audit::log_action(
        &state.db,
        Some(auth.0.user_id),
        Some(req.org_id),
        "repo_access_rule.create",
        Some("repo_access_rule"),
        Some(&rule_id.to_string()),
        Some(json!({"rule_type": req.rule_type, "pattern": req.pattern})),
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(json!({
        "id": rule_id.to_string(),
        "org_id": req.org_id.to_string(),
        "rule_type": req.rule_type,
        "pattern": req.pattern,
    })))
}

/// GET /api/admin/repo-access-rules — List repository access rules
#[derive(Debug, Deserialize)]
pub struct RepoAccessRulesQuery {
    pub org_id: Option<Uuid>,
}

pub async fn list_repo_access_rules(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Query(query): Query<RepoAccessRulesQuery>,
) -> Result<Json<Value>, AppError> {
    let rows: Vec<(
        Uuid,
        Uuid,
        String,
        String,
        Option<String>,
        Option<Uuid>,
        bool,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        r#"SELECT id, org_id, rule_type, pattern, description, created_by, enabled, created_at
        FROM repo_access_rules
        WHERE ($1::uuid IS NULL OR org_id = $1)
        ORDER BY org_id, rule_type, created_at"#,
    )
    .bind(query.org_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let rules: Vec<Value> = rows
        .iter()
        .map(
            |(id, org_id, rule_type, pattern, desc, created_by, enabled, created_at)| {
        json!({
            "id": id.to_string(),
            "org_id": org_id.to_string(),
            "rule_type": rule_type,
            "pattern": pattern,
            "description": desc,
            "created_by": created_by.map(|u| u.to_string()),
            "enabled": enabled,
            "created_at": created_at,
        })
            },
        )
        .collect();

    Ok(Json(json!({ "rules": rules })))
}

/// DELETE /api/admin/repo-access-rules/{id} — Delete a repository access rule
pub async fn delete_repo_access_rule(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(rule_id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let result = sqlx::query("DELETE FROM repo_access_rules WHERE id = $1")
        .bind(rule_id)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Access rule not found".into()));
    }

    Ok(Json(json!({ "success": true })))
}

// ================================================================
// Feature Flags management
// ================================================================

#[derive(Debug, Deserialize)]
pub struct UpsertFeatureFlagRequest {
    pub key: String,
    pub value: serde_json::Value,  // true/false or { "debug": bool, "release": bool }
    pub description: Option<String>,
}

/// POST /api/admin/feature-flags — Create or update a feature flag
pub async fn upsert_feature_flag(
    State(state): State<AppState>,
    auth: AdminGuard,
    Json(req): Json<UpsertFeatureFlagRequest>,
) -> Result<Json<Value>, AppError> {
    if req.key.is_empty() {
        return Err(AppError::BadRequest("key must not be empty".into()));
    }

    sqlx::query(
        r#"INSERT INTO feature_flags (key, value, description)
        VALUES ($1, $2, $3)
        ON CONFLICT (key) DO UPDATE SET
            value = EXCLUDED.value,
            description = COALESCE(EXCLUDED.description, feature_flags.description),
            updated_at = now()"#,
    )
    .bind(&req.key)
    .bind(&req.value)
    .bind(&req.description)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    crate::services::audit::log_action(
        &state.db,
        Some(auth.0.user_id),
        None,
        "feature_flag.upsert",
        Some("feature_flag"),
        Some(&req.key),
        Some(json!({"value": req.value})),
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(json!({
        "success": true,
        "key": req.key,
    })))
}

/// GET /api/admin/feature-flags — List all feature flags
pub async fn list_feature_flags(
    State(state): State<AppState>,
    _auth: AdminGuard,
) -> Result<Json<Value>, AppError> {
    let rows: Vec<(
        String,
        serde_json::Value,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
    )> = sqlx::query_as(
        "SELECT key, value, description, updated_at FROM feature_flags ORDER BY key",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let flags: Vec<Value> = rows
        .iter()
        .map(|(key, value, desc, updated_at)| {
        json!({
            "key": key,
            "value": value,
            "description": desc,
            "updated_at": updated_at,
        })
        })
        .collect();

    Ok(Json(json!({ "flags": flags })))
}

/// DELETE /api/admin/feature-flags/{key} — Delete a feature flag
pub async fn delete_feature_flag(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(key): Path<String>,
) -> Result<Json<Value>, AppError> {
    let result = sqlx::query("DELETE FROM feature_flags WHERE key = $1")
        .bind(&key)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Feature flag not found".into()));
    }

    Ok(Json(json!({ "success": true })))
}

// ================================================================
// Data export
// ================================================================

#[derive(Debug, Deserialize)]
pub struct ExportRequest {
    pub export_type: String,   // "csv" or "json"
    pub query_type: String,    // "summary", "developers", "projects", "organizations", "tools"
    pub org_id: Option<Uuid>,
}

/// POST /api/admin/export — Create a data export job
pub async fn create_export(
    State(state): State<AppState>,
    auth: AdminGuard,
    Json(req): Json<ExportRequest>,
) -> Result<Json<Value>, AppError> {
    if req.export_type != "csv" && req.export_type != "json" {
        return Err(AppError::BadRequest(
            "export_type must be 'csv' or 'json'".into(),
        ));
    }
    let valid_query_types = [
        "summary",
        "developers",
        "projects",
        "organizations",
        "tools",
    ];
    if !valid_query_types.contains(&req.query_type.as_str()) {
        return Err(AppError::BadRequest(format!(
            "query_type must be one of: {}",
            valid_query_types.join(", ")
        )));
    }

    let export_id = Uuid::new_v4();

    // For JSON export, generate data synchronously
    if req.export_type == "json" {
        let data = generate_json_export(&state, &req.query_type, req.org_id).await?;
        let path = format!("exports/{}/{}.json", auth.0.user_id, export_id);

        sqlx::query(
            r#"INSERT INTO export_jobs (id, user_id, org_id, export_type, query_type, status, file_path)
            VALUES ($1, $2, $3, $4, $5, $6, $7)"#
        )
        .bind(export_id)
        .bind(auth.0.user_id)
        .bind(req.org_id)
        .bind(&req.export_type)
        .bind(&req.query_type)
        .bind("completed")
        .bind(&path)
        .execute(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

        return Ok(Json(json!({
            "id": export_id.to_string(),
            "status": "completed",
            "data": data,
        })));
    }

    // CSV export: create a pending job
    sqlx::query(
        r#"INSERT INTO export_jobs (id, user_id, org_id, export_type, query_type, status)
        VALUES ($1, $2, $3, $4, $5, 'pending')"#,
    )
    .bind(export_id)
    .bind(auth.0.user_id)
    .bind(req.org_id)
    .bind(&req.export_type)
    .bind(&req.query_type)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    Ok(Json(json!({
        "id": export_id.to_string(),
        "status": "pending",
        "export_type": req.export_type,
        "query_type": req.query_type,
    })))
}

async fn generate_json_export(
    state: &AppState,
    query_type: &str,
    _org_id: Option<Uuid>,
) -> Result<Value, AppError> {
    match query_type {
        "summary" => {
            let row: (Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(
                r#"SELECT
                    COALESCE(SUM(ai_additions), 0),
                    COALESCE(SUM(human_additions), 0),
                    COUNT(*)
                FROM metrics_events WHERE event_type = 1"#,
            )
            .fetch_one(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?;

            Ok(json!({
                "ai_lines": row.0.unwrap_or(0),
                "human_lines": row.1.unwrap_or(0),
                "total_commits": row.2.unwrap_or(0),
            }))
        }
        "developers" => {
            let rows: Vec<(String, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
                r#"SELECT author_email, COUNT(*), COALESCE(SUM(ai_additions), 0), COALESCE(SUM(human_additions), 0)
                FROM metrics_events WHERE event_type = 1 AND author_email IS NOT NULL
                GROUP BY author_email ORDER BY COUNT(*) DESC"#
            )
            .fetch_all(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?;

            Ok(json!(rows.iter().map(|(email, commits, ai, human)| {
                json!({"email": email, "commits": commits.unwrap_or(0), "ai_lines": ai.unwrap_or(0), "human_lines": human.unwrap_or(0)})
            }).collect::<Vec<_>>()))
        }
        "projects" => {
            let rows: Vec<(String, Option<String>, Option<i64>, Option<i64>)> = sqlx::query_as(
                r#"SELECT repo_url, NULL::text AS branch,
                    COALESCE(SUM(ai_additions), 0),
                    COALESCE(SUM(GREATEST(COALESCE(git_diff_added_lines, 0) - COALESCE(ai_additions, 0), 0)), 0)
                FROM metrics_events
                WHERE event_type = 1 AND repo_url IS NOT NULL AND repo_url != ''
                GROUP BY repo_url
                ORDER BY repo_url"#
            )
            .fetch_all(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?;

            Ok(json!(rows.iter().map(|(url, branch, ai, human)| {
                json!({"repo_url": url, "branch": branch, "ai_lines": ai.unwrap_or(0), "human_lines": human.unwrap_or(0)})
            }).collect::<Vec<_>>()))
        }
        "organizations" => {
            let rows: Vec<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
                r#"SELECT o.name, COALESCE(SUM(m.ai_additions), 0), COALESCE(SUM(m.human_additions), 0)
                FROM organizations o
                LEFT JOIN metrics_events m ON m.org_id = o.id AND m.event_type = 1
                GROUP BY o.id, o.name ORDER BY o.name"#
            )
            .fetch_all(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?;

            Ok(json!(rows.iter().map(|(name, ai, human)| {
                json!({"organization": name, "ai_lines": ai.unwrap_or(0), "human_lines": human.unwrap_or(0)})
            }).collect::<Vec<_>>()))
        }
        "tools" => {
            let rows: Vec<(String, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
                r#"SELECT tool_model, COALESCE(SUM(ai_additions), 0), COALESCE(SUM(mixed_additions), 0), COALESCE(SUM(ai_accepted), 0)
                FROM tool_model_stats GROUP BY tool_model ORDER BY SUM(ai_additions) DESC"#
            )
            .fetch_all(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?;

            Ok(json!(rows.iter().map(|(tool_model, ai, mixed, accepted)| {
                json!({"tool_model": tool_model, "ai_additions": ai.unwrap_or(0), "mixed_additions": mixed.unwrap_or(0), "ai_accepted": accepted.unwrap_or(0)})
            }).collect::<Vec<_>>()))
        }
        _ => Ok(json!({})),
    }
}

/// GET /api/admin/export/{id} — Get export job status and download
pub async fn get_export(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(export_id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let row: Option<(Uuid, String, String, String, String, Option<String>, chrono::DateTime<chrono::Utc>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        r#"SELECT id, export_type, query_type, status, COALESCE(file_path, ''), error_message, created_at, completed_at
        FROM export_jobs WHERE id = $1"#
    )
    .bind(export_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let (id, export_type, query_type, status, file_path, error, created_at, completed_at) =
        match row {
        Some(r) => r,
        None => return Err(AppError::NotFound("Export job not found".into())),
    };

    Ok(Json(json!({
        "id": id.to_string(),
        "export_type": export_type,
        "query_type": query_type,
        "status": status,
        "file_path": if file_path.is_empty() { None } else { Some(file_path) },
        "error": error,
        "created_at": created_at,
        "completed_at": completed_at,
    })))
}

// ================================================================
// Data retention policies (Phase 6)
// ================================================================

#[derive(Debug, Deserialize)]
pub struct UpsertRetentionPolicyRequest {
    pub org_id: Uuid,
    pub metrics_retention_days: Option<i32>,
    pub cas_retention_days: Option<i32>,
    pub audit_retention_days: Option<i32>,
    pub ci_events_retention_days: Option<i32>,
    pub alerts_retention_days: Option<i32>,
    pub auto_purge: Option<bool>,
}

/// PUT /api/admin/retention-policies — Create or update a retention policy
pub async fn upsert_retention_policy(
    State(state): State<AppState>,
    auth: AdminGuard,
    Json(req): Json<UpsertRetentionPolicyRequest>,
) -> Result<Json<Value>, AppError> {
    let policy = crate::services::data_retention::upsert_retention_policy(
        &state.db,
        req.org_id,
        req.metrics_retention_days,
        req.cas_retention_days,
        req.audit_retention_days,
        req.ci_events_retention_days,
        req.alerts_retention_days,
        req.auto_purge,
    )
    .await?;

    crate::services::audit::log_action(
        &state.db,
        Some(auth.0.user_id),
        Some(req.org_id),
        "retention_policy.upsert",
        Some("retention_policy"),
        Some(&req.org_id.to_string()),
        Some(serde_json::json!({
            "metrics_retention_days": req.metrics_retention_days,
            "cas_retention_days": req.cas_retention_days,
            "auto_purge": req.auto_purge,
        })),
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(policy))
}

#[derive(Debug, Deserialize)]
pub struct RetentionPolicyQuery {
    pub org_id: Uuid,
}

/// GET /api/admin/retention-policies — Get retention policy for an organization
pub async fn get_retention_policy(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Query(query): Query<RetentionPolicyQuery>,
) -> Result<Json<Value>, AppError> {
    let policy =
        crate::services::data_retention::get_retention_policy(&state.db, query.org_id).await?;

    Ok(Json(policy))
}

/// POST /api/admin/purge-expired-data — Trigger data purge for all orgs with auto_purge
pub async fn purge_expired_data(
    State(state): State<AppState>,
    auth: AdminGuard,
) -> Result<Json<Value>, AppError> {
    let result = crate::services::data_retention::purge_expired_data(&state.db).await?;

    crate::services::audit::log_action(
        &state.db,
        Some(auth.0.user_id),
        None,
        "data_purge.execute",
        Some("system"),
        None,
        Some(result.clone()),
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(result))
}

// ================================================================
// CAS access log query (Phase 6)
// ================================================================

#[derive(Debug, Deserialize)]
pub struct CasAccessLogQuery {
    pub cas_hash: Option<String>,
    pub user_id: Option<Uuid>,
    pub org_id: Option<Uuid>,
    pub limit: Option<i64>,
}

/// GET /api/admin/cas-access-log — Query CAS access audit log
pub async fn list_cas_access_log(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Query(query): Query<CasAccessLogQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(100).min(1000);

    let rows: Vec<(i64, Option<Uuid>, Option<Uuid>, Option<Uuid>, String, String, Option<String>, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        r#"SELECT id, user_id, org_id, api_key_id, cas_hash, access_method, purpose, ip_address, user_agent, created_at
        FROM cas_access_log
        WHERE ($1::text IS NULL OR cas_hash = $1)
          AND ($2::uuid IS NULL OR user_id = $2)
          AND ($3::uuid IS NULL OR org_id = $3)
        ORDER BY created_at DESC
        LIMIT $4"#
    )
    .bind(&query.cas_hash)
    .bind(query.user_id)
    .bind(query.org_id)
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let entries: Vec<Value> = rows
        .iter()
        .map(
            |(id, user_id, org_id, api_key_id, hash, method, purpose, ip, ua, created)| {
        json!({
            "id": id,
            "user_id": user_id.map(|u| u.to_string()),
            "org_id": org_id.map(|u| u.to_string()),
            "api_key_id": api_key_id.map(|u| u.to_string()),
            "cas_hash": hash,
            "access_method": method,
            "purpose": purpose,
            "ip_address": ip,
            "user_agent": ua,
            "created_at": created,
        })
            },
        )
        .collect();

    Ok(Json(json!({ "entries": entries, "count": entries.len() })))
}

// ================================================================
// User-specific API Keys (for user management page)
// ================================================================

/// GET /api/admin/users/{id}/api-keys — List API keys for a specific user
pub async fn list_user_api_keys(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(user_id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let rows: Vec<(
        Uuid,
        String,
        Option<String>,
        Vec<String>,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        r#"SELECT id, key_prefix, name, scopes, created_at, expires_at, last_used_at
        FROM api_keys WHERE user_id = $1 AND revoked_at IS NULL
        ORDER BY created_at DESC"#,
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let result: Vec<Value> = rows
        .iter()
        .map(|(id, prefix, name, scopes, created, expires, last_used)| {
        json!({
            "id": id.to_string(),
            "key_prefix": prefix,
            "name": name,
            "scopes": scopes,
            "created_at": created,
            "expires_at": expires,
            "last_used_at": last_used,
        })
        })
        .collect();

    Ok(Json(json!({ "api_keys": result })))
}
