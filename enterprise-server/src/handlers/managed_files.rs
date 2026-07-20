use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Json, Response};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

use crate::auth::middleware::{AdminGuard, OptionalAuth};
use crate::error::AppError;
use crate::routes::AppState;

pub(crate) const MANAGED_FILE_MAX_BYTES: usize = 500 * 1024 * 1024;
pub(crate) const MANAGED_FILE_UPLOAD_BODY_MAX_BYTES: usize = 512 * 1024 * 1024;

pub async fn list_managed_files(
    State(state): State<AppState>,
    _auth: AdminGuard,
) -> Result<Json<Value>, AppError> {
    let files: Vec<(
        Uuid,
        String,
        String,
        Option<String>,
        bool,
        Option<String>,
        DateTime<Utc>,
        DateTime<Utc>,
    )> = sqlx::query_as(
        r#"SELECT id, slug, name, description, is_public, current_version, created_at, updated_at
           FROM managed_files
           ORDER BY updated_at DESC, slug ASC"#,
    )
    .fetch_all(&state.db)
    .await
    .map_err(AppError::Database)?;

    let versions: Vec<(
        Uuid,
        String,
        String,
        String,
        i64,
        String,
        DateTime<Utc>,
        Option<DateTime<Utc>>,
    )> = sqlx::query_as(
        r#"SELECT file_id, version, filename, sha256, size_bytes, content_type,
                  created_at, published_at
           FROM managed_file_versions
           ORDER BY created_at DESC"#,
    )
    .fetch_all(&state.db)
    .await
    .map_err(AppError::Database)?;

    let mut versions_by_file: HashMap<Uuid, Vec<Value>> = HashMap::new();
    for (file_id, version, filename, sha256, size_bytes, content_type, created_at, published_at) in
        versions
    {
        versions_by_file.entry(file_id).or_default().push(json!({
            "version": version,
            "filename": filename,
            "sha256": sha256,
            "size_bytes": size_bytes,
            "content_type": content_type,
            "created_at": created_at,
            "published_at": published_at,
        }));
    }

    let files = files
        .into_iter()
        .map(
            |(id, slug, name, description, is_public, current_version, created_at, updated_at)| {
                let versions = versions_by_file.remove(&id).unwrap_or_default();
                json!({
                    "id": id,
                    "slug": slug,
                    "name": name,
                    "description": description,
                    "is_public": is_public,
                    "current_version": current_version,
                    "latest_download_url": format!("/files/{}/latest/download", slug),
                    "versions": versions,
                    "created_at": created_at,
                    "updated_at": updated_at,
                })
            },
        )
        .collect::<Vec<_>>();

    Ok(Json(json!({ "files": files })))
}

pub async fn upload_managed_file(
    State(state): State<AppState>,
    AdminGuard(identity): AdminGuard,
    mut multipart: Multipart,
) -> Result<Json<Value>, AppError> {
    let mut slug = String::new();
    let mut name = String::new();
    let mut description = String::new();
    let mut version = String::new();
    let mut is_public = true;
    let mut upload: Option<(String, String, Vec<u8>)> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Multipart error: {}", e)))?
    {
        let field_name = field.name().unwrap_or("").to_string();
        match field_name.as_str() {
            "slug" => slug = multipart_text(field).await?,
            "name" => name = multipart_text(field).await?,
            "description" => description = multipart_text(field).await?,
            "version" => version = multipart_text(field).await?,
            "is_public" => {
                is_public = matches!(multipart_text(field).await?.as_str(), "true" | "1" | "on")
            }
            "file" => {
                if upload.is_some() {
                    return Err(AppError::BadRequest(
                        "Only one managed file can be uploaded at a time".into(),
                    ));
                }
                let filename = field
                    .file_name()
                    .ok_or_else(|| AppError::BadRequest("Uploaded file needs a filename".into()))?
                    .to_string();
                validate_filename(&filename)?;
                let content_type = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("File error: {}", e)))?
                    .to_vec();
                validate_managed_file_size(data.len())?;
                upload = Some((filename, content_type, data));
            }
            _ => {}
        }
    }

    let slug = normalize_slug(&slug)?;
    let name = require_text(&name, "name")?.to_string();
    let version = validate_version(&version)?.to_string();
    let (filename, content_type, data) =
        upload.ok_or_else(|| AppError::BadRequest("file is required".into()))?;
    let existing_file_id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM managed_files WHERE slug = $1")
            .bind(&slug)
            .fetch_optional(&state.db)
            .await
            .map_err(AppError::Database)?;
    let file_id = if let Some(file_id) = existing_file_id {
        let version_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM managed_file_versions WHERE file_id = $1 AND version = $2)",
        )
        .bind(file_id)
        .bind(&version)
        .fetch_one(&state.db)
        .await
        .map_err(AppError::Database)?;
        if version_exists {
            return Err(AppError::Conflict(format!(
                "File {} version {} already exists",
                slug, version
            )));
        }
        sqlx::query(
            r#"UPDATE managed_files
               SET name = $2, description = NULLIF($3, ''), is_public = $4, updated_at = now()
               WHERE id = $1"#,
        )
        .bind(file_id)
        .bind(&name)
        .bind(description.trim())
        .bind(is_public)
        .execute(&state.db)
        .await
        .map_err(AppError::Database)?;
        file_id
    } else {
        sqlx::query_scalar(
            r#"INSERT INTO managed_files (slug, name, description, is_public, created_by)
               VALUES ($1, $2, NULLIF($3, ''), $4, $5)
               RETURNING id"#,
        )
        .bind(&slug)
        .bind(&name)
        .bind(description.trim())
        .bind(is_public)
        .bind(identity.user_id)
        .fetch_one(&state.db)
        .await
        .map_err(AppError::Database)?
    };

    let sha256 = sha256_hex(&data);
    let size_bytes = data.len() as i64;
    let storage_path = state
        .cas_store
        .put_managed_file(&slug, &version, &filename, &data)
        .await?;

    sqlx::query(
        r#"INSERT INTO managed_file_versions
           (file_id, version, filename, sha256, size_bytes, content_type, storage_path, created_by)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
    )
    .bind(file_id)
    .bind(&version)
    .bind(&filename)
    .bind(&sha256)
    .bind(size_bytes)
    .bind(normalize_content_type(&content_type))
    .bind(&storage_path)
    .bind(identity.user_id)
    .execute(&state.db)
    .await
    .map_err(AppError::Database)?;

    Ok(Json(json!({
        "success": true,
        "slug": slug,
        "version": version,
        "filename": filename,
        "sha256": sha256,
        "size_bytes": size_bytes,
        "status": "draft",
    })))
}

