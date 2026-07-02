use axum::extract::{Query, State};
use axum::response::Json;
use serde::Deserialize;
use serde_json::Value;

use crate::auth::middleware::{AuthExtractor, HeaderExtractor};
use crate::error::AppError;
use crate::models::cas::CasUploadRequest;
use crate::pos_encoded::validate_hex_hash;
use crate::routes::AppState;

/// POST /worker/cas/upload — Batch upload CAS objects
pub async fn upload_cas(
    State(state): State<AppState>,
    auth: AuthExtractor,
    headers: HeaderExtractor,
    Json(req): Json<CasUploadRequest>,
) -> Result<Json<Value>, AppError> {
    tracing::info!(
        "CAS upload: {} objects, author_identity={:?}",
        req.objects.len(),
        headers.0.author_identity,
    );

    let mut results = Vec::new();
    let mut success_count = 0i64;
    let mut failure_count = 0i64;

    for object in &req.objects {
        match process_cas_object(&state, object, &auth.0, &headers.0).await {
            Ok(()) => {
                results.push(serde_json::json!({
                    "hash": object.hash,
                    "status": "ok",
                }));
                success_count += 1;
            }
            Err(e) => {
                tracing::warn!("CAS upload failed for hash {}: {}", object.hash, e);
                results.push(serde_json::json!({
                    "hash": object.hash,
                    "status": "error",
                    "error": e.to_string(),
                }));
                failure_count += 1;
            }
        }
    }

    Ok(Json(serde_json::json!({
        "results": results,
        "success_count": success_count,
        "failure_count": failure_count,
    })))
}

async fn process_cas_object(
    state: &AppState,
    object: &crate::models::cas::CasObject,
    identity: &crate::models::user::AuthIdentity,
    headers: &crate::models::user::RequestHeaders,
) -> Result<(), AppError> {
    validate_hex_hash(&object.hash)?;

    let content_json = serde_json::to_value(&object.content)
        .map_err(|e| AppError::BadRequest(format!("Invalid content JSON: {}", e)))?;

    // Server-side secrets detection (defense in depth)
    let scan_result = crate::services::secrets::scan_json_for_secrets(&content_json);
    if scan_result.secrets_found > 0 {
        tracing::warn!(
            "CAS upload contains {} potential secret(s): hash={} detections={:?}",
            scan_result.secrets_found, object.hash,
            scan_result.detections.iter().map(|(p, v)| format!("{}={}", p, v)).collect::<Vec<_>>()
        );
        // Log to audit trail
        crate::services::audit::log_action(
            &state.db, Some(identity.user_id), identity.org_id,
            "cas.secret_detected", Some("cas_object"), Some(&object.hash),
            Some(serde_json::json!({
                "secrets_found": scan_result.secrets_found,
                "detections": scan_result.detections.iter().take(5).map(|(p, v)| serde_json::json!({"path": p, "preview": v})).collect::<Vec<_>>(),
            })),
            None, None,
        ).await.ok();
    }

    let metadata_json = if object.metadata.is_empty() {
        None
    } else {
        Some(serde_json::to_value(&object.metadata).unwrap())
    };

    let content_str = serde_json::to_string(&object.content)
        .map_err(|e| AppError::Internal(format!("Failed to serialize content: {}", e)))?;

    // Upsert: insert if not exists (idempotent)
    sqlx::query(
        r#"INSERT INTO cas_objects (hash, content, metadata, author_identity, user_id, org_id, size_bytes)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (hash) DO NOTHING"#
    )
    .bind(&object.hash)
    .bind(&content_json)
    .bind(&metadata_json)
    .bind(&headers.author_identity)
    .bind(identity.user_id)
    .bind(identity.org_id)
    .bind(content_str.len() as i32)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Store content in S3
    state.cas_store.put(&object.hash, content_str.as_bytes()).await?;

    // Record ownership
    sqlx::query(
        r#"INSERT INTO cas_ownership (hash, user_id, org_id)
        VALUES ($1, $2, $3)
        ON CONFLICT (hash, user_id) DO NOTHING"#
    )
    .bind(&object.hash)
    .bind(identity.user_id)
    .bind(identity.org_id)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct CasReadQuery {
    pub hashes: String,
}

/// GET /worker/cas/?hashes=... — Batch read CAS objects
pub async fn read_cas(
    State(state): State<AppState>,
    auth: AuthExtractor,
    Query(query): Query<CasReadQuery>,
) -> Result<Json<Value>, AppError> {
    let hashes: Vec<&str> = query
        .hashes
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    if hashes.len() > 100 {
        return Err(AppError::BadRequest("Maximum 100 hashes per request".into()));
    }

    for hash in &hashes {
        validate_hex_hash(hash)?;
    }

    tracing::info!("CAS read: {} hashes requested", hashes.len());

    let mut results = Vec::new();
    let mut success_count = 0i64;
    let mut failure_count = 0i64;

    for hash in &hashes {
        // Data isolation: admin sees all CAS objects within their org, non-admin sees only their own
        let row: Option<(serde_json::Value,)> = if auth.0.is_admin() {
            sqlx::query_as(
                "SELECT content FROM cas_objects WHERE hash = $1 AND ($2::uuid IS NULL OR org_id = $2)"
            )
            .bind(*hash)
            .bind(auth.0.org_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?
        } else {
            sqlx::query_as(
                "SELECT content FROM cas_objects WHERE hash = $1 AND user_id = $2 AND ($3::uuid IS NULL OR org_id = $3)"
            )
            .bind(*hash)
            .bind(auth.0.user_id)
            .bind(auth.0.org_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?
        };

        match row {
            Some((content,)) => {
                results.push(serde_json::json!({
                    "hash": hash,
                    "status": "ok",
                    "content": content,
                }));
                success_count += 1;
            }
            None => {
                // Fallback: try to read from S3 if not in DB
                match state.cas_store.get(*hash).await {
                    Ok(Some(data)) => {
                        match serde_json::from_slice::<serde_json::Value>(&data) {
                            Ok(content) => {
                                results.push(serde_json::json!({
                                    "hash": hash,
                                    "status": "ok",
                                    "content": content,
                                }));
                                success_count += 1;
                            }
                            Err(_) => {
                                let content_str = String::from_utf8_lossy(&data).to_string();
                                results.push(serde_json::json!({
                                    "hash": hash,
                                    "status": "ok",
                                    "content": content_str,
                                }));
                                success_count += 1;
                            }
                        }
                    }
                    Ok(None) => {
                        results.push(serde_json::json!({
                            "hash": hash,
                            "status": "error",
                            "error": "Not found",
                        }));
                        failure_count += 1;
                    }
                    Err(e) => {
                        tracing::warn!("CAS S3 fallback failed for hash {}: {}", hash, e);
                        results.push(serde_json::json!({
                            "hash": hash,
                            "status": "error",
                            "error": "Not found",
                        }));
                        failure_count += 1;
                    }
                }
            }
        }
    }

    // Log CAS access for audit (Phase 6)
    for hash in &hashes {
        crate::services::data_retention::log_cas_access(
            &state.db,
            Some(auth.0.user_id),
            auth.0.org_id,
            None,
            *hash,
            "api",
            None,
            None,
            None,
        ).await.ok();
    }

    Ok(Json(serde_json::json!({
        "results": results,
        "success_count": success_count,
        "failure_count": failure_count,
    })))
}
