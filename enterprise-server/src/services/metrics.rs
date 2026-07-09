//! Metrics service - handles decoding, storing, and querying metrics events

use sqlx::{PgPool, Postgres, QueryBuilder, Transaction};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
use uuid::Uuid;

use crate::config::MetricsRollupWriteMode;
use crate::error::AppError;
use crate::models::metrics::{
    DecodedMetricEvent, MetricEvent, MetricUploadError, MetricsUploadResponse,
};
use crate::pos_encoded::decode_event;

const METRICS_INSERT_CHUNK_SIZE: usize = 500;
const POSTGRES_MAX_BIND_PARAMETERS: usize = 65_535;
const METRICS_TOOL_MODEL_EVENT_BIND_COUNT: usize = 10;
const METRICS_TOOL_MODEL_INSERT_CHUNK_SIZE: usize = 5_000;
const _: () = assert!(
    METRICS_TOOL_MODEL_INSERT_CHUNK_SIZE * METRICS_TOOL_MODEL_EVENT_BIND_COUNT
        < POSTGRES_MAX_BIND_PARAMETERS
);

/// Process a batch of metrics events
pub async fn process_metrics_batch(
    pool: &PgPool,
    events: Vec<MetricEvent>,
    user_id: Option<Uuid>,
    org_id: Option<Uuid>,
    distinct_id: Option<String>,
    rollup_write_mode: MetricsRollupWriteMode,
) -> MetricsUploadResponse {
    let decode_started = Instant::now();
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

    let decode_prepare_ms = decode_started.elapsed().as_secs_f64() * 1000.0;
    let storage_started = Instant::now();
    let mut inserted_chunks = 0usize;
    let mut failed_chunks = 0usize;

    for chunk in rows.chunks(METRICS_INSERT_CHUNK_SIZE) {
        if let Err(e) = insert_metrics_chunk(pool, chunk, rollup_write_mode).await {
            failed_chunks += 1;
            tracing::warn!(
                "Failed to bulk insert metrics chunk with {} events: {}",
                chunk.len(),
                e
            );
            errors.extend(chunk.iter().map(|row| MetricUploadError {
                index: row.index,
                error: format!("Storage error: {}", e),
            }));
        } else {
            inserted_chunks += 1;
        }
    }
    let storage_ms = storage_started.elapsed().as_secs_f64() * 1000.0;

    tracing::debug!(
        total_events = events.len(),
        prepared_rows = rows.len(),
        errors = errors.len(),
        inserted_chunks,
        failed_chunks,
        rollup_write_mode = rollup_write_mode.as_str(),
        decode_prepare_ms,
        storage_ms,
        "metrics batch processing timing"
    );

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
    tool_rollups: Vec<PreparedToolRollup>,
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
        let tool_rollups = prepare_tool_rollups(event);

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
            tool_rollups,
            prompt_id: event.prompt_id.clone(),
            session_id: event.session_id.clone(),
            file_path: event.file_path.clone(),
            custom_attrs_json,
            raw_values_json,
            raw_attrs_json,
        })
    }
}

#[derive(Debug, Clone)]
struct PreparedToolRollup {
    tool_model: String,
    ai_lines: i64,
    mixed_lines: i64,
    ai_accepted: i64,
    total_ai_additions: i64,
    total_ai_deletions: i64,
}