#[derive(Debug, Deserialize)]
pub struct PublishManagedFileRequest {
    version: String,
}

pub async fn publish_managed_file(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(slug): Path<String>,
    Json(req): Json<PublishManagedFileRequest>,
) -> Result<Json<Value>, AppError> {
    let slug = normalize_slug(&slug)?;
    let version = validate_version(&req.version)?;
    let result = sqlx::query(
        r#"WITH selected AS (
               UPDATE managed_file_versions mfv
               SET published_at = COALESCE(published_at, now())
               FROM managed_files mf
               WHERE mfv.file_id = mf.id AND mf.slug = $1 AND mfv.version = $2
               RETURNING mfv.file_id
           )
           UPDATE managed_files mf
           SET current_version = $2, updated_at = now()
           FROM selected
           WHERE mf.id = selected.file_id"#,
    )
    .bind(&slug)
    .bind(version)
    .execute(&state.db)
    .await
    .map_err(AppError::Database)?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!(
            "File {} version {} was not found",
            slug, version
        )));
    }

    Ok(Json(json!({
        "success": true,
        "slug": slug,
        "version": version,
        "download_url": format!("/files/{}/latest/download", slug),
    })))
}

#[derive(Debug, Deserialize)]
pub struct UpdateManagedFileRequest {
    name: String,
    description: Option<String>,
    is_public: bool,
}

pub async fn update_managed_file(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path(slug): Path<String>,
    Json(req): Json<UpdateManagedFileRequest>,
) -> Result<Json<Value>, AppError> {
    let slug = normalize_slug(&slug)?;
    let name = require_text(&req.name, "name")?;
    let result = sqlx::query(
        r#"UPDATE managed_files
           SET name = $2, description = $3, is_public = $4, updated_at = now()
           WHERE slug = $1"#,
    )
    .bind(&slug)
    .bind(name)
    .bind(req.description.as_deref().map(str::trim))
    .bind(req.is_public)
    .execute(&state.db)
    .await
    .map_err(AppError::Database)?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("File {} was not found", slug)));
    }
    Ok(Json(json!({ "success": true, "slug": slug })))
}

