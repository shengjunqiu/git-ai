use axum::extract::{Query, State};
use axum::response::Json;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::task::JoinSet;

use crate::auth::middleware::{AuthExtractor, HeaderExtractor};
use crate::error::AppError;
use crate::models::cas::{CasObject, CasUploadRequest};
use crate::models::user::{AuthIdentity, RequestHeaders};
use crate::pos_encoded::validate_hex_hash;
use crate::routes::AppState;

const MAX_CAS_UPLOAD_OBJECTS: usize = 100;

/// POST /worker/cas/upload — Batch upload CAS objects
pub async fn upload_cas(
    State(state): State<AppState>,
    auth: AuthExtractor,
    headers: HeaderExtractor,
    Json(req): Json<CasUploadRequest>,
) -> Result<Json<Value>, AppError> {
    validate_cas_batch_size(req.objects.len())?;
    let objects = prepare_cas_objects(req.objects)?;

    tracing::info!(
        "CAS upload: {} objects, author_identity={:?}",
        objects.len(),
        headers.0.author_identity,
    );

    let response = process_cas_uploads(
        &state,
        auth.0,
        headers.0,
        objects,
        state.config.cas_upload_concurrency,
    )
    .await?;

    Ok(Json(response))
}

fn prepare_cas_objects(objects: Vec<CasObject>) -> Result<Vec<PreparedCasObject>, AppError> {
    objects.into_iter().map(PreparedCasObject::new).collect()
}

async fn process_cas_uploads(
    state: &AppState,
    identity: AuthIdentity,
    headers: RequestHeaders,
    objects: Vec<PreparedCasObject>,
    concurrency: usize,
) -> Result<Value, AppError> {
    let concurrency = concurrency.max(1);
    let mut object_iter = objects.into_iter().enumerate();
    let mut join_set = JoinSet::new();
    let mut completed = Vec::new();

    while join_set.len() < concurrency {
        let Some((index, object)) = object_iter.next() else {
            break;
        };
        spawn_cas_upload_task(
            &mut join_set,
            state.clone(),
            identity.clone(),
            headers.clone(),
            index,
            object,
        );
    }

    while let Some(join_result) = join_set.join_next().await {
        let task_result = join_result
            .map_err(|e| AppError::Internal(format!("CAS upload task failed: {}", e)))?;
        completed.push(task_result);

        if let Some((index, object)) = object_iter.next() {
            spawn_cas_upload_task(
                &mut join_set,
                state.clone(),
                identity.clone(),
                headers.clone(),
                index,
                object,
            );
        }
    }

    completed.sort_by_key(|result| result.index);

    let mut results = Vec::new();
    let mut success_count = 0i64;
    let mut failure_count = 0i64;

    for task_result in completed {
        match task_result.result {
            Ok(()) => {
                results.push(serde_json::json!({
                    "hash": task_result.hash,
                    "status": "ok",
                }));
                success_count += 1;
            }
            Err(e) => {
                tracing::warn!("CAS upload failed for hash {}: {}", task_result.hash, e);
                results.push(serde_json::json!({
                    "hash": task_result.hash,
                    "status": "error",
                    "error": e.to_string(),
                }));
                failure_count += 1;
            }
        }
    }

    Ok(serde_json::json!({
        "results": results,
        "success_count": success_count,
        "failure_count": failure_count,
    }))
}

fn spawn_cas_upload_task(
    join_set: &mut JoinSet<CasUploadTaskResult>,
    state: AppState,
    identity: AuthIdentity,
    headers: RequestHeaders,
    index: usize,
    object: PreparedCasObject,
) {
    join_set.spawn(async move {
        let hash = object.hash.clone();
        let result = process_prepared_cas_object(&state, &object, &identity, &headers).await;

        CasUploadTaskResult {
            index,
            hash,
            result,
        }
    });
}