#[derive(Debug, Clone)]
struct PreparedToolModelEventRow {
    metric_event_id: i64,
    org_id: Option<Uuid>,
    user_id: Option<Uuid>,
    timestamp: i64,
    tool_model: String,
    ai_additions: i64,
    mixed_additions: i64,
    ai_accepted: i64,
    total_ai_additions: i64,
    total_ai_deletions: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DailyRollupKey {
    day: chrono::NaiveDate,
    org_id: Uuid,
    user_id: Uuid,
    repo_url: String,
    tool_model: String,
}

#[derive(Debug, Clone)]
struct PreparedDailyRollup {
    key: DailyRollupKey,
    commits: i64,
    total_lines: i64,
    ai_lines: i64,
    human_lines: i64,
    mixed_lines: i64,
    ai_accepted: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RollupDirtyScope {
    day: chrono::NaiveDate,
    org_id: Uuid,
    user_id: Uuid,
}

async fn insert_metrics_chunk(
    pool: &PgPool,
    rows: &[PreparedMetricRow],
    rollup_write_mode: MetricsRollupWriteMode,
) -> Result<(), AppError> {
    if rows.is_empty() {
        return Ok(());
    }

    let total_started = Instant::now();
    let begin_started = Instant::now();
    let mut tx = pool.begin().await.map_err(AppError::Database)?;
    let tx_begin_ms = begin_started.elapsed().as_secs_f64() * 1000.0;

    let events_started = Instant::now();
    let metric_event_ids = insert_metrics_events_chunk(&mut tx, rows).await?;
    let events_insert_ms = events_started.elapsed().as_secs_f64() * 1000.0;

    let tool_events_started = Instant::now();
    let tool_model_rows =
        insert_metrics_tool_model_events_chunk(&mut tx, rows, &metric_event_ids).await?;
    let tool_model_insert_ms = tool_events_started.elapsed().as_secs_f64() * 1000.0;

    let rollups_started = Instant::now();
    let (daily_rollup_rows, dirty_scope_rows) = match rollup_write_mode {
        MetricsRollupWriteMode::Sync => (upsert_metrics_daily_rollups(&mut tx, rows).await?, 0),
        MetricsRollupWriteMode::DirtyAsync => {
            (0, mark_metrics_rollup_dirty_scopes(&mut tx, rows).await?)
        }
        MetricsRollupWriteMode::Off => (0, 0),
    };
    let daily_rollup_upsert_ms = if matches!(rollup_write_mode, MetricsRollupWriteMode::Sync) {
        rollups_started.elapsed().as_secs_f64() * 1000.0
    } else {
        0.0
    };
    let dirty_scope_mark_ms = if matches!(rollup_write_mode, MetricsRollupWriteMode::DirtyAsync) {
        rollups_started.elapsed().as_secs_f64() * 1000.0
    } else {
        0.0
    };

    let commit_started = Instant::now();
    tx.commit().await.map_err(AppError::Database)?;
    let tx_commit_ms = commit_started.elapsed().as_secs_f64() * 1000.0;
    let total_ms = total_started.elapsed().as_secs_f64() * 1000.0;

    tracing::debug!(
        rows = rows.len(),
        tool_model_rows,
        daily_rollup_rows,
        dirty_scope_rows,
        write_rollups = matches!(rollup_write_mode, MetricsRollupWriteMode::Sync),
        rollup_write_mode = rollup_write_mode.as_str(),
        tx_begin_ms,
        events_insert_ms,
        tool_model_insert_ms,
        daily_rollup_upsert_ms,
        dirty_scope_mark_ms,
        tx_commit_ms,
        total_ms,
        "metrics chunk write timing"
    );

    Ok(())
}

async fn insert_metrics_events_chunk(
    tx: &mut Transaction<'_, Postgres>,
    rows: &[PreparedMetricRow],
) -> Result<Vec<i64>, AppError> {
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

    builder.push(" RETURNING id");

    let inserted_rows: Vec<(i64,)> = builder
        .build_query_as()
        .fetch_all(&mut **tx)
        .await
        .map_err(AppError::Database)?;

    Ok(inserted_rows.into_iter().map(|(id,)| id).collect())
}

fn prepare_tool_rollups(event: &DecodedMetricEvent) -> Vec<PreparedToolRollup> {
    let Some(pairs) = event.tool_model_pairs.as_deref() else {
        return Vec::new();
    };

    pairs
        .iter()
        .enumerate()
        .filter_map(|(idx, pair)| {
            if pair == "all" || pair.is_empty() {
                return None;
            }

            Some(PreparedToolRollup {
                tool_model: pair.clone(),
                ai_lines: metric_value_at(event.ai_additions.as_deref(), idx),
                mixed_lines: metric_value_at(event.mixed_additions.as_deref(), idx),
                ai_accepted: metric_value_at(event.ai_accepted.as_deref(), idx),
                total_ai_additions: raw_metric_value_at(&event.raw_values, "7", idx),
                total_ai_deletions: raw_metric_value_at(&event.raw_values, "8", idx),
            })
        })
        .collect()
}

fn raw_metric_value_at(
    raw_values: &std::collections::HashMap<String, serde_json::Value>,
    key: &str,
    idx: usize,
) -> i64 {
    raw_values
        .get(key)
        .and_then(|value| value.as_array())
        .and_then(|values| values.get(idx))
        .and_then(|value| value.as_i64())
        .unwrap_or(0)
}

fn metric_value_at(values: Option<&[i32]>, idx: usize) -> i64 {
    values
        .and_then(|values| values.get(idx).copied())
        .unwrap_or(0)
        .into()
}

fn prepare_tool_model_event_rows(
    rows: &[PreparedMetricRow],
    metric_event_ids: &[i64],
) -> Result<Vec<PreparedToolModelEventRow>, AppError> {
    if rows.len() != metric_event_ids.len() {
        return Err(AppError::Internal(format!(
            "Inserted metrics event id count mismatch: expected {}, got {}",
            rows.len(),
            metric_event_ids.len()
        )));
    }

    let mut tool_rows = Vec::new();
    for (row, metric_event_id) in rows.iter().zip(metric_event_ids.iter().copied()) {
        if row.event_type != 1 {
            continue;
        }

        for tool in &row.tool_rollups {
            tool_rows.push(PreparedToolModelEventRow {
                metric_event_id,
                org_id: row.org_id,
                user_id: row.user_id,
                timestamp: row.timestamp,
                tool_model: tool.tool_model.clone(),
                ai_additions: tool.ai_lines,
                mixed_additions: tool.mixed_lines,
                ai_accepted: tool.ai_accepted,
                total_ai_additions: tool.total_ai_additions,
                total_ai_deletions: tool.total_ai_deletions,
            });
        }
    }

    Ok(tool_rows)
}

async fn insert_metrics_tool_model_events_chunk(
    tx: &mut Transaction<'_, Postgres>,
    rows: &[PreparedMetricRow],
    metric_event_ids: &[i64],
) -> Result<usize, AppError> {
    let tool_rows = prepare_tool_model_event_rows(rows, metric_event_ids)?;
    if tool_rows.is_empty() {
        return Ok(0);
    }
    let tool_row_count = tool_rows.len();

    for chunk in tool_rows.chunks(METRICS_TOOL_MODEL_INSERT_CHUNK_SIZE) {
        insert_metrics_tool_model_event_rows_chunk(tx, chunk).await?;
    }

    Ok(tool_row_count)
}

async fn insert_metrics_tool_model_event_rows_chunk(
    tx: &mut Transaction<'_, Postgres>,
    tool_rows: &[PreparedToolModelEventRow],
) -> Result<(), AppError> {
    if tool_rows.is_empty() {
        return Ok(());
    }

    let mut builder: QueryBuilder<Postgres> = QueryBuilder::new(
        r#"INSERT INTO metrics_tool_model_events (
            metric_event_id, org_id, user_id, timestamp, tool_model,
            ai_additions, mixed_additions, ai_accepted,
            total_ai_additions, total_ai_deletions
        ) "#,
    );

