use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Json, Response};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::auth::middleware::{AdminGuard, AuthExtractor, OptionalAuth};
use crate::error::AppError;
use crate::models::release::{ChannelInfo, FeatureFlagsResponse};
use crate::routes::AppState;

/// GET /worker/releases — Get release version info
pub async fn get_releases(
    State(state): State<AppState>,
    _auth: OptionalAuth,
) -> Result<Json<Value>, AppError> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT channel, version, checksum FROM release_channels"
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let mut channels_map: HashMap<String, ChannelInfo> = HashMap::new();
    for (channel, version, checksum) in rows {
        channels_map.insert(channel, ChannelInfo { version, checksum });
    }

    let response = crate::models::release::ReleasesResponse { channels: channels_map };
    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /worker/releases/{channel}/download/{filename} — Download release asset
pub async fn download_release(
    State(state): State<AppState>,
    Path((channel, filename)): Path<(String, String)>,
) -> Result<Response, AppError> {
    // Validate channel
    let valid_channels = ["latest", "next", "enterprise-latest", "enterprise-next"];
    if !valid_channels.contains(&channel.as_str()) {
        return Err(AppError::NotFound(format!("Unknown release channel: {}", channel)));
    }

    // Try to get from S3 first
    match state.cas_store.get_release(&channel, &filename).await {
        Ok(Some(data)) => {
            let content_type = guess_content_type(&filename);
            let mut response = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .header(header::CONTENT_LENGTH, data.len())
                .header(header::CACHE_CONTROL, "public, max-age=3600");

            // For install scripts, set executable content type
            if filename.ends_with(".sh") {
                response = response.header(header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8");
            } else if filename.ends_with(".ps1") {
                response = response.header(header::CONTENT_TYPE, "text/plain; charset=utf-8");
            }

            return Ok(response.body(Body::from(data)).map_err(|e| {
                AppError::Internal(format!("Failed to build response: {}", e))
            })?);
        }
        Ok(None) => {
            // Try to serve from DB metadata (for small files like SHA256SUMS)
            let row: Option<(String, Option<i64>)> = sqlx::query_as(
                "SELECT sha256, size_bytes FROM release_assets WHERE channel = $1 AND filename = $2"
            )
            .bind(&channel)
            .bind(&filename)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?;

            match row {
                Some(_) => {
                    // Asset exists in DB but not in S3 yet
                    Err(AppError::NotFound("Release asset not yet uploaded".into()))
                }
                None => Err(AppError::NotFound(format!(
                    "Release asset not found: {}/{}", channel, filename
                ))),
            }
        }
        Err(e) => {
            tracing::warn!("S3 release download failed: {}", e);
            Err(AppError::NotFound(format!(
                "Release asset not found: {}/{}", channel, filename
            )))
        }
    }
}

/// GET /worker/config/feature-flags — Get feature flags
pub async fn get_feature_flags(
    State(state): State<AppState>,
) -> Json<Value> {
    // Try to load from DB first
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT key, value FROM feature_flags"
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    if !rows.is_empty() {
        let flags: serde_json::Map<String, Value> = rows.into_iter().map(|(k, v)| (k, v)).collect();
        return Json(Value::Object(flags));
    }

    // Fall back to defaults
    Json(serde_json::to_value(FeatureFlagsResponse::default()).unwrap())
}

/// POST /api/admin/releases/channel — Create or update a release channel
#[derive(Debug, Deserialize)]
pub struct UpdateChannelRequest {
    pub channel: String,
    pub version: String,
    pub checksum: String,
}

pub async fn update_release_channel(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Json(req): Json<UpdateChannelRequest>,
) -> Result<Json<Value>, AppError> {
    sqlx::query(
        r#"INSERT INTO release_channels (channel, version, checksum)
        VALUES ($1, $2, $3)
        ON CONFLICT (channel) DO UPDATE SET
            version = EXCLUDED.version,
            checksum = EXCLUDED.checksum,
            updated_at = now()"#
    )
    .bind(&req.channel)
    .bind(&req.version)
    .bind(&req.checksum)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    tracing::info!("Release channel updated: {}={}", req.channel, req.version);

    Ok(Json(json!({
        "success": true,
        "channel": req.channel,
        "version": req.version,
    })))
}

/// POST /api/admin/releases/upload — Upload a release asset
pub async fn upload_release_asset(
    State(state): State<AppState>,
    _auth: AdminGuard,
    multipart: Multipart,
) -> Result<Json<Value>, AppError> {
    let mut channel = String::new();
    let mut filename = String::new();
    let mut sha256 = String::new();
    let mut data: Option<Vec<u8>> = None;

    let mut multipart = multipart;
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        AppError::BadRequest(format!("Multipart error: {}", e))
    })? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "channel" => {
                channel = field.text().await.map_err(|e| AppError::BadRequest(format!("Field error: {}", e)))?;
            }
            "filename" => {
                filename = field.text().await.map_err(|e| AppError::BadRequest(format!("Field error: {}", e)))?;
            }
            "sha256" => {
                sha256 = field.text().await.map_err(|e| AppError::BadRequest(format!("Field error: {}", e)))?;
            }
            "file" => {
                data = Some(field.bytes().await.map_err(|e| AppError::BadRequest(format!("File error: {}", e)))?.to_vec());
            }
            _ => {}
        }
    }

    if channel.is_empty() || filename.is_empty() {
        return Err(AppError::BadRequest("channel and filename are required".into()));
    }

    let data = data.ok_or_else(|| AppError::BadRequest("file is required".into()))?;
    let size_bytes = data.len() as i64;

    // Compute SHA256 if not provided
    if sha256.is_empty() {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(&data);
        sha256 = hex::encode(hasher.finalize());
    }

    // Store in S3
    state.cas_store.put_release(&channel, &filename, &data).await?;

    // Store metadata in DB
    sqlx::query(
        r#"INSERT INTO release_assets (channel, filename, sha256, size_bytes, storage_path)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (channel, filename) DO UPDATE SET
            sha256 = EXCLUDED.sha256,
            size_bytes = EXCLUDED.size_bytes,
            storage_path = EXCLUDED.storage_path"#
    )
    .bind(&channel)
    .bind(&filename)
    .bind(&sha256)
    .bind(size_bytes)
    .bind(format!("releases/{}/{}", channel, filename))
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    tracing::info!("Release asset uploaded: {}/{} ({} bytes)", channel, filename, size_bytes);

    Ok(Json(json!({
        "success": true,
        "channel": channel,
        "filename": filename,
        "sha256": sha256,
        "size_bytes": size_bytes,
    })))
}

