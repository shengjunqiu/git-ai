use axum::extract::{Path, Query, State};
use axum::response::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::jwt;
use crate::auth::middleware::AdminGuard;
use crate::error::AppError;
use crate::models::auth::CreateApiKeyRequest;
use crate::pagination::{
    clamp_limit, decode_cursor, decode_time_id_cursor, decode_time_uuid_cursor, encode_cursor,
    fetch_limit, pagination_meta, truncate_to_limit, PaginationQuery, TimeIdCursor, TimeUuidCursor,
    CURSOR_VERSION, DEFAULT_LIMIT, MAX_LIMIT,
};
use crate::routes::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct NameUuidCursor {
    v: u8,
    name: String,
    id: Uuid,
    #[serde(default)]
    include_personal: bool,
    #[serde(default)]
    q: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DepartmentListCursor {
    v: u8,
    org_name: String,
    department_name: String,
    department_id: Uuid,
    #[serde(default)]
    org_id: Option<Uuid>,
    #[serde(default)]
    q: Option<String>,
}

fn decode_optional_time_uuid_cursor(
    cursor: Option<&str>,
) -> Result<Option<TimeUuidCursor>, AppError> {
    cursor.map(decode_time_uuid_cursor).transpose()
}

fn encode_time_uuid_cursor(
    timestamp: chrono::DateTime<chrono::Utc>,
    id: Uuid,
) -> Result<String, AppError> {
    encode_cursor(&TimeUuidCursor::new(timestamp, id))
}

fn decode_name_uuid_cursor(cursor: Option<&str>) -> Result<Option<NameUuidCursor>, AppError> {
    let cursor: Option<NameUuidCursor> = cursor.map(decode_cursor).transpose()?;
    if let Some(cursor) = &cursor {
        validate_admin_cursor_version(cursor.v)?;
    }
    Ok(cursor)
}

fn encode_name_uuid_cursor(
    name: &str,
    id: Uuid,
    include_personal: bool,
    q: Option<&str>,
) -> Result<String, AppError> {
    encode_cursor(&NameUuidCursor {
        v: CURSOR_VERSION,
        name: name.to_string(),
        id,
        include_personal,
        q: q.map(str::to_string),
    })
}

fn decode_department_list_cursor(
    cursor: Option<&str>,
) -> Result<Option<DepartmentListCursor>, AppError> {
    let cursor: Option<DepartmentListCursor> = cursor.map(decode_cursor).transpose()?;
    if let Some(cursor) = &cursor {
        validate_admin_cursor_version(cursor.v)?;
    }
    Ok(cursor)
}

fn encode_department_list_cursor(
    org_name: &str,
    department_name: &str,
    department_id: Uuid,
    org_id: Option<Uuid>,
    q: Option<&str>,
) -> Result<String, AppError> {
    encode_cursor(&DepartmentListCursor {
        v: CURSOR_VERSION,
        org_name: org_name.to_string(),
        department_name: department_name.to_string(),
        department_id,
        org_id,
        q: q.map(str::to_string),
    })
}

fn normalize_option_query(query: Option<String>) -> Option<String> {
    query
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_admin_cursor_version(version: u8) -> Result<(), AppError> {
    if version == CURSOR_VERSION {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "Unsupported pagination cursor version: {}",
            version
        )))
    }
}

fn admin_org_id(auth: &AdminGuard) -> Result<Uuid, AppError> {
    auth.0.org_id.ok_or_else(|| {
        AppError::Forbidden("An organization-scoped administrator is required".into())
    })
}

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
    crate::services::registration::validate_department(&state.db, req.org_id, req.department_id)
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
        "git_tracking_upload_enabled": false,
        "install_nonce": install_nonce,
    })))
}