    builder.push_values(tool_rows, |mut row_builder, row| {
        row_builder
            .push_bind(row.metric_event_id)
            .push_bind(row.org_id)
            .push_bind(row.user_id)
            .push_bind(row.timestamp)
            .push_bind(&row.tool_model)
            .push_bind(row.ai_additions)
            .push_bind(row.mixed_additions)
            .push_bind(row.ai_accepted)
            .push_bind(row.total_ai_additions)
            .push_bind(row.total_ai_deletions);
    });

    builder
        .build()
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

    Ok(())
}

fn prepare_daily_rollups(rows: &[PreparedMetricRow]) -> Vec<PreparedDailyRollup> {
    let mut rollups: HashMap<DailyRollupKey, PreparedDailyRollup> = HashMap::new();

    for row in rows {
        if row.event_type != 1 {
            continue;
        }

        let day = metric_day(row.timestamp);
        let org_id = row.org_id.unwrap_or_else(Uuid::nil);
        let user_id = row.user_id.unwrap_or_else(Uuid::nil);
        let repo_url = row.repo_url.clone().unwrap_or_default();
        let total_lines = i64::from(row.git_diff_added_lines.unwrap_or(0));
        let ai_lines = i64::from(row.ai_additions_total);
        let human_lines = (total_lines - ai_lines).max(0);
        let summary_key = DailyRollupKey {
            day,
            org_id,
            user_id,
            repo_url: repo_url.clone(),
            tool_model: String::new(),
        };

        add_rollup_delta(
            &mut rollups,
            summary_key.clone(),
            PreparedDailyRollup {
                key: summary_key,
                commits: 1,
                total_lines,
                ai_lines,
                human_lines,
                mixed_lines: i64::from(row.mixed_additions_total),
                ai_accepted: i64::from(row.ai_accepted_total),
            },
        );

        for tool in &row.tool_rollups {
            let tool_key = DailyRollupKey {
                day,
                org_id,
                user_id,
                repo_url: repo_url.clone(),
                tool_model: tool.tool_model.clone(),
            };
            add_rollup_delta(
                &mut rollups,
                tool_key.clone(),
                PreparedDailyRollup {
                    key: tool_key,
                    commits: 1,
                    total_lines: 0,
                    ai_lines: tool.ai_lines,
                    human_lines: 0,
                    mixed_lines: tool.mixed_lines,
                    ai_accepted: tool.ai_accepted,
                },
            );
        }
    }

    let mut rollups: Vec<_> = rollups.into_values().collect();
    // Keep concurrent upserts locking rollup primary keys in the same order.
    rollups.sort_by(|left, right| {
        left.key
            .day
            .cmp(&right.key.day)
            .then_with(|| left.key.org_id.cmp(&right.key.org_id))
            .then_with(|| left.key.user_id.cmp(&right.key.user_id))
            .then_with(|| left.key.repo_url.cmp(&right.key.repo_url))
            .then_with(|| left.key.tool_model.cmp(&right.key.tool_model))
    });
    rollups
}

fn add_rollup_delta(
    rollups: &mut HashMap<DailyRollupKey, PreparedDailyRollup>,
    key: DailyRollupKey,
    delta: PreparedDailyRollup,
) {
    rollups
        .entry(key)
        .and_modify(|existing| {
            existing.commits += delta.commits;
            existing.total_lines += delta.total_lines;
            existing.ai_lines += delta.ai_lines;
            existing.human_lines += delta.human_lines;
            existing.mixed_lines += delta.mixed_lines;
            existing.ai_accepted += delta.ai_accepted;
        })
        .or_insert(delta);
}

fn metric_day(timestamp: i64) -> chrono::NaiveDate {
    chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0)
        .map(|dt| dt.date_naive())
        .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid date"))
}