fn validate_cas_batch_size(object_count: usize) -> Result<(), AppError> {
    if object_count > MAX_CAS_UPLOAD_OBJECTS {
        return Err(AppError::BadRequest(format!(
            "Maximum {} CAS objects per batch",
            MAX_CAS_UPLOAD_OBJECTS
        )));
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct PreparedCasObject {
    hash: String,
    content_json: serde_json::Value,
    content_str: String,
    metadata_json: Option<serde_json::Value>,
}

impl PreparedCasObject {
    fn new(object: CasObject) -> Result<Self, AppError> {
        validate_hex_hash(&object.hash)?;

        let content_str = canonical_content_string(&object.content)?;
        let content_hash = sha256_hex(content_str.as_bytes());

        if object.hash != content_hash {
            return Err(AppError::BadRequest(format!(
                "CAS hash mismatch: expected {}, got {}",
                content_hash, object.hash
            )));
        }

        let metadata_json = if object.metadata.is_empty() {
            None
        } else {
            Some(serde_json::to_value(&object.metadata)?)
        };

        Ok(Self {
            hash: object.hash,
            content_json: object.content,
            content_str,
            metadata_json,
        })
    }
}

struct CasUploadTaskResult {
    index: usize,
    hash: String,
    result: Result<(), AppError>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::middleware::HeaderExtractor;
    use crate::models::cas::CasObject;
    use crate::models::user::{AuthIdentity, AuthMethod, RequestHeaders};
    use object_store::local::LocalFileSystem;
    use sqlx::PgPool;
    use sqlx::postgres::PgPoolOptions;
    use std::collections::HashMap;
    use std::sync::Arc;
    use uuid::Uuid;

    #[test]
    fn validates_cas_batch_size() {
        assert!(validate_cas_batch_size(MAX_CAS_UPLOAD_OBJECTS).is_ok());
        assert!(matches!(
            validate_cas_batch_size(MAX_CAS_UPLOAD_OBJECTS + 1),
            Err(AppError::BadRequest(_))
        ));
    }

    struct TestDatabase {
        state: AppState,
        admin_pool: PgPool,
        db_name: String,
    }

    impl TestDatabase {
        async fn new(cas_store: crate::services::cas::CasStore) -> anyhow::Result<Option<Self>> {
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
                    eprintln!("skipping CAS test: could not connect to admin database: {error}");
                    return Ok(None);
                }
            };

            if let Err(error) = create_database(&admin_pool, &db_name).await {
                eprintln!(
                    "skipping CAS test: could not create isolated database {db_name}: {error}"
                );
                return Ok(None);
            }

            let pool = PgPoolOptions::new()
                .max_connections(6)
                .connect(&test_url)
                .await?;
            crate::db::run_migrations(&pool).await?;

            let config = test_config(&test_url);
            let redis = redis::Client::open(config.redis_url.clone())?;
            let state = AppState {
                db: pool,
                redis,
                config,
                cas_store,
                rate_limiter: crate::services::rate_limit::RateLimiter::new(),
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
    async fn upload_cas_accepts_matching_hash_and_reads_from_db() -> anyhow::Result<()> {
        let object_store_dir = tempfile::tempdir()?;
        let Some(db) = TestDatabase::new(local_cas_store(object_store_dir.path())?).await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let object = cas_object(cas_content("valid"));
        let hash = object.hash.clone();

        let response = upload_object(&db.state, user_id, org_id, object).await?;
        assert_eq!(response.0["success_count"], 1);
        assert_eq!(response.0["failure_count"], 0);
        assert_eq!(table_count(&db.state.db, "cas_objects").await?, 1);
        assert_eq!(table_count(&db.state.db, "cas_ownership").await?, 1);
        assert!(db.state.cas_store.get(&hash).await?.is_some());

        let read_response = read_cas(
            State(db.state.clone()),
            auth_extractor(user_id, org_id),
            Query(CasReadQuery {
                hashes: hash.clone(),
            }),
        )
        .await?;
        assert_eq!(read_response.0["success_count"], 1);
        assert_eq!(read_response.0["results"][0]["hash"], hash);
        assert_eq!(read_response.0["results"][0]["status"], "ok");

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_cas_rejects_hash_mismatch() -> anyhow::Result<()> {
        let object_store_dir = tempfile::tempdir()?;
        let Some(db) = TestDatabase::new(local_cas_store(object_store_dir.path())?).await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let mut object = cas_object(cas_content("original"));
        object.content = cas_content("tampered");

        let result = upload_object(&db.state, user_id, org_id, object).await;

        assert!(matches!(result, Err(AppError::BadRequest(_))));
        assert_eq!(table_count(&db.state.db, "cas_objects").await?, 0);
        assert_eq!(table_count(&db.state.db, "cas_ownership").await?, 0);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn read_cas_does_not_fallback_to_object_store_without_db_record() -> anyhow::Result<()> {
        let object_store_dir = tempfile::tempdir()?;
        let Some(db) = TestDatabase::new(local_cas_store(object_store_dir.path())?).await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let object = cas_object(cas_content("s3-only"));
        let content = canonical_content_string(&object.content)?;
        db.state
            .cas_store
            .put(&object.hash, content.as_bytes())
            .await?;

        let read_response = read_cas(
            State(db.state.clone()),
            auth_extractor(user_id, org_id),
            Query(CasReadQuery {
                hashes: object.hash.clone(),
            }),
        )
        .await?;

        assert_eq!(read_response.0["success_count"], 0);
        assert_eq!(read_response.0["failure_count"], 1);
        assert_eq!(read_response.0["results"][0]["hash"], object.hash);
        assert_eq!(read_response.0["results"][0]["status"], "error");
        assert_eq!(read_response.0["results"][0]["error"], "Not found");

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_cas_object_store_failure_leaves_no_db_record() -> anyhow::Result<()> {
        let object_store_file = tempfile::NamedTempFile::new()?;
        let Some(db) = TestDatabase::new(local_cas_store(object_store_file.path())?).await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let object = cas_object(cas_content("store-failure"));

        let response = upload_object(&db.state, user_id, org_id, object).await?;

        assert_eq!(response.0["success_count"], 0);
        assert_eq!(response.0["failure_count"], 1);
        assert_eq!(table_count(&db.state.db, "cas_objects").await?, 0);
        assert_eq!(table_count(&db.state.db, "cas_ownership").await?, 0);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_cas_concurrent_same_hash_same_content_is_idempotent() -> anyhow::Result<()> {
        let object_store_dir = tempfile::tempdir()?;
        let Some(db) = TestDatabase::new(local_cas_store(object_store_dir.path())?).await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let object = cas_object(cas_content("concurrent-same"));

        let first = upload_object(&db.state, user_id, org_id, object.clone());
        let second = upload_object(&db.state, user_id, org_id, object);
        let (first_response, second_response) = tokio::join!(first, second);

        assert_eq!(first_response?.0["success_count"], 1);
        assert_eq!(second_response?.0["success_count"], 1);
        assert_eq!(table_count(&db.state.db, "cas_objects").await?, 1);
        assert_eq!(table_count(&db.state.db, "cas_ownership").await?, 1);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_cas_concurrent_same_hash_different_content_rejects_mismatch()
    -> anyhow::Result<()> {
        let object_store_dir = tempfile::tempdir()?;
        let Some(db) = TestDatabase::new(local_cas_store(object_store_dir.path())?).await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let valid = cas_object(cas_content("concurrent-valid"));
        let mut tampered = valid.clone();
        tampered.content = cas_content("concurrent-tampered");

        let first = upload_object(&db.state, user_id, org_id, valid);
        let second = upload_object(&db.state, user_id, org_id, tampered);
        let (first_response, second_response) = tokio::join!(first, second);

        let successes = [&first_response, &second_response]
            .iter()
            .filter(|result| {
                result
                    .as_ref()
                    .is_ok_and(|response| response.0["success_count"] == 1)
            })
            .count();
        let bad_requests = [first_response, second_response]
            .into_iter()
            .filter(|result| matches!(result, Err(AppError::BadRequest(_))))
            .count();

        assert_eq!(successes, 1);
        assert_eq!(bad_requests, 1);
        assert_eq!(table_count(&db.state.db, "cas_objects").await?, 1);
        assert_eq!(table_count(&db.state.db, "cas_ownership").await?, 1);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_cas_processes_batch_with_bounded_concurrency() -> anyhow::Result<()> {
        let object_store_dir = tempfile::tempdir()?;
        let Some(db) = TestDatabase::new(local_cas_store(object_store_dir.path())?).await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let objects: Vec<CasObject> = (0..10)
            .map(|idx| cas_object(cas_content(&format!("batch-{idx}"))))
            .collect();
        let expected_hashes: Vec<String> =
            objects.iter().map(|object| object.hash.clone()).collect();

        let response = upload_objects(&db.state, user_id, org_id, objects).await?;

        assert_eq!(response.0["success_count"], 10);
        assert_eq!(response.0["failure_count"], 0);
        assert_eq!(table_count(&db.state.db, "cas_objects").await?, 10);
        assert_eq!(table_count(&db.state.db, "cas_ownership").await?, 10);
        for (idx, expected_hash) in expected_hashes.iter().enumerate() {
            assert_eq!(response.0["results"][idx]["hash"], *expected_hash);
            assert_eq!(response.0["results"][idx]["status"], "ok");
        }

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn process_cas_uploads_with_concurrency_one_preserves_serial_result_order()
    -> anyhow::Result<()> {
        let object_store_dir = tempfile::tempdir()?;
        let Some(db) = TestDatabase::new(local_cas_store(object_store_dir.path())?).await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let objects: Vec<CasObject> = (0..4)
            .map(|idx| cas_object(cas_content(&format!("serial-{idx}"))))
            .collect();
        let expected_hashes: Vec<String> =
            objects.iter().map(|object| object.hash.clone()).collect();
        let prepared = prepare_cas_objects(objects)?;

        let response = process_cas_uploads(
            &db.state,
            auth_extractor(user_id, org_id).0,
            RequestHeaders::default(),
            prepared,
            1,
        )
        .await?;

        assert_eq!(response["success_count"], 4);
        assert_eq!(response["failure_count"], 0);
        for (idx, expected_hash) in expected_hashes.iter().enumerate() {
            assert_eq!(response["results"][idx]["hash"], *expected_hash);
            assert_eq!(response["results"][idx]["status"], "ok");
        }

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_cas_rejects_batch_hash_mismatch_before_writes() -> anyhow::Result<()> {
        let object_store_dir = tempfile::tempdir()?;
        let Some(db) = TestDatabase::new(local_cas_store(object_store_dir.path())?).await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let valid = cas_object(cas_content("valid-before-bad-request"));
        let mut tampered = cas_object(cas_content("tampered-before-bad-request"));
        tampered.content = cas_content("tampered-after-hash");

        let result =
            upload_objects(&db.state, user_id, org_id, vec![valid.clone(), tampered]).await;

        assert!(matches!(result, Err(AppError::BadRequest(_))));
        assert_eq!(table_count(&db.state.db, "cas_objects").await?, 0);
        assert_eq!(table_count(&db.state.db, "cas_ownership").await?, 0);
        assert!(db.state.cas_store.get(&valid.hash).await?.is_none());

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_cas_aggregates_partial_batch_failures() -> anyhow::Result<()> {
        let object_store_dir = tempfile::tempdir()?;
        let (success_object, failing_object) = cas_objects_with_distinct_prefixes();
        std::fs::create_dir_all(object_store_dir.path().join("cas"))?;
        std::fs::write(
            object_store_dir
                .path()
                .join("cas")
                .join(&failing_object.hash[..2]),
            b"not a directory",
        )?;

        let Some(db) = TestDatabase::new(local_cas_store(object_store_dir.path())?).await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;

        let response = upload_objects(
            &db.state,
            user_id,
            org_id,
            vec![success_object.clone(), failing_object.clone()],
        )
        .await?;

        assert_eq!(response.0["success_count"], 1);
        assert_eq!(response.0["failure_count"], 1);
        assert_eq!(response.0["results"][0]["hash"], success_object.hash);
        assert_eq!(response.0["results"][0]["status"], "ok");
        assert_eq!(response.0["results"][1]["hash"], failing_object.hash);
        assert_eq!(response.0["results"][1]["status"], "error");
        assert_eq!(table_count(&db.state.db, "cas_objects").await?, 1);
        assert_eq!(table_count(&db.state.db, "cas_ownership").await?, 1);

        db.cleanup().await?;
        Ok(())
    }

    async fn upload_object(
        state: &AppState,
        user_id: Uuid,
        org_id: Uuid,
        object: CasObject,
    ) -> Result<Json<Value>, AppError> {
        upload_objects(state, user_id, org_id, vec![object]).await
    }

    async fn upload_objects(
        state: &AppState,
        user_id: Uuid,
        org_id: Uuid,
        objects: Vec<CasObject>,
    ) -> Result<Json<Value>, AppError> {
        upload_cas(
            State(state.clone()),
            auth_extractor(user_id, org_id),
            HeaderExtractor(RequestHeaders::default()),
            Json(CasUploadRequest { objects }),
        )
        .await
    }

    fn cas_object(content: Value) -> CasObject {
        CasObject {
            hash: content_hash(&content),
            content,
            metadata: HashMap::new(),
        }
    }

    fn cas_content(seed: &str) -> Value {
        serde_json::json!({
            "agent_id": {
                "tool": "Codex",
                "id": seed,
                "model": "gpt-5",
            },
            "messages": [
                {
                    "role": "user",
                    "content": seed,
                }
            ],
            "total_additions": 1,
            "total_deletions": 0,
        })
    }

    fn cas_objects_with_distinct_prefixes() -> (CasObject, CasObject) {
        let first = cas_object(cas_content("partial-0"));

        for idx in 1.. {
            let candidate = cas_object(cas_content(&format!("partial-{idx}")));
            if first.hash[..2] != candidate.hash[..2] {
                return (first, candidate);
            }
        }

        unreachable!("sha256 prefixes should diverge for generated test objects")
    }

    fn content_hash(content: &Value) -> String {
        let canonical =
            canonical_content_string(content).expect("test content should canonicalize");
        sha256_hex(canonical.as_bytes())
    }

    fn local_cas_store(path: &std::path::Path) -> anyhow::Result<crate::services::cas::CasStore> {
        let store = LocalFileSystem::new_with_prefix(path)?;
        Ok(crate::services::cas::CasStore::from_object_store(
            Arc::new(store),
            "test-cas".into(),
        ))
    }

    async fn insert_test_identity(pool: &PgPool) -> anyhow::Result<(Uuid, Uuid)> {
        let user_id = Uuid::new_v4();
        let org_id = Uuid::new_v4();

        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org_id)
            .bind("CAS Test Org")
            .bind(format!("cas-test-{}", org_id.simple()))
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO users (id, email, name, default_org_id) VALUES ($1, $2, $3, $4)")
            .bind(user_id)
            .bind(format!("{user_id}@example.com"))
            .bind("CAS Test User")
            .bind(org_id)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO org_members (user_id, org_id, role) VALUES ($1, $2, $3)")
            .bind(user_id)
            .bind(org_id)
            .bind("member")
            .execute(pool)
            .await?;

        Ok((user_id, org_id))
    }

    fn auth_extractor(user_id: Uuid, org_id: Uuid) -> AuthExtractor {
        AuthExtractor(AuthIdentity {
            user_id,
            email: format!("{user_id}@example.com"),
            name: "CAS Test User".into(),
            org_id: Some(org_id),
            org_slug: Some(format!("cas-test-{}", org_id.simple())),
            department_id: None,
            role: Some("member".into()),
            scopes: vec!["cas:write".into(), "cas:read".into()],
            auth_method: AuthMethod::ApiKey,
        })
    }

    async fn table_count(pool: &PgPool, table: &str) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(pool)
            .await?)
    }

    fn test_config(database_url: &str) -> crate::config::AppConfig {
        crate::config::AppConfig {
            database_url: database_url.to_string(),
            database_max_connections: 20,
            database_min_connections: 1,
            database_acquire_timeout_seconds: 5,
            redis_url: "redis://127.0.0.1:6379".to_string(),
            jwt_secret: "cas-test-secret".to_string(),
            s3_endpoint: "http://localhost:9000".to_string(),
            s3_bucket: "git-ai-cas".to_string(),
            s3_access_key: "minioadmin".to_string(),
            s3_secret_key: "minioadmin".to_string(),
            s3_region: "us-east-1".to_string(),
            cas_upload_concurrency: 8,
            metrics_write_rollups: true,
            dashboard_use_rollups: false,
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
        format!("git_ai_cas_test_{}", Uuid::new_v4().simple())
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

async fn process_prepared_cas_object(
    state: &AppState,
    object: &PreparedCasObject,
    identity: &AuthIdentity,
    headers: &RequestHeaders,
) -> Result<(), AppError> {
    // Server-side secrets detection (defense in depth)
    let scan_result = crate::services::secrets::scan_json_for_secrets(&object.content_json);
    if scan_result.secrets_found > 0 {
        tracing::warn!(
            "CAS upload contains {} potential secret(s): hash={} detections={:?}",
            scan_result.secrets_found,
            object.hash,
            scan_result
                .detections
                .iter()
                .map(|(p, v)| format!("{}={}", p, v))
                .collect::<Vec<_>>()
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

    // Store content in S3 before marking it readable in Postgres.
    state
        .cas_store
        .put(&object.hash, object.content_str.as_bytes())
        .await?;

    let mut tx = state.db.begin().await.map_err(AppError::Database)?;

    // Upsert: insert if not exists (idempotent)
    sqlx::query(
        r#"INSERT INTO cas_objects (hash, content, metadata, author_identity, user_id, org_id, size_bytes)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (hash) DO NOTHING"#
    )
    .bind(&object.hash)
    .bind(&object.content_json)
    .bind(&object.metadata_json)
    .bind(&headers.author_identity)
    .bind(identity.user_id)
    .bind(identity.org_id)
    .bind(object.content_str.len() as i32)
    .execute(&mut *tx)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Record ownership
    sqlx::query(
        r#"INSERT INTO cas_ownership (hash, user_id, org_id)
        VALUES ($1, $2, $3)
        ON CONFLICT (hash, user_id) DO NOTHING"#,
    )
    .bind(&object.hash)
    .bind(identity.user_id)
    .bind(identity.org_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| AppError::Database(e))?;

    tx.commit().await.map_err(AppError::Database)?;

    Ok(())
}

fn canonical_content_string(content: &serde_json::Value) -> Result<String, AppError> {
    serde_json_canonicalizer::to_string(content)
        .map_err(|e| AppError::BadRequest(format!("Failed to canonicalize content JSON: {}", e)))
}

fn sha256_hex(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
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
        return Err(AppError::BadRequest(
            "Maximum 100 hashes per request".into(),
        ));
    }

    for hash in &hashes {
        validate_hex_hash(hash)?;
    }

    tracing::info!("CAS read: {} hashes requested", hashes.len());

    let mut results = Vec::new();
    let mut success_count = 0i64;
    let mut failure_count = 0i64;

    for hash in &hashes {
        // Data isolation: admin sees all CAS objects within their org, non-admin sees only their own.
        // Reads are served only from DB-authorized records; S3 is not a fallback authorization source.
        let row: Option<(serde_json::Value,)> = if auth.0.is_admin() {
            sqlx::query_as(
                "SELECT co.content \
                 FROM cas_objects co \
                 WHERE co.hash = $1 \
                   AND (\
                     $2::uuid IS NULL \
                     OR co.org_id = $2 \
                     OR EXISTS (\
                       SELECT 1 FROM cas_ownership own \
                       WHERE own.hash = co.hash AND own.org_id = $2\
                     )\
                   )",
            )
            .bind(*hash)
            .bind(auth.0.org_id)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?
        } else {
            sqlx::query_as(
                "SELECT co.content \
                 FROM cas_objects co \
                 WHERE co.hash = $1 \
                   AND (\
                     co.user_id = $2 \
                     OR EXISTS (\
                       SELECT 1 FROM cas_ownership own \
                       WHERE own.hash = co.hash \
                         AND own.user_id = $2 \
                         AND ($3::uuid IS NULL OR own.org_id = $3)\
                     )\
                   ) \
                   AND ($3::uuid IS NULL OR co.org_id = $3 OR EXISTS (\
                     SELECT 1 FROM cas_ownership own \
                     WHERE own.hash = co.hash AND own.user_id = $2 AND own.org_id = $3\
                   ))",
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
                results.push(serde_json::json!({
                    "hash": hash,
                    "status": "error",
                    "error": "Not found",
                }));
                failure_count += 1;
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
        )
        .await
        .ok();
    }

    Ok(Json(serde_json::json!({
        "results": results,
        "success_count": success_count,
        "failure_count": failure_count,
    })))
}