pub async fn delete_managed_file_version(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path((slug, version)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let slug = normalize_slug(&slug)?;
    let version = validate_version(&version)?;
    let row: Option<(Uuid, Option<String>, String)> = sqlx::query_as(
        r#"SELECT mf.id, mf.current_version, mfv.storage_path
           FROM managed_files mf
           JOIN managed_file_versions mfv ON mfv.file_id = mf.id
           WHERE mf.slug = $1 AND mfv.version = $2"#,
    )
    .bind(&slug)
    .bind(version)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?;
    let Some((file_id, current_version, storage_path)) = row else {
        return Err(AppError::NotFound(format!(
            "File {} version {} was not found",
            slug, version
        )));
    };
    if current_version.as_deref() == Some(version) {
        return Err(AppError::Conflict(
            "The current published version cannot be deleted".into(),
        ));
    }

    sqlx::query("DELETE FROM managed_file_versions WHERE file_id = $1 AND version = $2")
        .bind(file_id)
        .bind(version)
        .execute(&state.db)
        .await
        .map_err(AppError::Database)?;
    state.cas_store.delete_path(&storage_path).await?;

    Ok(Json(json!({ "success": true })))
}

pub async fn download_managed_file(
    State(state): State<AppState>,
    OptionalAuth(identity): OptionalAuth,
    Path((slug, requested_version)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let slug = normalize_slug(&slug)?;
    let file: Option<(Uuid, bool, Option<String>)> =
        sqlx::query_as("SELECT id, is_public, current_version FROM managed_files WHERE slug = $1")
            .bind(&slug)
            .fetch_optional(&state.db)
            .await
            .map_err(AppError::Database)?;
    let Some((file_id, is_public, current_version)) = file else {
        return Err(AppError::NotFound("File not found".into()));
    };
    if !is_public && identity.is_none() {
        return Err(AppError::Unauthorized(
            "Login is required to download this file".into(),
        ));
    }

    let version = if requested_version == "latest" {
        current_version.ok_or_else(|| AppError::NotFound("File has not been published".into()))?
    } else {
        validate_version(&requested_version)?.to_string()
    };
    let version_row: Option<(String, String, String)> = sqlx::query_as(
        r#"SELECT filename, content_type, storage_path
           FROM managed_file_versions
           WHERE file_id = $1 AND version = $2 AND published_at IS NOT NULL"#,
    )
    .bind(file_id)
    .bind(&version)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?;
    let Some((filename, content_type, storage_path)) = version_row else {
        return Err(AppError::NotFound(
            "Published file version not found".into(),
        ));
    };

    let data = state
        .cas_store
        .get_release_path(&storage_path)
        .await?
        .ok_or_else(|| AppError::NotFound("File content not found".into()))?;
    let content_disposition = format!("attachment; filename=\"{}\"", ascii_filename(&filename));
    let cache_control = if requested_version == "latest" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, normalize_content_type(&content_type))
        .header(header::CONTENT_LENGTH, data.len())
        .header(header::CONTENT_DISPOSITION, content_disposition)
        .header(header::CACHE_CONTROL, cache_control)
        .body(Body::from(data))
        .map_err(|e| AppError::Internal(format!("Failed to build download response: {}", e)))
}

async fn multipart_text(field: axum::extract::multipart::Field<'_>) -> Result<String, AppError> {
    field
        .text()
        .await
        .map_err(|e| AppError::BadRequest(format!("Field error: {}", e)))
}

fn normalize_slug(value: &str) -> Result<String, AppError> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty()
        || value.len() > 80
        || !value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
    {
        return Err(AppError::BadRequest(
            "slug may only contain lowercase letters, numbers, hyphens, and underscores".into(),
        ));
    }
    Ok(value)
}

fn validate_version(value: &str) -> Result<&str, AppError> {
    let value = require_text(value, "version")?;
    if matches!(value, "." | "..")
        || value.len() > 80
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '+'))
    {
        return Err(AppError::BadRequest(
            "version contains invalid characters".into(),
        ));
    }
    Ok(value)
}

fn validate_filename(value: &str) -> Result<(), AppError> {
    let value = require_text(value, "filename")?;
    if value.len() > 240
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(AppError::BadRequest("filename is invalid".into()));
    }
    Ok(())
}

fn validate_managed_file_size(size_bytes: usize) -> Result<(), AppError> {
    if size_bytes == 0 {
        return Err(AppError::BadRequest("Uploaded file is empty".into()));
    }
    if size_bytes > MANAGED_FILE_MAX_BYTES {
        return Err(AppError::BadRequest(format!(
            "Uploaded file exceeds the {} MiB limit",
            MANAGED_FILE_MAX_BYTES / 1024 / 1024
        )));
    }
    Ok(())
}

fn require_text<'a>(value: &'a str, field: &str) -> Result<&'a str, AppError> {
    let value = value.trim();
    if value.is_empty() {
        Err(AppError::BadRequest(format!("{} is required", field)))
    } else {
        Ok(value)
    }
}

fn normalize_content_type(value: &str) -> &str {
    let value = value.trim();
    if value.is_empty() || value.len() > 200 || value.contains(['\r', '\n']) {
        "application/octet-stream"
    } else {
        value
    }
}

fn ascii_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_and_version_reject_path_traversal() {
        assert!(normalize_slug("../secret").is_err());
        assert!(validate_version("../../secret").is_err());
        assert!(validate_version("..").is_err());
        assert!(validate_filename("../secret.txt").is_err());
    }

    #[test]
    fn managed_file_size_rejects_empty_and_oversized_uploads() {
        assert!(validate_managed_file_size(0).is_err());
        assert!(validate_managed_file_size(MANAGED_FILE_MAX_BYTES).is_ok());
        assert!(validate_managed_file_size(MANAGED_FILE_MAX_BYTES + 1).is_err());
    }

    #[test]
    fn download_filename_is_header_safe() {
        assert_eq!(ascii_filename("说明 \\\".zip"), "__ __.zip");
    }
}