async fn upsert_metrics_daily_rollups(
    tx: &mut Transaction<'_, Postgres>,
    rows: &[PreparedMetricRow],
) -> Result<usize, AppError> {
    let rollups = prepare_daily_rollups(rows);
    if rollups.is_empty() {
        return Ok(0);
    }
    let rollup_count = rollups.len();

    let mut builder: QueryBuilder<Postgres> = QueryBuilder::new(
        r#"INSERT INTO metrics_daily_rollups (
            day, org_id, user_id, repo_url, tool_model,
            commits, total_lines, ai_lines, human_lines, mixed_lines, ai_accepted
        ) "#,
    );

    builder.push_values(&rollups, |mut row_builder, row| {
        row_builder
            .push_bind(row.key.day)
            .push_bind(row.key.org_id)
            .push_bind(row.key.user_id)
            .push_bind(&row.key.repo_url)
            .push_bind(&row.key.tool_model)
            .push_bind(row.commits)
            .push_bind(row.total_lines)
            .push_bind(row.ai_lines)
            .push_bind(row.human_lines)
            .push_bind(row.mixed_lines)
            .push_bind(row.ai_accepted);
    });

    builder.push(
        r#" ON CONFLICT (day, org_id, user_id, repo_url, tool_model) DO UPDATE SET
            commits = metrics_daily_rollups.commits + EXCLUDED.commits,
            total_lines = metrics_daily_rollups.total_lines + EXCLUDED.total_lines,
            ai_lines = metrics_daily_rollups.ai_lines + EXCLUDED.ai_lines,
            human_lines = metrics_daily_rollups.human_lines + EXCLUDED.human_lines,
            mixed_lines = metrics_daily_rollups.mixed_lines + EXCLUDED.mixed_lines,
            ai_accepted = metrics_daily_rollups.ai_accepted + EXCLUDED.ai_accepted,
            updated_at = now()"#,
    );

    builder
        .build()
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

    Ok(rollup_count)
}

