//! Metrics service - handles decoding, storing, and querying metrics events

use sqlx::{PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

use crate::error::AppError;
use crate::models::metrics::{
    DecodedMetricEvent, MetricEvent, MetricUploadError, MetricsUploadResponse,
};
use crate::pos_encoded::decode_event;

const METRICS_INSERT_CHUNK_SIZE: usize = 500;

/// Process a batch of metrics events
pub async fn process_metrics_batch(
    pool: &PgPool,
    events: Vec<MetricEvent>,
    user_id: Option<Uuid>,
    org_id: Option<Uuid>,
    distinct_id: Option<String>,
) -> MetricsUploadResponse {
    let mut errors = Vec::new();
    let mut rows = Vec::new();

    for (idx, event) in events.iter().enumerate() {
        match decode_event(event) {
            Ok(decoded) => {
                match PreparedMetricRow::from_decoded(idx, &decoded, user_id, org_id, &distinct_id)
                {
                    Ok(row) => rows.push(row),
                    Err(e) => {
                        tracing::warn!("Failed to prepare metrics event at index {}: {}", idx, e);
                        errors.push(MetricUploadError {
                            index: idx,
                            error: format!("Storage error: {}", e),
                        });
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to decode metrics event at index {}: {}", idx, e);
                errors.push(MetricUploadError {
                    index: idx,
                    error: format!("Decode error: {}", e),
                });
            }
        }
    }

    for chunk in rows.chunks(METRICS_INSERT_CHUNK_SIZE) {
        if let Err(e) = insert_metrics_chunk(pool, chunk).await {
            tracing::warn!(
                "Failed to bulk insert metrics chunk with {} events: {}",
                chunk.len(),
                e
            );
            errors.extend(chunk.iter().map(|row| MetricUploadError {
                index: row.index,
                error: format!("Storage error: {}", e),
            }));
        }
    }

    errors.sort_by_key(|error| error.index);
    MetricsUploadResponse { errors }
}

#[derive(Debug)]
struct PreparedMetricRow {
    index: usize,
    event_type: i32,
    timestamp: i64,
    user_id: Option<Uuid>,
    distinct_id: Option<String>,
    org_id: Option<Uuid>,
    repo_url: Option<String>,
    author: Option<String>,
    tool: Option<String>,
    model: Option<String>,
    commit_sha: Option<String>,
    human_additions: Option<i32>,
    ai_additions_total: i32,
    mixed_additions_total: i32,
    unknown_additions: i32,
    ai_accepted_total: i32,
    git_diff_added_lines: Option<i32>,
    git_diff_deleted_lines: Option<i32>,
    tool_model_pairs_json: Option<serde_json::Value>,
    ai_additions_by_tool_json: Option<serde_json::Value>,
    prompt_id: Option<String>,
    session_id: Option<String>,
    file_path: Option<String>,
    custom_attrs_json: Option<serde_json::Value>,
    raw_values_json: serde_json::Value,
    raw_attrs_json: serde_json::Value,
}

impl PreparedMetricRow {
    fn from_decoded(
        index: usize,
        event: &DecodedMetricEvent,
        user_id: Option<Uuid>,
        org_id: Option<Uuid>,
        distinct_id: &Option<String>,
    ) -> Result<Self, AppError> {
        let ai_additions_total = aggregate_rollup(
            event.ai_additions.as_deref(),
            event.tool_model_pairs.as_deref(),
        );
        let mixed_additions_total = aggregate_rollup(
            event.mixed_additions.as_deref(),
            event.tool_model_pairs.as_deref(),
        );
        let ai_accepted_total = aggregate_rollup(
            event.ai_accepted.as_deref(),
            event.tool_model_pairs.as_deref(),
        );
        let unknown_additions = aggregate_unknown_additions(
            event.git_diff_added_lines,
            ai_additions_total,
            event.human_additions,
        );
        let ai_additions_by_tool_json = aggregate_by_tool(
            event.ai_additions.as_deref(),
            event.tool_model_pairs.as_deref(),
        );

        let raw_values_json = serde_json::to_value(&event.raw_values)
            .map_err(|e| AppError::Internal(format!("Failed to serialize raw_values: {}", e)))?;
        let raw_attrs_json = serde_json::to_value(&event.raw_attrs)
            .map_err(|e| AppError::Internal(format!("Failed to serialize raw_attrs: {}", e)))?;

        let tool_model_pairs_json = event
            .tool_model_pairs
            .as_ref()
            .map(|v| serde_json::to_value(v).unwrap_or(serde_json::Value::Null));

        let custom_attrs_json = event
            .custom_attributes
            .as_ref()
            .map(|v| serde_json::to_value(v).unwrap_or(serde_json::Value::Null));

        let effective_distinct_id = event.distinct_id.as_ref().or(distinct_id.as_ref()).cloned();

        Ok(Self {
            index,
            event_type: event.event_type as i32,
            timestamp: event.timestamp,
            user_id,
            distinct_id: effective_distinct_id,
            org_id,
            repo_url: event.repo_url.clone(),
            author: event.author.clone(),
            tool: event.tool.clone(),
            model: event.model.clone(),
            commit_sha: event.commit_sha.clone(),
            human_additions: event.human_additions,
            ai_additions_total,
            mixed_additions_total,
            unknown_additions,
            ai_accepted_total,
            git_diff_added_lines: event.git_diff_added_lines,
            git_diff_deleted_lines: event.git_diff_deleted_lines,
            tool_model_pairs_json,
            ai_additions_by_tool_json,
            prompt_id: event.prompt_id.clone(),
            session_id: event.session_id.clone(),
            file_path: event.file_path.clone(),
            custom_attrs_json,
            raw_values_json,
            raw_attrs_json,
        })
    }
}

async fn insert_metrics_chunk(pool: &PgPool, rows: &[PreparedMetricRow]) -> Result<(), AppError> {
    if rows.is_empty() {
        return Ok(());
    }

    let mut builder: QueryBuilder<Postgres> = QueryBuilder::new(
        r#"INSERT INTO metrics_events (
            event_type, timestamp, user_id, distinct_id, org_id,
            repo_url, author_email, tool, model, commit_sha,
            human_additions, ai_additions, mixed_additions,
            unknown_additions, ai_accepted,
            git_diff_added_lines, git_diff_deleted_lines,
            tool_model_pairs, ai_additions_by_tool,
            prompt_id, session_id, file_path,
            custom_attributes, raw_values, raw_attrs
        ) "#,
    );

    builder.push_values(rows, |mut row_builder, row| {
        row_builder
            .push_bind(row.event_type)
            .push_bind(row.timestamp)
            .push_bind(row.user_id)
            .push_bind(row.distinct_id.as_deref())
            .push_bind(row.org_id)
            .push_bind(row.repo_url.as_deref())
            .push_bind(row.author.as_deref())
            .push_bind(row.tool.as_deref())
            .push_bind(row.model.as_deref())
            .push_bind(row.commit_sha.as_deref())
            .push_bind(row.human_additions)
            .push_bind(row.ai_additions_total)
            .push_bind(row.mixed_additions_total)
            .push_bind(row.unknown_additions)
            .push_bind(row.ai_accepted_total)
            .push_bind(row.git_diff_added_lines)
            .push_bind(row.git_diff_deleted_lines)
            .push_bind(&row.tool_model_pairs_json)
            .push_bind(&row.ai_additions_by_tool_json)
            .push_bind(row.prompt_id.as_deref())
            .push_bind(row.session_id.as_deref())
            .push_bind(row.file_path.as_deref())
            .push_bind(&row.custom_attrs_json)
            .push_bind(&row.raw_values_json)
            .push_bind(&row.raw_attrs_json);
    });

    builder
        .build()
        .execute(pool)
        .await
        .map_err(AppError::Database)?;

    Ok(())
}

fn aggregate_rollup(values: Option<&[i32]>, tool_model_pairs: Option<&[String]>) -> i32 {
    let Some(values) = values else {
        return 0;
    };

    if let Some(tool_model_pairs) = tool_model_pairs {
        if let Some((idx, _)) = tool_model_pairs
            .iter()
            .enumerate()
            .find(|(_, pair)| pair.as_str() == "all")
        {
            if let Some(total) = values.get(idx) {
                return *total;
            }
        }
    }

    values.iter().sum()
}

fn aggregate_unknown_additions(
    git_diff_added_lines: Option<i32>,
    ai_additions: i32,
    human_additions: Option<i32>,
) -> i32 {
    git_diff_added_lines
        .unwrap_or(0)
        .saturating_sub(ai_additions)
        .saturating_sub(human_additions.unwrap_or(0))
        .max(0)
}

fn aggregate_by_tool(
    values: Option<&[i32]>,
    tool_model_pairs: Option<&[String]>,
) -> Option<serde_json::Value> {
    let values = values?;
    let pairs = tool_model_pairs?;

    let mut map = serde_json::Map::new();
    for (idx, pair) in pairs.iter().enumerate() {
        if pair == "all" {
            continue;
        }

        if let Some(value) = values.get(idx) {
            map.insert(pair.clone(), serde_json::json!(value));
        }
    }

    if map.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use std::collections::HashMap;

    #[test]
    fn aggregate_rollup_prefers_all_rollup() {
        let additions = [264, 264];
        let pairs = vec!["all".to_string(), "codex::gpt-5.5".to_string()];

        assert_eq!(aggregate_rollup(Some(&additions), Some(&pairs)), 264);
    }

    #[test]
    fn aggregate_rollup_sums_when_no_all_rollup_exists() {
        let additions = [120, 80];
        let pairs = vec![
            "codex::gpt-5.5".to_string(),
            "cursor::claude-sonnet".to_string(),
        ];

        assert_eq!(aggregate_rollup(Some(&additions), Some(&pairs)), 200);
    }

    #[test]
    fn aggregate_rollup_falls_back_to_sum_when_all_has_no_matching_value() {
        let additions = [120];
        let pairs = vec!["codex::gpt-5.5".to_string(), "all".to_string()];

        assert_eq!(aggregate_rollup(Some(&additions), Some(&pairs)), 120);
    }

    #[test]
    fn aggregate_rollup_defaults_to_zero_without_values() {
        assert_eq!(aggregate_rollup(None, None), 0);
    }

    #[test]
    fn aggregate_unknown_additions_counts_non_ai_unattributed_lines() {
        assert_eq!(aggregate_unknown_additions(Some(267), 264, Some(0)), 3);
    }

    #[test]
    fn aggregate_unknown_additions_never_goes_negative() {
        assert_eq!(aggregate_unknown_additions(Some(10), 12, Some(1)), 0);
    }

    #[test]
    fn aggregate_by_tool_skips_all_rollup() {
        let additions = [264, 120, 144];
        let pairs = vec![
            "all".to_string(),
            "codex::gpt-5.5".to_string(),
            "cursor::claude-sonnet".to_string(),
        ];

        assert_eq!(
            aggregate_by_tool(Some(&additions), Some(&pairs)),
            Some(serde_json::json!({
                "codex::gpt-5.5": 120,
                "cursor::claude-sonnet": 144,
            }))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn process_metrics_batch_uses_supplied_org_id() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.pool).await?;

        let response = process_metrics_batch(
            &db.pool,
            vec![committed_metric_event()],
            Some(user_id),
            Some(org_id),
            Some("metrics-test-device".into()),
        )
        .await;

        assert!(response.errors.is_empty());
        let stored_org_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT org_id FROM metrics_events WHERE user_id = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(user_id)
        .fetch_one(&db.pool)
        .await?;
        assert_eq!(stored_org_id, Some(org_id));

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn process_metrics_batch_preserves_null_org_id_when_not_supplied() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, _org_id) = insert_test_identity(&db.pool).await?;

        let response = process_metrics_batch(
            &db.pool,
            vec![committed_metric_event()],
            Some(user_id),
            None,
            Some("metrics-test-device".into()),
        )
        .await;

        assert!(response.errors.is_empty());
        let stored_org_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT org_id FROM metrics_events WHERE user_id = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(user_id)
        .fetch_one(&db.pool)
        .await?;
        assert_eq!(stored_org_id, None);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn process_metrics_batch_bulk_inserts_all_valid_events() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.pool).await?;
        let events: Vec<MetricEvent> = (0..10).map(committed_metric_event_with_seed).collect();

        let response = process_metrics_batch(
            &db.pool,
            events,
            Some(user_id),
            Some(org_id),
            Some("metrics-test-device".into()),
        )
        .await;

        assert!(response.errors.is_empty());
        assert_eq!(metrics_count(&db.pool).await?, 10);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn process_metrics_batch_splits_large_batches_into_chunks() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.pool).await?;
        let events: Vec<MetricEvent> = (0..=METRICS_INSERT_CHUNK_SIZE)
            .map(committed_metric_event_with_seed)
            .collect();

        let response = process_metrics_batch(
            &db.pool,
            events,
            Some(user_id),
            Some(org_id),
            Some("metrics-test-device".into()),
        )
        .await;

        assert!(response.errors.is_empty());
        assert_eq!(metrics_count(&db.pool).await?, 501);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn process_metrics_batch_keeps_partial_success_when_decode_fails() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.pool).await?;

        let response = process_metrics_batch(
            &db.pool,
            vec![
                committed_metric_event_with_seed(1),
                invalid_metric_event(),
                committed_metric_event_with_seed(2),
            ],
            Some(user_id),
            Some(org_id),
            Some("metrics-test-device".into()),
        )
        .await;

        assert_eq!(response.errors.len(), 1);
        assert_eq!(response.errors[0].index, 1);
        assert!(response.errors[0].error.starts_with("Decode error:"));
        assert_eq!(metrics_count(&db.pool).await?, 2);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn process_metrics_batch_reports_storage_errors_for_failed_chunk() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let missing_org_id = Uuid::new_v4();

        let response = process_metrics_batch(
            &db.pool,
            vec![
                committed_metric_event_with_seed(1),
                committed_metric_event_with_seed(2),
            ],
            None,
            Some(missing_org_id),
            Some("metrics-test-device".into()),
        )
        .await;

        assert_eq!(response.errors.len(), 2);
        assert_eq!(response.errors[0].index, 0);
        assert_eq!(response.errors[1].index, 1);
        assert!(
            response
                .errors
                .iter()
                .all(|error| error.error.starts_with("Storage error:"))
        );
        assert_eq!(metrics_count(&db.pool).await?, 0);

        db.cleanup().await?;
        Ok(())
    }

    struct TestDatabase {
        pool: PgPool,
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
                        "skipping metrics database test: could not connect to admin database: {error}"
                    );
                    return Ok(None);
                }
            };

            if let Err(error) = create_database(&admin_pool, &db_name).await {
                eprintln!(
                    "skipping metrics database test: could not create isolated database {db_name}: {error}"
                );
                admin_pool.close().await;
                return Ok(None);
            }

            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(&test_url)
                .await?;
            crate::db::run_migrations(&pool).await?;

            Ok(Some(Self {
                pool,
                admin_pool,
                db_name,
            }))
        }

        async fn cleanup(self) -> anyhow::Result<()> {
            self.pool.close().await;
            drop_database(&self.admin_pool, &self.db_name).await?;
            self.admin_pool.close().await;
            Ok(())
        }
    }

    async fn insert_test_identity(pool: &PgPool) -> anyhow::Result<(Uuid, Uuid)> {
        let user_id = Uuid::new_v4();
        let org_id = Uuid::new_v4();

        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org_id)
            .bind("Metrics Test Org")
            .bind(format!("metrics-test-{}", org_id.simple()))
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO users (id, email, name, default_org_id) VALUES ($1, $2, $3, $4)")
            .bind(user_id)
            .bind(format!("{user_id}@example.com"))
            .bind("Metrics Test User")
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

    fn committed_metric_event() -> MetricEvent {
        committed_metric_event_with_seed(0)
    }

    fn committed_metric_event_with_seed(seed: usize) -> MetricEvent {
        let mut values = HashMap::new();
        values.insert("0".into(), serde_json::json!(10));
        values.insert("1".into(), serde_json::json!(2));
        values.insert("2".into(), serde_json::json!(30));
        values.insert("3".into(), serde_json::json!(["all", "codex::gpt-5"]));
        values.insert("5".into(), serde_json::json!([20, 5]));

        let mut attrs = HashMap::new();
        attrs.insert("0".into(), serde_json::json!("1.3.2"));
        attrs.insert(
            "1".into(),
            serde_json::json!(format!("https://example.com/repo-{seed}.git")),
        );
        attrs.insert("2".into(), serde_json::json!("dev@example.com"));
        attrs.insert("3".into(), serde_json::json!(format!("abc{seed}")));

        MetricEvent {
            t: 1_700_000_000,
            e: 1,
            v: values,
            a: attrs,
        }
    }

    fn invalid_metric_event() -> MetricEvent {
        MetricEvent {
            t: 1_700_000_000,
            e: 999,
            v: HashMap::new(),
            a: HashMap::new(),
        }
    }

    async fn metrics_count(pool: &PgPool) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM metrics_events")
            .fetch_one(pool)
            .await?)
    }

    fn test_database_url() -> String {
        dotenvy::dotenv().ok();
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://gitai:gitai@localhost:5433/gitai_enterprise".into())
    }

    fn unique_test_database_name() -> String {
        format!("git_ai_metrics_test_{}", Uuid::new_v4().simple())
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