/// GET /api/admin/releases/assets — List all release assets
pub async fn list_release_assets(
    State(state): State<AppState>,
    _auth: AdminGuard,
) -> Result<Json<Value>, AppError> {
    let rows: Vec<(String, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT channel, filename, sha256, size_bytes FROM release_assets ORDER BY channel, filename"
    )
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let assets: Vec<Value> = rows.iter().map(|(channel, filename, sha256, size)| {
        json!({
            "channel": channel,
            "filename": filename,
            "sha256": sha256,
            "size_bytes": size,
        })
    }).collect();

    Ok(Json(json!({ "assets": assets })))
}

/// DELETE /api/admin/releases/assets/{channel}/{filename} — Delete a release asset
pub async fn delete_release_asset(
    State(state): State<AppState>,
    _auth: AdminGuard,
    Path((channel, filename)): Path<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let result = sqlx::query(
        "DELETE FROM release_assets WHERE channel = $1 AND filename = $2"
    )
    .bind(&channel)
    .bind(&filename)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Release asset not found".into()));
    }

    Ok(Json(json!({ "success": true })))
}

fn guess_content_type(filename: &str) -> &'static str {
    if filename.ends_with(".exe") {
        "application/octet-stream"
    } else if filename.ends_with(".sh") {
        "text/x-shellscript; charset=utf-8"
    } else if filename.ends_with(".ps1") {
        "text/plain; charset=utf-8"
    } else if filename.ends_with("SHA256SUMS") {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}