fn prepare_rollup_dirty_scopes(rows: &[PreparedMetricRow]) -> Vec<RollupDirtyScope> {
    let mut scopes = HashSet::new();

    for row in rows {
        if row.event_type != 1 {
            continue;
        }

        scopes.insert(RollupDirtyScope {
            day: metric_day(row.timestamp),
            org_id: row.org_id.unwrap_or_else(Uuid::nil),
            user_id: row.user_id.unwrap_or_else(Uuid::nil),
        });
    }

    let mut scopes: Vec<_> = scopes.into_iter().collect();
    scopes.sort_by(|left, right| {
        left.day
            .cmp(&right.day)
            .then_with(|| left.org_id.cmp(&right.org_id))
            .then_with(|| left.user_id.cmp(&right.user_id))
    });
    scopes
}

async fn mark_metrics_rollup_dirty_scopes(
    tx: &mut Transaction<'_, Postgres>,
    rows: &[PreparedMetricRow],
) -> Result<usize, AppError> {
    let scopes = prepare_rollup_dirty_scopes(rows);
    if scopes.is_empty() {
        return Ok(0);
    }
    let scope_count = scopes.len();

    let mut builder: QueryBuilder<Postgres> =
        QueryBuilder::new("INSERT INTO metrics_rollup_dirty_scopes (day, org_id, user_id) ");

    builder.push_values(&scopes, |mut row_builder, scope| {
        row_builder
            .push_bind(scope.day)
            .push_bind(scope.org_id)
            .push_bind(scope.user_id);
    });

    builder
        .build()
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

    Ok(scope_count)
}

pub fn spawn_metrics_rollup_worker(
    pool: PgPool,
    interval: Duration,
    batch_size: i64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);

        loop {
            ticker.tick().await;

            match process_metrics_rollup_dirty_scopes(&pool, batch_size).await {
                Ok(processed) if processed > 0 => {
                    tracing::info!(
                        processed_scopes = processed,
                        "processed metrics rollup dirty scopes"
                    );
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "failed to process metrics rollup dirty scopes");
                }
            }
        }
    })
}