/// GET /api/admin/users/{id} — Get user details
pub async fn get_user(
    State(state): State<AppState>,
    auth: AdminGuard,
    Path(user_id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let org_id = admin_org_id(&auth)?;
    let row: Option<(
        Uuid,
        String,
        String,
        Option<Uuid>,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
        bool,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<Uuid>,
    )> = sqlx::query_as(
        "SELECT u.id, u.email, u.name, u.personal_org_id, u.created_at, u.updated_at, \
                om.git_tracking_upload_enabled, om.git_tracking_upload_authorized_at, \
                om.git_tracking_upload_authorized_by \
         FROM users u \
         JOIN org_members om ON om.user_id = u.id \
         WHERE u.id = $1 AND om.org_id = $2",
    )
    .bind(user_id)
    .bind(org_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let (
        id,
        email,
        name,
        personal_org_id,
        created_at,
        updated_at,
        git_tracking_upload_enabled,
        git_tracking_upload_authorized_at,
        git_tracking_upload_authorized_by,
    ) = match row {
        Some(r) => r,
        None => {
            return Err(AppError::NotFound(
                "User not found in the administrator's organization".into(),
            ))
        }
    };

    // Get org memberships
    let org_rows: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT org_id, role FROM org_members WHERE user_id = $1 AND org_id = $2")
            .bind(user_id)
            .bind(org_id)
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
        "git_tracking_upload_enabled": git_tracking_upload_enabled,
        "git_tracking_upload_authorized_at": git_tracking_upload_authorized_at,
        "git_tracking_upload_authorized_by": git_tracking_upload_authorized_by,
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

#[derive(Debug, Deserialize)]
pub struct GitTrackingUploadAuthorizationRequest {
    pub authorized: bool,
}

const MAX_BULK_GIT_TRACKING_UPLOAD_USERS: usize = 100;

#[derive(Debug, Deserialize)]
pub struct BulkGitTrackingUploadAuthorizationRequest {
    pub user_ids: Vec<Uuid>,
}

/// PUT /api/admin/users/{id}/git-tracking-upload — Grant or revoke a
/// developer's permission to upload Git tracking data in the admin's org.
pub async fn update_git_tracking_upload_authorization(
    State(state): State<AppState>,
    auth: AdminGuard,
    Path(user_id): Path<Uuid>,
    Json(req): Json<GitTrackingUploadAuthorizationRequest>,
) -> Result<Json<Value>, AppError> {
    let org_id = admin_org_id(&auth)?;
    let mut tx = state.db.begin().await.map_err(AppError::Database)?;

    let updated: Option<(bool, Option<chrono::DateTime<chrono::Utc>>, Option<Uuid>)> =
        sqlx::query_as(
            r#"UPDATE org_members
           SET git_tracking_upload_enabled = $1,
               git_tracking_upload_authorized_at = CASE WHEN $1 THEN now() ELSE NULL END,
               git_tracking_upload_authorized_by = CASE WHEN $1 THEN $2 ELSE NULL END
           WHERE user_id = $3 AND org_id = $4
           RETURNING git_tracking_upload_enabled,
                     git_tracking_upload_authorized_at,
                     git_tracking_upload_authorized_by"#,
        )
        .bind(req.authorized)
        .bind(auth.0.user_id)
        .bind(user_id)
        .bind(org_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(AppError::Database)?;

    let Some((enabled, authorized_at, authorized_by)) = updated else {
        return Err(AppError::NotFound(
            "User not found in the administrator's organization".into(),
        ));
    };

    let action = if enabled {
        "developer.git_tracking_upload.grant"
    } else {
        "developer.git_tracking_upload.revoke"
    };
    sqlx::query(
        r#"INSERT INTO audit_log
           (user_id, org_id, action, resource_type, resource_id, details)
           VALUES ($1, $2, $3, 'user', $4, $5)"#,
    )
    .bind(auth.0.user_id)
    .bind(org_id)
    .bind(action)
    .bind(user_id.to_string())
    .bind(json!({ "authorized": enabled }))
    .execute(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    tx.commit().await.map_err(AppError::Database)?;

    Ok(Json(json!({
        "user_id": user_id,
        "org_id": org_id,
        "git_tracking_upload_enabled": enabled,
        "git_tracking_upload_authorized_at": authorized_at,
        "git_tracking_upload_authorized_by": authorized_by,
    })))
}

/// POST /api/admin/users/git-tracking-upload/authorize — Grant Git tracking
/// upload permission to multiple users in the administrator's organization.
pub async fn bulk_authorize_git_tracking_upload(
    State(state): State<AppState>,
    auth: AdminGuard,
    Json(req): Json<BulkGitTrackingUploadAuthorizationRequest>,
) -> Result<Json<Value>, AppError> {
    if req.user_ids.is_empty() {
        return Err(AppError::BadRequest(
            "At least one user must be selected".into(),
        ));
    }
    if req.user_ids.len() > MAX_BULK_GIT_TRACKING_UPLOAD_USERS {
        return Err(AppError::BadRequest(format!(
            "No more than {MAX_BULK_GIT_TRACKING_UPLOAD_USERS} users can be authorized at once"
        )));
    }

    let mut user_ids = req.user_ids;
    user_ids.sort_unstable();
    user_ids.dedup();

    let org_id = admin_org_id(&auth)?;
    let mut tx = state.db.begin().await.map_err(AppError::Database)?;
    let matched_user_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM org_members WHERE org_id = $1 AND user_id = ANY($2::uuid[])",
    )
    .bind(org_id)
    .bind(&user_ids)
    .fetch_all(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    if matched_user_ids.len() != user_ids.len() {
        return Err(AppError::NotFound(
            "One or more users were not found in the administrator's organization".into(),
        ));
    }

    let authorized_user_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"UPDATE org_members
           SET git_tracking_upload_enabled = true,
               git_tracking_upload_authorized_at = now(),
               git_tracking_upload_authorized_by = $1
           WHERE org_id = $2 AND user_id = ANY($3::uuid[])
           RETURNING user_id"#,
    )
    .bind(auth.0.user_id)
    .bind(org_id)
    .bind(&user_ids)
    .fetch_all(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    sqlx::query(
        r#"INSERT INTO audit_log
           (user_id, org_id, action, resource_type, resource_id, details)
           SELECT $1, $2, 'developer.git_tracking_upload.grant', 'user',
                  selected.user_id::text,
                  jsonb_build_object('authorized', true, 'bulk', true)
           FROM UNNEST($3::uuid[]) AS selected(user_id)"#,
    )
    .bind(auth.0.user_id)
    .bind(org_id)
    .bind(&user_ids)
    .execute(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    tx.commit().await.map_err(AppError::Database)?;

    Ok(Json(json!({
        "authorized_count": authorized_user_ids.len(),
        "user_ids": authorized_user_ids,
        "org_id": org_id,
    })))
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
    auth: AdminGuard,
    Query(query): Query<PaginationQuery>,
) -> Result<Json<Value>, AppError> {
    let org_id = admin_org_id(&auth)?;
    let limit = clamp_limit(query.limit, DEFAULT_LIMIT, MAX_LIMIT);
    let cursor = decode_optional_time_uuid_cursor(query.cursor.as_deref())?;
    let cursor_timestamp = cursor.as_ref().map(|cursor| cursor.timestamp.clone());
    let cursor_id = cursor.as_ref().map(|cursor| cursor.id);

    let mut rows: Vec<(
        Uuid,
        String,
        String,
        Option<Uuid>,
        chrono::DateTime<chrono::Utc>,
        bool,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<Uuid>,
        Value,
    )> = sqlx::query_as(
        r#"SELECT
            u.id,
            u.email,
            u.name,
            u.personal_org_id,
            u.created_at,
            om.git_tracking_upload_enabled,
            om.git_tracking_upload_authorized_at,
            om.git_tracking_upload_authorized_by,
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
                    ORDER BY ak.created_at DESC, ak.id DESC
                ) FILTER (WHERE ak.id IS NOT NULL),
                '[]'::jsonb
            ) AS api_keys
        FROM users u
        JOIN org_members om ON om.user_id = u.id AND om.org_id = $1
        LEFT JOIN api_keys ak ON ak.user_id = u.id
            AND ak.revoked_at IS NULL
            AND (ak.org_id IS NULL OR ak.org_id = om.org_id)
        WHERE ($2::timestamptz IS NULL OR (u.created_at, u.id) < ($2::timestamptz, $3::uuid))
        GROUP BY u.id, u.email, u.name, u.personal_org_id, u.created_at,
                 om.git_tracking_upload_enabled, om.git_tracking_upload_authorized_at,
                 om.git_tracking_upload_authorized_by
        ORDER BY u.created_at DESC, u.id DESC
        LIMIT $4"#,
    )
    .bind(org_id)
    .bind(cursor_timestamp)
    .bind(cursor_id)
    .bind(fetch_limit(limit))
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let has_more = truncate_to_limit(&mut rows, limit);
    let next_cursor = if has_more {
        rows.last()
            .map(|(id, _, _, _, created_at, _, _, _, _)| {
                encode_time_uuid_cursor(created_at.clone(), *id)
            })
            .transpose()?
    } else {
        None
    };

    let users: Vec<Value> = rows
        .iter()
        .map(
            |(
                id,
                email,
                name,
                personal_org_id,
                created_at,
                git_tracking_upload_enabled,
                git_tracking_upload_authorized_at,
                git_tracking_upload_authorized_by,
                api_keys,
            )| {
                json!({
                    "id": id.to_string(),
                    "email": email,
                    "name": name,
                    "personal_org_id": personal_org_id.map(|u| u.to_string()),
                    "created_at": created_at,
                    "git_tracking_upload_enabled": git_tracking_upload_enabled,
                    "git_tracking_upload_authorized_at": git_tracking_upload_authorized_at,
                    "git_tracking_upload_authorized_by": git_tracking_upload_authorized_by,
                    "api_keys": api_keys,
                })
            },
        )
        .collect();

    Ok(Json(json!({
        "users": users,
        "pagination": pagination_meta(limit, has_more, next_cursor),
    })))
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
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

pub async fn list_organizations(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Query(query): Query<ListOrganizationsQuery>,
) -> Result<Json<Value>, AppError> {
    let include_personal = query.include_personal.unwrap_or(false);
    let search = normalize_option_query(query.q);
    let limit = clamp_limit(query.limit, DEFAULT_LIMIT, MAX_LIMIT);
    let cursor = decode_name_uuid_cursor(query.cursor.as_deref())?;
    if cursor.as_ref().is_some_and(|cursor| {
        cursor.include_personal != include_personal || cursor.q.as_ref() != search.as_ref()
    }) {
        return Err(AppError::BadRequest(
            "Pagination cursor does not match organization filters".into(),
        ));
    }
    let cursor_name = cursor.as_ref().map(|cursor| cursor.name.clone());
    let cursor_id = cursor.as_ref().map(|cursor| cursor.id);

    let mut rows: Vec<(Uuid, String, String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
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
         AND ( \
             $2::text IS NULL \
             OR POSITION(LOWER($2::text) IN LOWER(o.name)) > 0 \
             OR POSITION(LOWER($2::text) IN LOWER(o.slug)) > 0 \
         ) \
         AND ($3::text IS NULL OR (o.name, o.id) > ($3::text, $4::uuid)) \
         ORDER BY o.name ASC, o.id ASC \
         LIMIT $5",
    )
    .bind(include_personal)
    .bind(search.as_deref())
    .bind(cursor_name)
    .bind(cursor_id)
    .bind(fetch_limit(limit))
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let has_more = truncate_to_limit(&mut rows, limit);
    let next_cursor = if has_more {
        rows.last()
            .map(|(id, name, _, _)| {
                encode_name_uuid_cursor(name, *id, include_personal, search.as_deref())
            })
            .transpose()?
    } else {
        None
    };

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

    Ok(Json(json!({
        "organizations": orgs,
        "pagination": pagination_meta(limit, has_more, next_cursor),
    })))
}

// ================================================================
// Department management
// ================================================================

#[derive(Debug, Deserialize)]
pub struct ListDepartmentsQuery {
    pub org_id: Option<Uuid>,
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDepartmentRequest {
    pub org_id: Uuid,
    pub name: String,
    pub code: Option<String>,
    pub parent_id: Option<Uuid>,
}

/// GET /api/admin/departments — List departments
pub async fn list_departments(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Query(query): Query<ListDepartmentsQuery>,
) -> Result<Json<Value>, AppError> {
    let search = normalize_option_query(query.q);
    let limit = clamp_limit(query.limit, DEFAULT_LIMIT, MAX_LIMIT);
    let cursor = decode_department_list_cursor(query.cursor.as_deref())?;
    if cursor
        .as_ref()
        .is_some_and(|cursor| cursor.org_id != query.org_id || cursor.q.as_ref() != search.as_ref())
    {
        return Err(AppError::BadRequest(
            "Pagination cursor does not match department filters".into(),
        ));
    }
    let cursor_org_name = cursor.as_ref().map(|cursor| cursor.org_name.clone());
    let cursor_department_name = cursor.as_ref().map(|cursor| cursor.department_name.clone());
    let cursor_department_id = cursor.as_ref().map(|cursor| cursor.department_id);

    let mut rows: Vec<(
        Uuid,
        Uuid,
        String,
        String,
        String,
        Option<Uuid>,
        chrono::DateTime<chrono::Utc>,
        String,
        String,
        Option<String>,
        Option<String>,
        i64,
    )> = sqlx::query_as(
        r#"SELECT
            d.id,
            d.org_id,
            d.code,
            d.name,
            d.slug,
            d.parent_id,
            d.created_at,
            o.name AS org_name,
            o.slug AS org_slug,
            parent.code AS parent_code,
            parent.name AS parent_name,
            COUNT(om.user_id)::bigint AS member_count
        FROM departments d
        JOIN organizations o ON o.id = d.org_id
        LEFT JOIN departments parent ON parent.id = d.parent_id AND parent.org_id = d.org_id
        LEFT JOIN org_members om ON om.org_id = d.org_id AND om.department_id = d.id
        WHERE ($1::uuid IS NULL OR d.org_id = $1)
          AND (
              $2::text IS NULL
              OR POSITION(LOWER($2::text) IN LOWER(d.name)) > 0
              OR POSITION(LOWER($2::text) IN LOWER(d.code)) > 0
              OR POSITION(LOWER($2::text) IN LOWER(d.slug)) > 0
          )
          AND ($3::text IS NULL OR (o.name, d.name, d.id) > ($3::text, $4::text, $5::uuid))
        GROUP BY d.id, d.org_id, d.code, d.name, d.slug, d.parent_id, d.created_at,
                 o.name, o.slug, parent.code, parent.name
        ORDER BY o.name ASC, d.name ASC, d.id ASC
        LIMIT $6"#,
    )
    .bind(query.org_id)
    .bind(search.as_deref())
    .bind(cursor_org_name)
    .bind(cursor_department_name)
    .bind(cursor_department_id)
    .bind(fetch_limit(limit))
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let has_more = truncate_to_limit(&mut rows, limit);
    let next_cursor = if has_more {
        rows.last()
            .map(|(id, _, _, name, _, _, _, org_name, _, _, _, _)| {
                encode_department_list_cursor(org_name, name, *id, query.org_id, search.as_deref())
            })
            .transpose()?
    } else {
        None
    };

    let departments: Vec<Value> = rows
        .iter()
        .map(
            |(
                id,
                org_id,
                code,
                name,
                slug,
                parent_id,
                created_at,
                org_name,
                org_slug,
                parent_code,
                parent_name,
                member_count,
            )| {
                json!({
                    "id": id.to_string(),
                    "org_id": org_id.to_string(),
                    "code": code,
                    "name": name,
                    "slug": slug,
                    "parent_id": parent_id.map(|id| id.to_string()),
                    "parent_code": parent_code,
                    "parent_name": parent_name,
                    "org_name": org_name,
                    "org_slug": org_slug,
                    "member_count": member_count,
                    "created_at": created_at,
                })
            },
        )
        .collect();

    Ok(Json(json!({
        "departments": departments,
        "pagination": pagination_meta(limit, has_more, next_cursor),
    })))
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
    let code = match req.code.as_deref().map(str::trim) {
        Some("") => {
            return Err(AppError::BadRequest(
                "Department code cannot be empty".into(),
            ));
        }
        Some(code) => code.to_ascii_uppercase(),
        None => format!("D-{}", dept_id.simple()).to_ascii_uppercase(),
    };

    if let Some(parent_id) = req.parent_id {
        let parent_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM departments WHERE id = $1 AND org_id = $2)",
        )
        .bind(parent_id)
        .bind(req.org_id)
        .fetch_one(&state.db)
        .await
        .map_err(AppError::Database)?;

        if !parent_exists {
            return Err(AppError::BadRequest(
                "Parent department must belong to the same organization".into(),
            ));
        }
    }

    sqlx::query(
        "INSERT INTO departments (id, org_id, code, name, slug, parent_id) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(dept_id)
    .bind(req.org_id)
    .bind(&code)
    .bind(name)
    .bind(&slug)
    .bind(req.parent_id)
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
        Some(json!({
            "code": code,
            "name": name,
            "slug": slug,
            "parent_id": req.parent_id,
        })),
        None,
        None,
    )
    .await
    .ok();

    Ok(Json(json!({
        "id": dept_id.to_string(),
        "org_id": req.org_id.to_string(),
        "code": code,
        "name": name,
        "slug": slug,
        "parent_id": req.parent_id.map(|id| id.to_string()),
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
    use chrono::TimeZone;

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

    #[test]
    fn decode_log_cursor_returns_none_without_cursor() {
        assert!(decode_log_cursor(None).unwrap().is_none());
    }

    #[test]
    fn decode_log_cursor_round_trips_timestamp_and_id() {
        let timestamp = chrono::Utc.with_ymd_and_hms(2026, 7, 9, 10, 0, 0).unwrap();
        let encoded = encode_log_cursor(timestamp, 123).unwrap();

        let decoded = decode_log_cursor(Some(&encoded)).unwrap().unwrap();

        assert_eq!(decoded.timestamp, timestamp);
        assert_eq!(decoded.id, 123);
    }

    #[test]
    fn decode_log_cursor_rejects_invalid_input() {
        let err = decode_log_cursor(Some("bad*cursor")).unwrap_err();

        assert!(matches!(err, AppError::BadRequest(_)));
    }
}

/// DELETE /api/admin/departments/{id} — Delete a department
pub async fn delete_department(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(dept_id): Path<Uuid>,
) -> Result<Json<Value>, AppError> {
    let has_children: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM departments WHERE parent_id = $1)")
            .bind(dept_id)
            .fetch_one(&state.db)
            .await
            .map_err(AppError::Database)?;

    if has_children {
        return Err(AppError::BadRequest(
            "Department with child departments cannot be deleted".into(),
        ));
    }

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
    _auth: AdminGuard,
    Query(query): Query<PaginationQuery>,
) -> Result<Json<Value>, AppError> {
    // Admin can see all keys, no user_id filter
    let limit = clamp_limit(query.limit, DEFAULT_LIMIT, MAX_LIMIT);
    let cursor = decode_optional_time_uuid_cursor(query.cursor.as_deref())?;
    let cursor_timestamp = cursor.as_ref().map(|cursor| cursor.timestamp.clone());
    let cursor_id = cursor.as_ref().map(|cursor| cursor.id);

    let mut rows: Vec<(
        Uuid,
        String,
        Option<String>,
        Vec<String>,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        r#"SELECT id, key_prefix, name, scopes, created_at, expires_at, last_used_at
        FROM api_keys
        WHERE revoked_at IS NULL
          AND ($1::timestamptz IS NULL OR (created_at, id) < ($1::timestamptz, $2::uuid))
        ORDER BY created_at DESC, id DESC
        LIMIT $3"#,
    )
    .bind(cursor_timestamp)
    .bind(cursor_id)
    .bind(fetch_limit(limit))
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let has_more = truncate_to_limit(&mut rows, limit);
    let next_cursor = if has_more {
        rows.last()
            .map(|(id, _, _, _, created, _, _)| encode_time_uuid_cursor(created.clone(), *id))
            .transpose()?
    } else {
        None
    };

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

    Ok(Json(json!({
        "api_keys": result,
        "pagination": pagination_meta(limit, has_more, next_cursor),
    })))
}

#[cfg(test)]
mod log_pagination_tests {
    use super::*;
    use crate::config::{AppConfig, MetricsRollupWriteMode};
    use crate::models::user::{AuthIdentity, AuthMethod};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use sqlx::postgres::PgPoolOptions;
    use sqlx::PgPool;

    struct TestDatabase {
        state: AppState,
        admin_pool: PgPool,
        db_name: String,
    }

    impl TestDatabase {
        async fn new() -> anyhow::Result<Option<Self>> {
            let database_url = test_database_url();
            let db_name = unique_test_database_name();
            let admin_url = database_url_for_database(&database_url, "postgres")?;
            let test_url = database_url_for_database(&database_url, &db_name)?;

            let admin_pool = match PgPoolOptions::new()
                .max_connections(2)
                .connect(&admin_url)
                .await
            {
                Ok(pool) => pool,
                Err(error) => {
                    eprintln!(
                        "skipping admin log pagination test: could not connect to admin database: {error}"
                    );
                    return Ok(None);
                }
            };

            if let Err(error) = create_database(&admin_pool, &db_name).await {
                eprintln!(
                    "skipping admin log pagination test: could not create isolated database {db_name}: {error}"
                );
                admin_pool.close().await;
                return Ok(None);
            }

            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(&test_url)
                .await?;
            crate::db::run_migrations(&pool).await?;

            let config = test_config(&test_url);
            let redis = redis::Client::open(config.redis_url.clone())?;
            let auth_password_limiter = crate::routes::auth_password_limiter(&config);
            let cas_store = crate::services::cas::CasStore::new(&config)?;
            let state = AppState {
                db: pool,
                redis,
                config,
                cas_store,
                rate_limiter: crate::services::rate_limit::RateLimiter::new(),
                auth_password_limiter,
            };

            Ok(Some(Self {
                state,
                admin_pool,
                db_name,
            }))
        }

        async fn cleanup(self) -> anyhow::Result<()> {
            self.state.db.close().await;
            drop_database(&self.admin_pool, &self.db_name).await?;
            self.admin_pool.close().await;
            Ok(())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn audit_log_cursor_paginates_without_repeating_tie_breaker_ids() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-07-09T10:00:00Z")?
            .with_timezone(&chrono::Utc);
        let first_id =
            insert_audit_log(&db.state.db, user_id, org_id, "target", created_at).await?;
        let second_id =
            insert_audit_log(&db.state.db, user_id, org_id, "target", created_at).await?;
        let third_id =
            insert_audit_log(&db.state.db, user_id, org_id, "target", created_at).await?;
        insert_audit_log(&db.state.db, user_id, org_id, "other", created_at).await?;

        let Json(first_page) = list_audit_log(
            State(db.state.clone()),
            admin_guard(user_id, org_id),
            Query(AuditLogQuery {
                user_id: Some(user_id),
                org_id: Some(org_id),
                action: Some("target".into()),
                limit: Some(2),
                cursor: None,
            }),
        )
        .await?;
        assert_eq!(entry_ids(&first_page), vec![third_id, second_id]);
        assert_eq!(first_page["pagination"]["limit"].as_i64(), Some(2));
        assert_eq!(first_page["pagination"]["has_more"].as_bool(), Some(true));

        let cursor = first_page["pagination"]["next_cursor"]
            .as_str()
            .expect("first page should include next_cursor")
            .to_string();
        let Json(second_page) = list_audit_log(
            State(db.state.clone()),
            admin_guard(user_id, org_id),
            Query(AuditLogQuery {
                user_id: Some(user_id),
                org_id: Some(org_id),
                action: Some("target".into()),
                limit: Some(2),
                cursor: Some(cursor),
            }),
        )
        .await?;
        assert_eq!(entry_ids(&second_page), vec![first_id]);
        assert_eq!(second_page["pagination"]["has_more"].as_bool(), Some(false));
        assert!(second_page["pagination"]["next_cursor"].is_null());

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cas_access_log_cursor_paginates_with_hash_filter() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-07-09T10:00:00Z")?
            .with_timezone(&chrono::Utc);
        let first_id =
            insert_cas_access_log(&db.state.db, user_id, org_id, "hash-a", created_at).await?;
        let second_id =
            insert_cas_access_log(&db.state.db, user_id, org_id, "hash-a", created_at).await?;
        let third_id =
            insert_cas_access_log(&db.state.db, user_id, org_id, "hash-a", created_at).await?;
        insert_cas_access_log(&db.state.db, user_id, org_id, "hash-b", created_at).await?;

        let Json(first_page) = list_cas_access_log(
            State(db.state.clone()),
            admin_guard(user_id, org_id),
            Query(CasAccessLogQuery {
                cas_hash: Some("hash-a".into()),
                user_id: Some(user_id),
                org_id: Some(org_id),
                limit: Some(2),
                cursor: None,
            }),
        )
        .await?;
        assert_eq!(entry_ids(&first_page), vec![third_id, second_id]);
        assert_eq!(first_page["pagination"]["has_more"].as_bool(), Some(true));

        let cursor = first_page["pagination"]["next_cursor"]
            .as_str()
            .expect("first page should include next_cursor")
            .to_string();
        let Json(second_page) = list_cas_access_log(
            State(db.state.clone()),
            admin_guard(user_id, org_id),
            Query(CasAccessLogQuery {
                cas_hash: Some("hash-a".into()),
                user_id: Some(user_id),
                org_id: Some(org_id),
                limit: Some(2),
                cursor: Some(cursor),
            }),
        )
        .await?;
        assert_eq!(entry_ids(&second_page), vec![first_id]);
        assert_eq!(second_page["pagination"]["has_more"].as_bool(), Some(false));
        assert!(second_page["pagination"]["next_cursor"].is_null());

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn users_list_cursor_paginates_three_pages() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (admin_user_id, org_id) = insert_test_identity(&db.state.db).await?;
        set_user_created_at(&db.state.db, admin_user_id, fixed_timestamp(2000)).await?;

        let created_at = fixed_timestamp(2030);
        for idx in 1..=5 {
            insert_user(&db.state.db, uuid_tail(idx), org_id, created_at).await?;
        }

        let Json(first_page) = list_users(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(PaginationQuery {
                limit: Some(2),
                cursor: None,
            }),
        )
        .await?;
        assert_eq!(
            object_ids(&first_page, "users"),
            vec![uuid_tail(5), uuid_tail(4)]
        );
        assert_eq!(first_page["pagination"]["has_more"].as_bool(), Some(true));

        let cursor = required_next_cursor(&first_page);
        let Json(second_page) = list_users(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(PaginationQuery {
                limit: Some(2),
                cursor: Some(cursor),
            }),
        )
        .await?;
        assert_eq!(
            object_ids(&second_page, "users"),
            vec![uuid_tail(3), uuid_tail(2)]
        );

        let cursor = required_next_cursor(&second_page);
        let Json(third_page) = list_users(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(PaginationQuery {
                limit: Some(2),
                cursor: Some(cursor),
            }),
        )
        .await?;
        let third_ids = object_ids(&third_page, "users");
        assert_eq!(third_ids[0], uuid_tail(1));
        assert_eq!(third_page["pagination"]["has_more"].as_bool(), Some(false));

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn git_tracking_authorization_grant_revoke_updates_list_audit_and_guard(
    ) -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (admin_user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let developer_id = uuid_tail(301);
        insert_user(&db.state.db, developer_id, org_id, fixed_timestamp(2030)).await?;

        let default_authorization: (bool, Option<chrono::DateTime<chrono::Utc>>, Option<Uuid>) =
            sqlx::query_as(
                "SELECT git_tracking_upload_enabled, git_tracking_upload_authorized_at, \
                    git_tracking_upload_authorized_by \
             FROM org_members WHERE user_id = $1 AND org_id = $2",
            )
            .bind(developer_id)
            .bind(org_id)
            .fetch_one(&db.state.db)
            .await?;
        assert_eq!(default_authorization, (false, None, None));

        let Json(default_list) = list_users(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(PaginationQuery {
                limit: Some(100),
                cursor: None,
            }),
        )
        .await?;
        let default_developer = listed_user(&default_list, developer_id);
        assert_eq!(
            default_developer["git_tracking_upload_enabled"].as_bool(),
            Some(false)
        );
        assert!(default_developer["git_tracking_upload_authorized_at"].is_null());
        assert!(default_developer["git_tracking_upload_authorized_by"].is_null());

        let unauthorized = crate::auth::middleware::require_git_tracking_upload_authorization(
            &db.state.db,
            developer_id,
            Some(org_id),
        )
        .await
        .expect_err("new organization members must not be authorized to upload");
        assert_error_status(unauthorized, StatusCode::FORBIDDEN);

        let Json(granted) = update_git_tracking_upload_authorization(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Path(developer_id),
            Json(GitTrackingUploadAuthorizationRequest { authorized: true }),
        )
        .await?;
        assert_eq!(granted["git_tracking_upload_enabled"].as_bool(), Some(true));
        assert_eq!(
            granted["git_tracking_upload_authorized_by"].as_str(),
            Some(admin_user_id.to_string().as_str())
        );
        assert!(granted["git_tracking_upload_authorized_at"].is_string());

        crate::auth::middleware::require_git_tracking_upload_authorization(
            &db.state.db,
            developer_id,
            Some(org_id),
        )
        .await?;

        let Json(granted_list) = list_users(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(PaginationQuery {
                limit: Some(100),
                cursor: None,
            }),
        )
        .await?;
        let granted_developer = listed_user(&granted_list, developer_id);
        assert_eq!(
            granted_developer["git_tracking_upload_enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            granted_developer["git_tracking_upload_authorized_by"].as_str(),
            Some(admin_user_id.to_string().as_str())
        );
        assert!(granted_developer["git_tracking_upload_authorized_at"].is_string());

        let Json(revoked) = update_git_tracking_upload_authorization(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Path(developer_id),
            Json(GitTrackingUploadAuthorizationRequest { authorized: false }),
        )
        .await?;
        assert_eq!(
            revoked["git_tracking_upload_enabled"].as_bool(),
            Some(false)
        );
        assert!(revoked["git_tracking_upload_authorized_at"].is_null());
        assert!(revoked["git_tracking_upload_authorized_by"].is_null());

        let revoked_immediately =
            crate::auth::middleware::require_git_tracking_upload_authorization(
                &db.state.db,
                developer_id,
                Some(org_id),
            )
            .await
            .expect_err("revocation must take effect without refreshing credentials");
        assert_error_status(revoked_immediately, StatusCode::FORBIDDEN);

        let Json(revoked_list) = list_users(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(PaginationQuery {
                limit: Some(100),
                cursor: None,
            }),
        )
        .await?;
        let revoked_developer = listed_user(&revoked_list, developer_id);
        assert_eq!(
            revoked_developer["git_tracking_upload_enabled"].as_bool(),
            Some(false)
        );
        assert!(revoked_developer["git_tracking_upload_authorized_at"].is_null());
        assert!(revoked_developer["git_tracking_upload_authorized_by"].is_null());

        let audit_entries: Vec<(
            String,
            Option<Uuid>,
            Option<Uuid>,
            Option<String>,
            Option<String>,
            Option<Value>,
        )> = sqlx::query_as(
            "SELECT action, user_id, org_id, resource_type, resource_id, details \
             FROM audit_log WHERE resource_id = $1 ORDER BY id",
        )
        .bind(developer_id.to_string())
        .fetch_all(&db.state.db)
        .await?;
        assert_eq!(audit_entries.len(), 2);
        assert_eq!(
            audit_entries[0],
            (
                "developer.git_tracking_upload.grant".into(),
                Some(admin_user_id),
                Some(org_id),
                Some("user".into()),
                Some(developer_id.to_string()),
                Some(json!({ "authorized": true })),
            )
        );
        assert_eq!(
            audit_entries[1],
            (
                "developer.git_tracking_upload.revoke".into(),
                Some(admin_user_id),
                Some(org_id),
                Some("user".into()),
                Some(developer_id.to_string()),
                Some(json!({ "authorized": false })),
            )
        );

        sqlx::query(
            "UPDATE org_members SET git_tracking_upload_enabled = true \
             WHERE user_id = $1 AND org_id = $2",
        )
        .bind(developer_id)
        .bind(org_id)
        .execute(&db.state.db)
        .await?;
        sqlx::query("UPDATE users SET status = 'disabled' WHERE id = $1")
            .bind(developer_id)
            .execute(&db.state.db)
            .await?;
        let disabled = crate::auth::middleware::require_git_tracking_upload_authorization(
            &db.state.db,
            developer_id,
            Some(org_id),
        )
        .await
        .expect_err("an authorized but disabled account must still be rejected");
        assert_error_status(disabled, StatusCode::UNAUTHORIZED);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn git_tracking_authorization_update_returns_not_found_for_cross_org_target(
    ) -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (admin_user_id, admin_org_id) = insert_test_identity(&db.state.db).await?;
        let other_org_id = uuid_tail(302);
        let other_user_id = uuid_tail(303);
        insert_organization(&db.state.db, other_org_id, "Other Authorization Org").await?;
        insert_user(
            &db.state.db,
            other_user_id,
            other_org_id,
            fixed_timestamp(2030),
        )
        .await?;

        let error = update_git_tracking_upload_authorization(
            State(db.state.clone()),
            admin_guard(admin_user_id, admin_org_id),
            Path(other_user_id),
            Json(GitTrackingUploadAuthorizationRequest { authorized: true }),
        )
        .await
        .expect_err("an administrator must not authorize a user in another organization");
        assert_error_status(error, StatusCode::NOT_FOUND);

        let still_disabled: bool = sqlx::query_scalar(
            "SELECT git_tracking_upload_enabled FROM org_members \
             WHERE user_id = $1 AND org_id = $2",
        )
        .bind(other_user_id)
        .bind(other_org_id)
        .fetch_one(&db.state.db)
        .await?;
        assert!(!still_disabled);

        let cross_org_audit_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM audit_log WHERE resource_id = $1")
                .bind(other_user_id.to_string())
                .fetch_one(&db.state.db)
                .await?;
        assert_eq!(cross_org_audit_count, 0);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bulk_git_tracking_authorization_is_atomic_and_audited() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (admin_user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let first_user_id = uuid_tail(304);
        let second_user_id = uuid_tail(305);
        insert_user(&db.state.db, first_user_id, org_id, fixed_timestamp(2030)).await?;
        insert_user(&db.state.db, second_user_id, org_id, fixed_timestamp(2031)).await?;

        let Json(granted) = bulk_authorize_git_tracking_upload(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Json(BulkGitTrackingUploadAuthorizationRequest {
                user_ids: vec![second_user_id, first_user_id, first_user_id],
            }),
        )
        .await?;
        assert_eq!(granted["authorized_count"].as_u64(), Some(2));

        let authorizations: Vec<(Uuid, bool, Option<Uuid>)> = sqlx::query_as(
            "SELECT user_id, git_tracking_upload_enabled, git_tracking_upload_authorized_by \
             FROM org_members WHERE org_id = $1 AND user_id = ANY($2::uuid[]) ORDER BY user_id",
        )
        .bind(org_id)
        .bind(vec![first_user_id, second_user_id])
        .fetch_all(&db.state.db)
        .await?;
        assert_eq!(
            authorizations,
            vec![
                (first_user_id, true, Some(admin_user_id)),
                (second_user_id, true, Some(admin_user_id)),
            ]
        );

        let audit_entries: Vec<(String, Option<Value>)> = sqlx::query_as(
            "SELECT resource_id, details FROM audit_log \
             WHERE action = 'developer.git_tracking_upload.grant' \
               AND resource_id = ANY($1::text[]) ORDER BY resource_id",
        )
        .bind(vec![first_user_id.to_string(), second_user_id.to_string()])
        .fetch_all(&db.state.db)
        .await?;
        assert_eq!(audit_entries.len(), 2);
        assert!(audit_entries.iter().all(|(_, details)| {
            details.as_ref() == Some(&json!({ "authorized": true, "bulk": true }))
        }));

        let other_org_id = uuid_tail(306);
        let other_user_id = uuid_tail(307);
        let untouched_user_id = uuid_tail(308);
        insert_organization(&db.state.db, other_org_id, "Other Bulk Authorization Org").await?;
        insert_user(
            &db.state.db,
            other_user_id,
            other_org_id,
            fixed_timestamp(2032),
        )
        .await?;
        insert_user(
            &db.state.db,
            untouched_user_id,
            org_id,
            fixed_timestamp(2033),
        )
        .await?;

        let error = bulk_authorize_git_tracking_upload(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Json(BulkGitTrackingUploadAuthorizationRequest {
                user_ids: vec![untouched_user_id, other_user_id],
            }),
        )
        .await
        .expect_err("a cross-organization user must reject the whole batch");
        assert_error_status(error, StatusCode::NOT_FOUND);

        let untouched_authorized: bool = sqlx::query_scalar(
            "SELECT git_tracking_upload_enabled FROM org_members \
             WHERE org_id = $1 AND user_id = $2",
        )
        .bind(org_id)
        .bind(untouched_user_id)
        .fetch_one(&db.state.db)
        .await?;
        assert!(!untouched_authorized);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn api_keys_cursor_paginates_active_keys_only() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (admin_user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let created_at = fixed_timestamp(2030);
        for idx in 1..=3 {
            insert_api_key(
                &db.state.db,
                uuid_tail(idx),
                admin_user_id,
                org_id,
                created_at,
                false,
            )
            .await?;
        }
        insert_api_key(
            &db.state.db,
            uuid_tail(9),
            admin_user_id,
            org_id,
            created_at,
            true,
        )
        .await?;

        let Json(first_page) = list_api_keys(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(PaginationQuery {
                limit: Some(2),
                cursor: None,
            }),
        )
        .await?;
        assert_eq!(
            object_ids(&first_page, "api_keys"),
            vec![uuid_tail(3), uuid_tail(2)]
        );
        assert_eq!(first_page["pagination"]["has_more"].as_bool(), Some(true));

        let cursor = required_next_cursor(&first_page);
        let Json(second_page) = list_api_keys(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(PaginationQuery {
                limit: Some(2),
                cursor: Some(cursor.clone()),
            }),
        )
        .await?;
        assert_eq!(object_ids(&second_page, "api_keys"), vec![uuid_tail(1)]);

        let Json(user_first_page) = list_user_api_keys(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Path(admin_user_id),
            Query(PaginationQuery {
                limit: Some(2),
                cursor: None,
            }),
        )
        .await?;
        assert_eq!(
            object_ids(&user_first_page, "api_keys"),
            vec![uuid_tail(3), uuid_tail(2)]
        );

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn organizations_and_departments_cursor_paginate_by_name() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (admin_user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let org_a = uuid_tail(101);
        let org_b = uuid_tail(102);
        let org_c = uuid_tail(103);
        let org_d = uuid_tail(104);
        for org in [org_a, org_b, org_c] {
            insert_organization(&db.state.db, org, "Aaa Cursor Org").await?;
        }
        insert_organization(&db.state.db, org_d, "Bbb Cursor Org").await?;

        let Json(first_page) = list_organizations(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(ListOrganizationsQuery {
                include_personal: Some(false),
                q: None,
                limit: Some(2),
                cursor: None,
            }),
        )
        .await?;
        assert_eq!(object_ids(&first_page, "organizations"), vec![org_a, org_b]);

        let cursor = required_next_cursor(&first_page);
        let Json(second_page) = list_organizations(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(ListOrganizationsQuery {
                include_personal: Some(false),
                q: None,
                limit: Some(2),
                cursor: Some(cursor.clone()),
            }),
        )
        .await?;
        let second_orgs = object_ids(&second_page, "organizations");
        assert_eq!(second_orgs[0], org_c);

        let error = list_organizations(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(ListOrganizationsQuery {
                include_personal: Some(false),
                q: Some("bbb cursor".into()),
                limit: Some(2),
                cursor: Some(cursor),
            }),
        )
        .await
        .expect_err("a cursor must stay bound to its organization filters");
        assert_error_status(error, StatusCode::BAD_REQUEST);

        let Json(searched_orgs) = list_organizations(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(ListOrganizationsQuery {
                include_personal: Some(false),
                q: Some("bbb cursor".into()),
                limit: Some(100),
                cursor: None,
            }),
        )
        .await?;
        assert_eq!(object_ids(&searched_orgs, "organizations"), vec![org_d]);

        let dept_a = uuid_tail(201);
        let dept_b = uuid_tail(202);
        let dept_c = uuid_tail(203);
        let dept_other_org = uuid_tail(204);
        for dept in [dept_a, dept_b, dept_c] {
            insert_department(&db.state.db, dept, org_a, "Platform").await?;
        }
        insert_department(&db.state.db, dept_other_org, org_b, "Platform").await?;

        let Json(first_dept_page) = list_departments(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(ListDepartmentsQuery {
                org_id: Some(org_a),
                q: None,
                limit: Some(2),
                cursor: None,
            }),
        )
        .await?;
        assert_eq!(
            object_ids(&first_dept_page, "departments"),
            vec![dept_a, dept_b]
        );

        let cursor = required_next_cursor(&first_dept_page);
        let Json(second_dept_page) = list_departments(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(ListDepartmentsQuery {
                org_id: Some(org_a),
                q: None,
                limit: Some(2),
                cursor: Some(cursor.clone()),
            }),
        )
        .await?;
        assert_eq!(object_ids(&second_dept_page, "departments"), vec![dept_c]);

        let error = list_departments(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(ListDepartmentsQuery {
                org_id: Some(org_b),
                q: None,
                limit: Some(2),
                cursor: Some(cursor),
            }),
        )
        .await
        .expect_err("a cursor must stay bound to its department filters");
        assert_error_status(error, StatusCode::BAD_REQUEST);

        let Json(searched_departments) = list_departments(
            State(db.state.clone()),
            admin_guard(admin_user_id, org_id),
            Query(ListDepartmentsQuery {
                org_id: Some(org_a),
                q: Some("PLAT".into()),
                limit: Some(100),
                cursor: None,
            }),
        )
        .await?;
        assert_eq!(
            object_ids(&searched_departments, "departments"),
            vec![dept_a, dept_b, dept_c]
        );

        db.cleanup().await?;
        Ok(())
    }

    async fn insert_test_identity(pool: &PgPool) -> anyhow::Result<(Uuid, Uuid)> {
        let user_id = Uuid::new_v4();
        let org_id = Uuid::new_v4();

        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org_id)
            .bind("Admin Log Pagination Test Org")
            .bind(format!("admin-log-pagination-test-{}", org_id.simple()))
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO users (id, email, name, default_org_id) VALUES ($1, $2, $3, $4)")
            .bind(user_id)
            .bind(format!("{user_id}@example.com"))
            .bind("Admin Log Pagination Test User")
            .bind(org_id)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO org_members (user_id, org_id, role) VALUES ($1, $2, $3)")
            .bind(user_id)
            .bind(org_id)
            .bind("admin")
            .execute(pool)
            .await?;

        Ok((user_id, org_id))
    }

    async fn set_user_created_at(
        pool: &PgPool,
        user_id: Uuid,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE users SET created_at = $1 WHERE id = $2")
            .bind(created_at)
            .bind(user_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn insert_user(
        pool: &PgPool,
        user_id: Uuid,
        org_id: Uuid,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO users (id, email, name, default_org_id, created_at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user_id)
        .bind(format!("{user_id}@example.com"))
        .bind(format!("Test User {user_id}"))
        .bind(org_id)
        .bind(created_at)
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO org_members (user_id, org_id, role) VALUES ($1, $2, $3)")
            .bind(user_id)
            .bind(org_id)
            .bind("member")
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn insert_api_key(
        pool: &PgPool,
        key_id: Uuid,
        user_id: Uuid,
        org_id: Uuid,
        created_at: chrono::DateTime<chrono::Utc>,
        revoked: bool,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO api_keys (id, user_id, org_id, key_prefix, key_hash, name, scopes, created_at, revoked_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, CASE WHEN $9 THEN $8 ELSE NULL END)",
        )
        .bind(key_id)
        .bind(user_id)
        .bind(org_id)
        .bind(format!("k{}", &key_id.simple().to_string()[..7]))
        .bind(format!("hash-{key_id}"))
        .bind(format!("Key {key_id}"))
        .bind(vec!["metrics:write".to_string()])
        .bind(created_at)
        .bind(revoked)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_organization(pool: &PgPool, org_id: Uuid, name: &str) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org_id)
            .bind(name)
            .bind(format!("org-{}", org_id.simple()))
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn insert_department(
        pool: &PgPool,
        department_id: Uuid,
        org_id: Uuid,
        name: &str,
    ) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO departments (id, org_id, name, slug) VALUES ($1, $2, $3, $4)")
            .bind(department_id)
            .bind(org_id)
            .bind(name)
            .bind(format!("dept-{}", department_id.simple()))
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn insert_audit_log(
        pool: &PgPool,
        user_id: Uuid,
        org_id: Uuid,
        action: &str,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar(
            "INSERT INTO audit_log (user_id, org_id, action, created_at) VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(user_id)
        .bind(org_id)
        .bind(action)
        .bind(created_at)
        .fetch_one(pool)
        .await?)
    }

    async fn insert_cas_access_log(
        pool: &PgPool,
        user_id: Uuid,
        org_id: Uuid,
        cas_hash: &str,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar(
            "INSERT INTO cas_access_log (user_id, org_id, cas_hash, access_method, created_at) VALUES ($1, $2, $3, 'api', $4) RETURNING id",
        )
        .bind(user_id)
        .bind(org_id)
        .bind(cas_hash)
        .bind(created_at)
        .fetch_one(pool)
        .await?)
    }

    fn entry_ids(page: &Value) -> Vec<i64> {
        page["entries"]
            .as_array()
            .expect("entries should be an array")
            .iter()
            .map(|entry| entry["id"].as_i64().expect("entry id should be an integer"))
            .collect()
    }

    fn object_ids(page: &Value, key: &str) -> Vec<Uuid> {
        page[key]
            .as_array()
            .expect("response field should be an array")
            .iter()
            .map(|entry| {
                Uuid::parse_str(entry["id"].as_str().expect("entry id should be a string"))
                    .expect("entry id should be a UUID")
            })
            .collect()
    }

    fn listed_user(page: &Value, user_id: Uuid) -> &Value {
        page["users"]
            .as_array()
            .expect("users should be an array")
            .iter()
            .find(|user| {
                user["id"].as_str().and_then(|id| Uuid::parse_str(id).ok()) == Some(user_id)
            })
            .expect("user should be present in the administrator's organization")
    }

    fn assert_error_status(error: AppError, expected: StatusCode) {
        assert_eq!(error.into_response().status(), expected);
    }

    fn required_next_cursor(page: &Value) -> String {
        page["pagination"]["next_cursor"]
            .as_str()
            .expect("page should include next_cursor")
            .to_string()
    }

    fn fixed_timestamp(year: i32) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(&format!("{year}-07-09T10:00:00Z"))
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn uuid_tail(value: u32) -> Uuid {
        Uuid::parse_str(&format!("00000000-0000-0000-0000-{value:012}")).unwrap()
    }

    fn admin_guard(user_id: Uuid, org_id: Uuid) -> AdminGuard {
        AdminGuard(AuthIdentity {
            user_id,
            email: format!("{user_id}@example.com"),
            name: "Admin Log Pagination Test User".into(),
            org_id: Some(org_id),
            org_slug: Some(format!("admin-log-pagination-test-{}", org_id.simple())),
            department_id: None,
            role: Some("admin".into()),
            scopes: vec![],
            auth_method: AuthMethod::BearerToken,
        })
    }

    fn test_config(database_url: &str) -> AppConfig {
        AppConfig {
            database_url: database_url.to_string(),
            database_max_connections: 20,
            database_min_connections: 1,
            database_acquire_timeout_seconds: 5,
            redis_url: "redis://127.0.0.1:6379".to_string(),
            jwt_secret: "admin-log-pagination-test-secret".to_string(),
            s3_endpoint: "http://localhost:9000".to_string(),
            s3_bucket: "git-ai-cas".to_string(),
            s3_access_key: "minioadmin".to_string(),
            s3_secret_key: "minioadmin".to_string(),
            s3_region: "us-east-1".to_string(),
            cas_upload_concurrency: 8,
            auth_password_concurrency: 8,
            metrics_rollup_write_mode: MetricsRollupWriteMode::Sync,
            metrics_rollup_worker_enabled: false,
            metrics_rollup_worker_interval_seconds: 5,
            metrics_rollup_worker_batch_size: 100,
            dashboard_use_rollups: false,
            rate_limit_metrics_max_requests: 60,
            rate_limit_metrics_window_seconds: 60,
            rate_limit_cas_upload_max_requests: 30,
            rate_limit_cas_upload_window_seconds: 60,
            rate_limit_cas_read_max_requests: 100,
            rate_limit_cas_read_window_seconds: 60,
            rate_limit_oauth_max_requests: 600,
            rate_limit_oauth_window_seconds: 60,
            rate_limit_auth_max_requests: 300,
            rate_limit_auth_window_seconds: 60,
            rate_limit_admin_max_requests: 300,
            rate_limit_admin_window_seconds: 60,
            rate_limit_default_max_requests: 300,
            rate_limit_default_window_seconds: 60,
            base_url: "http://localhost:8080".to_string(),
            sentry_dsn: String::new(),
            posthog_host: String::new(),
            posthog_api_key: String::new(),
        }
    }

    fn test_database_url() -> String {
        dotenvy::dotenv().ok();
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://gitai:gitai@localhost:5433/gitai_enterprise".into())
    }

    fn unique_test_database_name() -> String {
        format!(
            "git_ai_admin_log_pagination_test_{}",
            Uuid::new_v4().simple()
        )
    }

    fn database_url_for_database(database_url: &str, database: &str) -> anyhow::Result<String> {
        let mut url = url::Url::parse(database_url)?;
        url.set_path(database);
        Ok(url.to_string())
    }

    async fn create_database(pool: &PgPool, db_name: &str) -> anyhow::Result<()> {
        sqlx::query(&format!("CREATE DATABASE {}", quote_ident(db_name)))
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn drop_database(pool: &PgPool, db_name: &str) -> anyhow::Result<()> {
        sqlx::query(&format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            quote_ident(db_name)
        ))
        .execute(pool)
        .await?;
        Ok(())
    }

    fn quote_ident(identifier: &str) -> String {
        format!("\"{}\"", identifier.replace('"', "\"\""))
    }
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

fn decode_log_cursor(cursor: Option<&str>) -> Result<Option<TimeIdCursor>, AppError> {
    cursor.map(decode_time_id_cursor).transpose()
}

fn encode_log_cursor(
    timestamp: chrono::DateTime<chrono::Utc>,
    id: i64,
) -> Result<String, AppError> {
    encode_cursor(&TimeIdCursor::new(timestamp, id))
}

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub user_id: Option<Uuid>,
    pub org_id: Option<Uuid>,
    pub action: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// GET /api/v1/audit-log — Query audit log
pub async fn list_audit_log(
    State(state): State<AppState>,
    _auth: AdminGuard,
    axum::extract::Query(query): axum::extract::Query<AuditLogQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = clamp_limit(query.limit, DEFAULT_LIMIT, MAX_LIMIT);
    let cursor = decode_log_cursor(query.cursor.as_deref())?;
    let cursor_timestamp = cursor.as_ref().map(|cursor| cursor.timestamp.clone());
    let cursor_id = cursor.as_ref().map(|cursor| cursor.id);

    let mut rows: Vec<(i64, Option<Uuid>, Option<Uuid>, String, Option<String>, Option<String>, Option<serde_json::Value>, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        r#"SELECT id, user_id, org_id, action, resource_type, resource_id, details, ip_address, user_agent, created_at
        FROM audit_log
        WHERE ($1::uuid IS NULL OR user_id = $1)
          AND ($2::uuid IS NULL OR org_id = $2)
          AND ($3::text IS NULL OR action = $3)
          AND ($4::timestamptz IS NULL OR (created_at, id) < ($4::timestamptz, $5::bigint))
        ORDER BY created_at DESC, id DESC
        LIMIT $6"#
    )
    .bind(query.user_id)
    .bind(query.org_id)
    .bind(&query.action)
    .bind(cursor_timestamp)
    .bind(cursor_id)
    .bind(fetch_limit(limit))
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let has_more = truncate_to_limit(&mut rows, limit);
    let next_cursor = if has_more {
        rows.last()
            .map(|(id, _, _, _, _, _, _, _, _, created_at)| {
                encode_log_cursor(created_at.clone(), *id)
            })
            .transpose()?
    } else {
        None
    };

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

    Ok(Json(json!({
        "entries": entries,
        "count": entries.len(),
        "pagination": pagination_meta(limit, has_more, next_cursor),
    })))
}

// ================================================================
// Repository access rules (whitelist / blacklist)
// ================================================================

#[derive(Debug, Deserialize)]
pub struct CreateRepoAccessRuleRequest {
    pub org_id: Uuid,
    pub rule_type: String, // "whitelist" or "blacklist"
    pub pattern: String,   // Glob pattern, e.g., "github.com/myorg/*"
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
    pub value: serde_json::Value, // true/false or { "debug": bool, "release": bool }
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
    pub export_type: String, // "csv" or "json"
    pub query_type: String,  // "summary", "developers", "projects", "organizations", "tools"
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
    pub cursor: Option<String>,
}

/// GET /api/admin/cas-access-log — Query CAS access audit log
pub async fn list_cas_access_log(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Query(query): Query<CasAccessLogQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = clamp_limit(query.limit, DEFAULT_LIMIT, MAX_LIMIT);
    let cursor = decode_log_cursor(query.cursor.as_deref())?;
    let cursor_timestamp = cursor.as_ref().map(|cursor| cursor.timestamp.clone());
    let cursor_id = cursor.as_ref().map(|cursor| cursor.id);

    let mut rows: Vec<(i64, Option<Uuid>, Option<Uuid>, Option<Uuid>, String, String, Option<String>, Option<String>, Option<String>, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        r#"SELECT id, user_id, org_id, api_key_id, cas_hash, access_method, purpose, ip_address, user_agent, created_at
        FROM cas_access_log
        WHERE ($1::text IS NULL OR cas_hash = $1)
          AND ($2::uuid IS NULL OR user_id = $2)
          AND ($3::uuid IS NULL OR org_id = $3)
          AND ($4::timestamptz IS NULL OR (created_at, id) < ($4::timestamptz, $5::bigint))
        ORDER BY created_at DESC, id DESC
        LIMIT $6"#
    )
    .bind(&query.cas_hash)
    .bind(query.user_id)
    .bind(query.org_id)
    .bind(cursor_timestamp)
    .bind(cursor_id)
    .bind(fetch_limit(limit))
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let has_more = truncate_to_limit(&mut rows, limit);
    let next_cursor = if has_more {
        rows.last()
            .map(|(id, _, _, _, _, _, _, _, _, created_at)| {
                encode_log_cursor(created_at.clone(), *id)
            })
            .transpose()?
    } else {
        None
    };

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

    Ok(Json(json!({
        "entries": entries,
        "count": entries.len(),
        "pagination": pagination_meta(limit, has_more, next_cursor),
    })))
}

// ================================================================
// User-specific API Keys (for user management page)
// ================================================================

/// GET /api/admin/users/{id}/api-keys — List API keys for a specific user
pub async fn list_user_api_keys(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(user_id): Path<Uuid>,
    Query(query): Query<PaginationQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = clamp_limit(query.limit, DEFAULT_LIMIT, MAX_LIMIT);
    let cursor = decode_optional_time_uuid_cursor(query.cursor.as_deref())?;
    let cursor_timestamp = cursor.as_ref().map(|cursor| cursor.timestamp.clone());
    let cursor_id = cursor.as_ref().map(|cursor| cursor.id);

    let mut rows: Vec<(
        Uuid,
        String,
        Option<String>,
        Vec<String>,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    )> = sqlx::query_as(
        r#"SELECT id, key_prefix, name, scopes, created_at, expires_at, last_used_at
        FROM api_keys
        WHERE user_id = $1
          AND revoked_at IS NULL
          AND ($2::timestamptz IS NULL OR (created_at, id) < ($2::timestamptz, $3::uuid))
        ORDER BY created_at DESC, id DESC
        LIMIT $4"#,
    )
    .bind(user_id)
    .bind(cursor_timestamp)
    .bind(cursor_id)
    .bind(fetch_limit(limit))
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let has_more = truncate_to_limit(&mut rows, limit);
    let next_cursor = if has_more {
        rows.last()
            .map(|(id, _, _, _, created, _, _)| encode_time_uuid_cursor(created.clone(), *id))
            .transpose()?
    } else {
        None
    };

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

    Ok(Json(json!({
        "api_keys": result,
        "pagination": pagination_meta(limit, has_more, next_cursor),
    })))
}