pub async fn process_metrics_rollup_dirty_scopes(
    pool: &PgPool,
    batch_size: i64,
) -> Result<u64, AppError> {
    let mut tx = pool.begin().await.map_err(AppError::Database)?;
    let batch_size = batch_size.max(1);

    let dirty_rows: Vec<(i64, chrono::NaiveDate, Uuid, Uuid)> = sqlx::query_as(
        r#"SELECT id, day, org_id, user_id
           FROM metrics_rollup_dirty_scopes
           WHERE claimed_at IS NULL OR claimed_at < now() - interval '5 minutes'
           ORDER BY id
           LIMIT $1
           FOR UPDATE SKIP LOCKED"#,
    )
    .bind(batch_size)
    .fetch_all(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    if dirty_rows.is_empty() {
        tx.commit().await.map_err(AppError::Database)?;
        return Ok(0);
    }

    let dirty_row_ids: Vec<i64> = dirty_rows.iter().map(|row| row.0).collect();
    let mut scope_set = HashSet::new();
    for (_, day, org_id, user_id) in &dirty_rows {
        scope_set.insert(RollupDirtyScope {
            day: *day,
            org_id: *org_id,
            user_id: *user_id,
        });
    }

    let mut scopes: Vec<_> = scope_set.into_iter().collect();
    scopes.sort_by(|left, right| {
        left.day
            .cmp(&right.day)
            .then_with(|| left.org_id.cmp(&right.org_id))
            .then_with(|| left.user_id.cmp(&right.user_id))
    });

    for scope in &scopes {
        rebuild_metrics_daily_rollups_for_scope(&mut tx, scope.day, scope.org_id, scope.user_id)
            .await?;
    }

    sqlx::query("DELETE FROM metrics_rollup_dirty_scopes WHERE id = ANY($1)")
        .bind(&dirty_row_ids)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

    let processed = scopes.len() as u64;
    tx.commit().await.map_err(AppError::Database)?;
    Ok(processed)
}

async fn rebuild_metrics_daily_rollups_for_scope(
    tx: &mut Transaction<'_, Postgres>,
    day: chrono::NaiveDate,
    org_id: Uuid,
    user_id: Uuid,
) -> Result<(), AppError> {
    sqlx::query(
        r#"DELETE FROM metrics_daily_rollups
           WHERE day = $1 AND org_id = $2 AND user_id = $3"#,
    )
    .bind(day)
    .bind(org_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(AppError::Database)?;

    sqlx::query(
        r#"INSERT INTO metrics_daily_rollups (
               day, org_id, user_id, repo_url, tool_model,
               commits, total_lines, ai_lines, human_lines, mixed_lines, ai_accepted
           )
           SELECT
               $1::date AS day,
               $2::uuid AS org_id,
               $3::uuid AS user_id,
               COALESCE(repo_url, '') AS repo_url,
               '' AS tool_model,
               COUNT(*)::bigint AS commits,
               COALESCE(SUM(git_diff_added_lines), 0)::bigint AS total_lines,
               COALESCE(SUM(ai_additions), 0)::bigint AS ai_lines,
               COALESCE(SUM(GREATEST(COALESCE(git_diff_added_lines, 0) - COALESCE(ai_additions, 0), 0)), 0)::bigint AS human_lines,
               COALESCE(SUM(mixed_additions), 0)::bigint AS mixed_lines,
               COALESCE(SUM(ai_accepted), 0)::bigint AS ai_accepted
           FROM metrics_events
           WHERE event_type = 1
             AND (to_timestamp(timestamp) AT TIME ZONE 'UTC')::date = $1
             AND COALESCE(org_id, '00000000-0000-0000-0000-000000000000'::uuid) = $2
             AND COALESCE(user_id, '00000000-0000-0000-0000-000000000000'::uuid) = $3
           GROUP BY COALESCE(repo_url, '')"#,
    )
    .bind(day)
    .bind(org_id)
    .bind(user_id)
    .execute(&mut **tx)
    .await
    .map_err(AppError::Database)?;

    sqlx::query(
        r#"INSERT INTO metrics_daily_rollups (
               day, org_id, user_id, repo_url, tool_model,
               commits, total_lines, ai_lines, human_lines, mixed_lines, ai_accepted
           )
           SELECT
               $1::date AS day,
               $2::uuid AS org_id,
               $3::uuid AS user_id,
               COALESCE(m.repo_url, '') AS repo_url,
               t.tool_model,
               COUNT(DISTINCT m.id)::bigint AS commits,
               0::bigint AS total_lines,
               COALESCE(SUM(t.ai_additions), 0)::bigint AS ai_lines,
               0::bigint AS human_lines,
               COALESCE(SUM(t.mixed_additions), 0)::bigint AS mixed_lines,
               COALESCE(SUM(t.ai_accepted), 0)::bigint AS ai_accepted
           FROM metrics_events m
           JOIN metrics_tool_model_events t ON t.metric_event_id = m.id
           WHERE m.event_type = 1
             AND (to_timestamp(m.timestamp) AT TIME ZONE 'UTC')::date = $1
             AND COALESCE(m.org_id, '00000000-0000-0000-0000-000000000000'::uuid) = $2
             AND COALESCE(m.user_id, '00000000-0000-0000-0000-000000000000'::uuid) = $3
             AND t.tool_model != 'all'
             AND t.tool_model != ''
           GROUP BY COALESCE(m.repo_url, ''), t.tool_model"#,
    )
    .bind(day)
    .bind(org_id)
    .bind(user_id)
    .execute(&mut **tx)
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
            MetricsRollupWriteMode::Sync,
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
            MetricsRollupWriteMode::Sync,
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

        let stored_rollup_org_id: Uuid = sqlx::query_scalar(
            "SELECT org_id FROM metrics_daily_rollups WHERE user_id = $1 AND tool_model = '' LIMIT 1",
        )
        .bind(user_id)
        .fetch_one(&db.pool)
        .await?;
        assert_eq!(stored_rollup_org_id, Uuid::nil());

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
            MetricsRollupWriteMode::Sync,
        )
        .await;

        assert!(response.errors.is_empty());
        assert_eq!(metrics_count(&db.pool).await?, 10);
        let summary: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT COALESCE(SUM(commits), 0)::bigint,
                    COALESCE(SUM(total_lines), 0)::bigint,
                    COALESCE(SUM(ai_lines), 0)::bigint,
                    COALESCE(SUM(human_lines), 0)::bigint
             FROM metrics_daily_rollups
             WHERE org_id = $1 AND user_id = $2 AND tool_model = ''",
        )
        .bind(org_id)
        .bind(user_id)
        .fetch_one(&db.pool)
        .await?;
        assert_eq!(summary, (10, 300, 200, 100));

        let tool_ai_lines: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(ai_lines), 0)::bigint
             FROM metrics_daily_rollups
             WHERE org_id = $1 AND user_id = $2 AND tool_model = $3",
        )
        .bind(org_id)
        .bind(user_id)
        .bind("codex::gpt-5")
        .fetch_one(&db.pool)
        .await?;
        assert_eq!(tool_ai_lines, 50);

        let tool_model_row: (i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT COALESCE(SUM(ai_additions), 0)::bigint,
                    COALESCE(SUM(mixed_additions), 0)::bigint,
                    COALESCE(SUM(ai_accepted), 0)::bigint,
                    COALESCE(SUM(total_ai_additions), 0)::bigint,
                    COALESCE(SUM(total_ai_deletions), 0)::bigint
             FROM metrics_tool_model_events
             WHERE org_id = $1 AND user_id = $2 AND tool_model = $3",
        )
        .bind(org_id)
        .bind(user_id)
        .bind("codex::gpt-5")
        .fetch_one(&db.pool)
        .await?;
        assert_eq!(tool_model_row, (50, 20, 30, 50, 0));

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn process_metrics_batch_can_disable_daily_rollups() -> anyhow::Result<()> {
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
            MetricsRollupWriteMode::Off,
        )
        .await;

        assert!(response.errors.is_empty());
        assert_eq!(metrics_count(&db.pool).await?, 1);
        assert_eq!(rollups_count(&db.pool).await?, 0);
        assert_eq!(tool_model_events_count(&db.pool).await?, 1);
        assert_eq!(rollup_dirty_scopes_count(&db.pool).await?, 0);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn process_metrics_batch_marks_dirty_scope_for_async_rollups() -> anyhow::Result<()> {
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
            MetricsRollupWriteMode::DirtyAsync,
        )
        .await;

        assert!(response.errors.is_empty());
        assert_eq!(metrics_count(&db.pool).await?, 1);
        assert_eq!(rollups_count(&db.pool).await?, 0);
        assert_eq!(tool_model_events_count(&db.pool).await?, 1);
        assert_eq!(rollup_dirty_scopes_count(&db.pool).await?, 1);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn process_metrics_rollup_dirty_scopes_rebuilds_daily_rollups() -> anyhow::Result<()> {
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
            MetricsRollupWriteMode::DirtyAsync,
        )
        .await;

        assert!(response.errors.is_empty());
        assert_eq!(rollups_count(&db.pool).await?, 0);
        assert_eq!(rollup_dirty_scopes_count(&db.pool).await?, 1);

        assert_eq!(process_metrics_rollup_dirty_scopes(&db.pool, 100).await?, 1);
        assert_eq!(rollup_dirty_scopes_count(&db.pool).await?, 0);
        assert_eq!(
            summary_rollup_totals(&db.pool, org_id, user_id).await?,
            (10, 300, 200, 100)
        );
        assert_eq!(
            tool_rollup_ai_lines(&db.pool, org_id, user_id, "codex::gpt-5").await?,
            50
        );

        assert_eq!(process_metrics_rollup_dirty_scopes(&db.pool, 100).await?, 0);

        let response = process_metrics_batch(
            &db.pool,
            vec![committed_metric_event_with_seed(10)],
            Some(user_id),
            Some(org_id),
            Some("metrics-test-device".into()),
            MetricsRollupWriteMode::DirtyAsync,
        )
        .await;

        assert!(response.errors.is_empty());
        assert_eq!(process_metrics_rollup_dirty_scopes(&db.pool, 100).await?, 1);
        assert_eq!(
            summary_rollup_totals(&db.pool, org_id, user_id).await?,
            (11, 330, 220, 110)
        );
        assert_eq!(
            tool_rollup_ai_lines(&db.pool, org_id, user_id, "codex::gpt-5").await?,
            55
        );

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
            MetricsRollupWriteMode::Sync,
        )
        .await;

        assert!(response.errors.is_empty());
        assert_eq!(metrics_count(&db.pool).await?, 501);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn process_metrics_batch_chunks_large_tool_model_rows() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.pool).await?;
        let tool_model_count = METRICS_TOOL_MODEL_INSERT_CHUNK_SIZE + 1;

        let response = process_metrics_batch(
            &db.pool,
            vec![committed_metric_event_with_tool_count(0, tool_model_count)],
            Some(user_id),
            Some(org_id),
            Some("metrics-test-device".into()),
            MetricsRollupWriteMode::Off,
        )
        .await;

        assert!(response.errors.is_empty());
        assert_eq!(metrics_count(&db.pool).await?, 1);
        assert_eq!(
            tool_model_events_count(&db.pool).await?,
            tool_model_count as i64
        );

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
            MetricsRollupWriteMode::Sync,
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
            MetricsRollupWriteMode::Sync,
        )
        .await;

        assert_eq!(response.errors.len(), 2);
        assert_eq!(response.errors[0].index, 0);
        assert_eq!(response.errors[1].index, 1);
        assert!(response
            .errors
            .iter()
            .all(|error| error.error.starts_with("Storage error:")));
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
        values.insert("4".into(), serde_json::json!([8, 2]));
        values.insert("5".into(), serde_json::json!([20, 5]));
        values.insert("6".into(), serde_json::json!([10, 3]));
        values.insert("7".into(), serde_json::json!([20, 5]));
        values.insert("8".into(), serde_json::json!([0, 0]));

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

    fn committed_metric_event_with_tool_count(seed: usize, tool_model_count: usize) -> MetricEvent {
        let mut values = HashMap::new();
        let mut pairs = Vec::with_capacity(tool_model_count + 1);
        pairs.push("all".to_string());
        pairs.extend((0..tool_model_count).map(|idx| format!("codex::gpt-5-{idx}")));
        let mut per_tool_values = vec![0; tool_model_count + 1];
        for value in per_tool_values.iter_mut().skip(1) {
            *value = 1;
        }

        values.insert("0".into(), serde_json::json!(10));
        values.insert("1".into(), serde_json::json!(2));
        values.insert("2".into(), serde_json::json!(30));
        values.insert("3".into(), serde_json::json!(pairs));
        values.insert("4".into(), serde_json::json!(per_tool_values));
        values.insert("5".into(), serde_json::json!(vec![0; tool_model_count + 1]));
        values.insert("6".into(), serde_json::json!(vec![0; tool_model_count + 1]));
        values.insert("7".into(), serde_json::json!(vec![0; tool_model_count + 1]));
        values.insert("8".into(), serde_json::json!(vec![0; tool_model_count + 1]));

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

    async fn rollups_count(pool: &PgPool) -> anyhow::Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM metrics_daily_rollups")
                .fetch_one(pool)
                .await?,
        )
    }

    async fn rollup_dirty_scopes_count(pool: &PgPool) -> anyhow::Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM metrics_rollup_dirty_scopes")
                .fetch_one(pool)
                .await?,
        )
    }

    async fn summary_rollup_totals(
        pool: &PgPool,
        org_id: Uuid,
        user_id: Uuid,
    ) -> anyhow::Result<(i64, i64, i64, i64)> {
        Ok(sqlx::query_as(
            "SELECT COALESCE(SUM(commits), 0)::bigint,
                    COALESCE(SUM(total_lines), 0)::bigint,
                    COALESCE(SUM(ai_lines), 0)::bigint,
                    COALESCE(SUM(human_lines), 0)::bigint
             FROM metrics_daily_rollups
             WHERE org_id = $1 AND user_id = $2 AND tool_model = ''",
        )
        .bind(org_id)
        .bind(user_id)
        .fetch_one(pool)
        .await?)
    }

    async fn tool_rollup_ai_lines(
        pool: &PgPool,
        org_id: Uuid,
        user_id: Uuid,
        tool_model: &str,
    ) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT COALESCE(SUM(ai_lines), 0)::bigint
             FROM metrics_daily_rollups
             WHERE org_id = $1 AND user_id = $2 AND tool_model = $3",
        )
        .bind(org_id)
        .bind(user_id)
        .bind(tool_model)
        .fetch_one(pool)
        .await?)
    }

    async fn tool_model_events_count(pool: &PgPool) -> anyhow::Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT COUNT(*) FROM metrics_tool_model_events")
                .fetch_one(pool)
                .await?,
        )
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
