//! Phase 6: Enterprise lifecycle APIs
//!
//! PR-level aggregation, AI code persistence tracking, agent readiness, and lifecycle queries.

use axum::extract::{Query, State};
use axum::response::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthExtractor;
use crate::error::AppError;
use crate::pagination::{
    clamp_limit, decode_cursor, encode_cursor, fetch_limit, pagination_meta, truncate_to_limit,
    CURSOR_VERSION, DEFAULT_LIMIT, MAX_LIMIT,
};
use crate::routes::AppState;

const PERSISTENCE_DEFAULT_DAYS: i64 = 365;
const LIFECYCLE_EVENT_LIMIT: i64 = 100;
const LIFECYCLE_EVENT_FETCH_LIMIT: i64 = LIFECYCLE_EVENT_LIMIT + 1;

// ================================================================
// PR-level aggregation
// ================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PullRequestCursor {
    v: u8,
    merged_at: Option<chrono::DateTime<chrono::Utc>>,
    id: Uuid,
}

fn decode_pull_request_cursor(cursor: Option<&str>) -> Result<Option<PullRequestCursor>, AppError> {
    let cursor: Option<PullRequestCursor> = cursor.map(decode_cursor).transpose()?;
    if let Some(cursor) = &cursor {
        if cursor.v != CURSOR_VERSION {
            return Err(AppError::BadRequest(format!(
                "Unsupported pagination cursor version: {}",
                cursor.v
            )));
        }
    }
    Ok(cursor)
}

fn encode_pull_request_cursor(
    merged_at: Option<chrono::DateTime<chrono::Utc>>,
    id: Uuid,
) -> Result<String, AppError> {
    encode_cursor(&PullRequestCursor {
        v: CURSOR_VERSION,
        merged_at,
        id,
    })
}

#[derive(Debug, Deserialize)]
pub struct PrAggregateQuery {
    pub org: Option<String>,
    pub repo: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// GET /api/v1/aggregate/pull-requests — PR-level AI attribution aggregation
pub async fn aggregate_pull_requests(
    State(state): State<AppState>,
    auth: AuthExtractor,
    Query(query): Query<PrAggregateQuery>,
) -> Result<Json<Value>, AppError> {
    let (_user_filter, org_filter) = crate::handlers::dashboard::build_data_filters(&auth.0);
    let limit = clamp_limit(query.limit, DEFAULT_LIMIT, MAX_LIMIT);
    let cursor = decode_pull_request_cursor(query.cursor.as_deref())?;
    let cursor_merged_at = cursor.as_ref().and_then(|cursor| cursor.merged_at);
    let cursor_id = cursor.as_ref().map(|cursor| cursor.id);

    let mut rows: Vec<(Uuid, Option<Uuid>, String, String, Option<String>, Option<String>, Option<String>, Option<chrono::DateTime<chrono::Utc>>, i32, i32, i32, f32, Option<Vec<String>>, i32, i32)> = sqlx::query_as(
        r#"SELECT id, org_id, repo_url, pr_id, pr_url, title, author_email, merged_at,
                  total_lines, ai_lines, human_lines, pct_ai, tools_used, files_changed, ai_files
        FROM pull_requests
        WHERE ($1::text IS NULL OR repo_url = $1)
          AND ($2::text IS NULL OR org_id = (SELECT id FROM organizations WHERE slug = $2))
          AND ($3::timestamptz IS NULL OR merged_at >= $3::timestamptz)
          AND ($4::timestamptz IS NULL OR merged_at <= $4::timestamptz)
          AND ($5::uuid IS NULL OR org_id = $5)
          AND (
              $7::uuid IS NULL
              OR (
                  $6::timestamptz IS NOT NULL
                  AND (
                      merged_at < $6::timestamptz
                      OR (merged_at = $6::timestamptz AND id < $7::uuid)
                      OR merged_at IS NULL
                  )
              )
              OR (
                  $6::timestamptz IS NULL
                  AND merged_at IS NULL
                  AND id < $7::uuid
              )
          )
        ORDER BY merged_at DESC NULLS LAST, id DESC
        LIMIT $8"#
    )
    .bind(&query.repo)
    .bind(&query.org)
    .bind(&query.since)
    .bind(&query.until)
    .bind(org_filter)
    .bind(cursor_merged_at)
    .bind(cursor_id)
    .bind(fetch_limit(limit))
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let has_more = truncate_to_limit(&mut rows, limit);
    let next_cursor = if has_more {
        rows.last()
            .map(|(id, _, _, _, _, _, _, merged_at, _, _, _, _, _, _, _)| {
                encode_pull_request_cursor(*merged_at, *id)
            })
            .transpose()?
    } else {
        None
    };

    let prs: Vec<Value> = rows.iter().map(|(id, org_id, repo_url, pr_id, pr_url, title, author, merged, total, ai, human, pct, tools, files, ai_files)| {
        json!({
            "id": id.to_string(),
            "org_id": org_id.map(|u| u.to_string()),
            "repo_url": repo_url,
            "pr_id": pr_id,
            "pr_url": pr_url,
            "title": title,
            "author": author,
            "merged_at": merged,
            "total_lines": total,
            "ai_lines": ai,
            "human_lines": human,
            "pct_ai": pct,
            "tools_used": tools,
            "files_changed": files,
            "ai_files": ai_files,
        })
    }).collect();

    let summary: (i64, Option<f64>, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
              COUNT(*)::bigint AS total_prs,
              AVG(pct_ai)::double precision AS avg_pct_ai,
              COUNT(*) FILTER (WHERE total_lines <= 100)::bigint AS small,
              COUNT(*) FILTER (WHERE total_lines > 100 AND total_lines <= 500)::bigint AS medium,
              COUNT(*) FILTER (WHERE total_lines > 500)::bigint AS large
        FROM pull_requests
        WHERE ($1::text IS NULL OR repo_url = $1)
          AND ($2::text IS NULL OR org_id = (SELECT id FROM organizations WHERE slug = $2))
          AND ($3::timestamptz IS NULL OR merged_at >= $3::timestamptz)
          AND ($4::timestamptz IS NULL OR merged_at <= $4::timestamptz)
          AND ($5::uuid IS NULL OR org_id = $5)"#,
    )
    .bind(&query.repo)
    .bind(&query.org)
    .bind(&query.since)
    .bind(&query.until)
    .bind(org_filter)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let (total_prs, avg_pct_ai, small, medium, large) = summary;
    let avg_pct_ai = avg_pct_ai.unwrap_or(0.0);

    Ok(Json(json!({
        "pull_requests": prs,
        "pagination": pagination_meta(limit, has_more, next_cursor),
        "summary": {
            "total_prs": total_prs,
            "avg_pct_ai": (avg_pct_ai * 100.0).round() / 100.0,
            "pr_size_distribution": { "small": small, "medium": medium, "large": large }
        }
    })))
}

/// POST /api/v1/pull-requests — Record a pull request with AI attribution
#[derive(Debug, Deserialize)]
pub struct CreatePullRequestRequest {
    pub org_id: Option<Uuid>,
    pub repo_url: String,
    pub pr_id: String,
    pub pr_url: Option<String>,
    pub title: Option<String>,
    pub author_email: Option<String>,
    pub merged_at: Option<chrono::DateTime<chrono::Utc>>,
    pub total_lines: Option<i32>,
    pub ai_lines: Option<i32>,
    pub human_lines: Option<i32>,
    pub tools_used: Option<Vec<String>>,
    pub files_changed: Option<i32>,
    pub ai_files: Option<i32>,
    pub commit_shas: Option<Vec<String>>,
}

pub async fn create_pull_request(
    State(state): State<AppState>,
    _auth: AuthExtractor,
    Json(req): Json<CreatePullRequestRequest>,
) -> Result<Json<Value>, AppError> {
    let total_lines = req.total_lines.unwrap_or(0);
    let ai_lines = req.ai_lines.unwrap_or(0);
    let human_lines = req.human_lines.unwrap_or(0);
    let pct_ai = if total_lines > 0 { (ai_lines as f32 / total_lines as f32) * 100.0 } else { 0.0 };

    let id = Uuid::new_v4();

    sqlx::query(
        r#"INSERT INTO pull_requests (id, org_id, repo_url, pr_id, pr_url, title, author_email, merged_at,
           total_lines, ai_lines, human_lines, pct_ai, tools_used, files_changed, ai_files, commit_shas)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
        ON CONFLICT (repo_url, pr_id) DO UPDATE SET
            title = COALESCE(EXCLUDED.title, pull_requests.title),
            merged_at = COALESCE(EXCLUDED.merged_at, pull_requests.merged_at),
            total_lines = EXCLUDED.total_lines,
            ai_lines = EXCLUDED.ai_lines,
            human_lines = EXCLUDED.human_lines,
            pct_ai = EXCLUDED.pct_ai,
            tools_used = EXCLUDED.tools_used,
            files_changed = EXCLUDED.files_changed,
            ai_files = EXCLUDED.ai_files,
            commit_shas = EXCLUDED.commit_shas,
            updated_at = now()"#
    )
    .bind(id)
    .bind(req.org_id)
    .bind(&req.repo_url)
    .bind(&req.pr_id)
    .bind(&req.pr_url)
    .bind(&req.title)
    .bind(&req.author_email)
    .bind(req.merged_at)
    .bind(total_lines)
    .bind(ai_lines)
    .bind(human_lines)
    .bind(pct_ai)
    .bind(&req.tools_used)
    .bind(req.files_changed.unwrap_or(0))
    .bind(req.ai_files.unwrap_or(0))
    .bind(&req.commit_shas)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    Ok(Json(json!({
        "id": id.to_string(),
        "pr_id": req.pr_id,
        "repo_url": req.repo_url,
        "pct_ai": pct_ai,
    })))
}

// ================================================================
// AI code persistence tracking
// ================================================================

#[derive(Debug, Deserialize)]
pub struct PersistenceQuery {
    pub org: Option<String>,
    pub repo: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
}

fn parse_persistence_date_param(
    name: &str,
    value: Option<&str>,
) -> Result<Option<chrono::NaiveDate>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }

    if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Ok(Some(date));
    }
    if let Ok(datetime) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(Some(datetime.with_timezone(&chrono::Utc).date_naive()));
    }

    Err(AppError::BadRequest(format!(
        "{} must be an RFC3339 timestamp or YYYY-MM-DD date",
        name
    )))
}

fn bounded_persistence_dates(
    since: Option<&str>,
    until: Option<&str>,
) -> Result<(chrono::NaiveDate, chrono::NaiveDate), AppError> {
    let until_date =
        parse_persistence_date_param("until", until)?.unwrap_or_else(|| chrono::Utc::now().date_naive());
    let since_date = parse_persistence_date_param("since", since)?
        .unwrap_or_else(|| until_date - chrono::Duration::days(PERSISTENCE_DEFAULT_DAYS));

    if since_date > until_date {
        return Err(AppError::BadRequest(
            "since must be earlier than or equal to until".into(),
        ));
    }

    Ok((since_date, until_date))
}

fn date_start_epoch_seconds(date: chrono::NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0)
        .expect("midnight is a valid time")
        .and_utc()
        .timestamp()
}

fn date_end_epoch_seconds(date: chrono::NaiveDate) -> i64 {
    date.and_hms_opt(23, 59, 59)
        .expect("23:59:59 is a valid time")
        .and_utc()
        .timestamp()
}

/// GET /api/v1/ai-code-persistence — AI code survival rate tracking
pub async fn get_ai_code_persistence(
    State(state): State<AppState>,
    auth: AuthExtractor,
    Query(query): Query<PersistenceQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = crate::handlers::dashboard::build_data_filters(&auth.0);
    let (since_date, until_date) =
        bounded_persistence_dates(query.since.as_deref(), query.until.as_deref())?;
    let since_epoch = date_start_epoch_seconds(since_date);
    let until_epoch = date_end_epoch_seconds(until_date);

    // Get the latest snapshot
    let snapshot: Option<(Uuid, Option<Uuid>, String, chrono::NaiveDate, i32, i32, i32, i32, f32, Option<serde_json::Value>)> = sqlx::query_as(
        r#"SELECT id, org_id, repo_url, snapshot_date,
                  total_ai_lines_introduced, lines_still_present, lines_modified, lines_deleted, survival_rate, by_tool
        FROM ai_code_persistence_snapshots
        WHERE ($1::text IS NULL OR repo_url = $1)
          AND ($2::text IS NULL OR org_id = (SELECT id FROM organizations WHERE slug = $2))
          AND snapshot_date >= $3::date
          AND ($4::uuid IS NULL OR org_id = $4)
          AND snapshot_date <= $5::date
        ORDER BY snapshot_date DESC
        LIMIT 1"#
    )
    .bind(&query.repo)
    .bind(&query.org)
    .bind(since_date)
    .bind(org_filter)
    .bind(until_date)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let snapshot_data = match snapshot {
        Some((_, org_id, repo_url, snap_date, total, present, modified, deleted, rate, by_tool)) => {
            json!({
                "org_id": org_id.map(|u| u.to_string()),
                "repo_url": repo_url,
                "snapshot_date": snap_date,
                "total_ai_lines_introduced": total,
                "lines_still_present": present,
                "lines_modified": modified,
                "lines_deleted": deleted,
                "survival_rate": rate,
                "by_tool": by_tool,
            })
        }
        None => serde_json::Value::Null,
    };

    // Get trend data (weekly snapshots)
    let trend_rows: Vec<(chrono::NaiveDate, f32)> = sqlx::query_as(
        r#"SELECT snapshot_date, survival_rate
        FROM ai_code_persistence_snapshots
        WHERE ($1::text IS NULL OR repo_url = $1)
          AND ($2::text IS NULL OR org_id = (SELECT id FROM organizations WHERE slug = $2))
          AND ($3::uuid IS NULL OR org_id = $3)
          AND snapshot_date >= $4::date
          AND snapshot_date <= $5::date
        ORDER BY snapshot_date"#
    )
    .bind(&query.repo)
    .bind(&query.org)
    .bind(org_filter)
    .bind(since_date)
    .bind(until_date)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let trend: Vec<Value> = trend_rows.iter().map(|(date, rate)| {
        json!({ "week": date.to_string(), "survival_rate": rate })
    }).collect();

    // If no persistence snapshot data exists yet, compute from metrics_events as fallback
    if snapshot_data.is_null() {
        let fallback: Option<(Option<i64>, Option<i64>)> = sqlx::query_as(
            r#"SELECT COALESCE(SUM(ai_additions), 0), COALESCE(SUM(human_additions), 0)
            FROM metrics_events WHERE event_type = 1
              AND ($1::uuid IS NULL OR user_id = $1)
              AND ($2::uuid IS NULL OR org_id = $2)
              AND timestamp >= $3::bigint
              AND timestamp <= $4::bigint"#
        )
        .bind(user_filter)
        .bind(org_filter)
        .bind(since_epoch)
        .bind(until_epoch)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

        let (ai_lines, _human_lines) = fallback.unwrap_or((Some(0), Some(0)));

        return Ok(Json(json!({
            "period": { "since": since_date, "until": until_date },
            "ai_code_snapshot": {
                "total_ai_lines_introduced": ai_lines.unwrap_or(0),
                "lines_still_present": ai_lines.unwrap_or(0),  // Assumed present without historical data
                "lines_modified": 0,
                "lines_deleted": 0,
                "survival_rate": if ai_lines.unwrap_or(0) > 0 { 100.0 } else { 0.0 },
                "by_tool": null,
                "source": "metrics_fallback",
            },
            "trend": trend,
        })));
    }

    Ok(Json(json!({
        "period": { "since": since_date, "until": until_date },
        "ai_code_snapshot": snapshot_data,
        "trend": trend,
    })))
}

// ================================================================
// Agent readiness evaluation
// ================================================================

#[derive(Debug, Deserialize)]
pub struct AgentReadinessQuery {
    pub org: Option<String>,
}

/// GET /api/v1/agent-readiness — Agent readiness scores
pub async fn get_agent_readiness(
    State(state): State<AppState>,
    auth: AuthExtractor,
    Query(query): Query<AgentReadinessQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = crate::handlers::dashboard::build_data_filters(&auth.0);

    let rows: Vec<(Uuid, Option<Uuid>, String, String, i32, String, Option<serde_json::Value>, chrono::NaiveDate, chrono::NaiveDate)> = sqlx::query_as(
        r#"SELECT id, org_id, tool, model, overall_score, trend, config_changes, eval_period_start, eval_period_end
        FROM agent_readiness_scores
        WHERE ($1::text IS NULL OR org_id = (SELECT id FROM organizations WHERE slug = $1))
          AND ($2::uuid IS NULL OR org_id = $2)
        ORDER BY eval_period_end DESC"#
    )
    .bind(&query.org)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // If no explicit readiness data, derive from tool_model_stats
    let agents: Vec<Value> = if rows.is_empty() {
        let tool_rows: Vec<(String, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
            r#"SELECT tms.tool_model,
                      COALESCE(SUM(tms.ai_additions), 0),
                      COALESCE(SUM(tms.mixed_additions), 0),
                      COALESCE(SUM(tms.ai_accepted), 0)
            FROM tool_model_stats tms
            JOIN projects p ON tms.project_id = p.id
            WHERE ($1::uuid IS NULL OR p.user_id = $1)
              AND ($2::uuid IS NULL OR p.org_id = $2)
            GROUP BY tms.tool_model
            ORDER BY SUM(tms.ai_additions) DESC"#
        )
        .bind(user_filter)
        .bind(org_filter)
        .fetch_all(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

        tool_rows.iter().map(|(tool_model, ai_add, _mixed, accepted)| {
            let ai_add = ai_add.unwrap_or(0);
            let accepted = accepted.unwrap_or(0);
            let score = if ai_add > 0 { ((accepted as f64 / ai_add as f64) * 100.0).min(100.0) as i32 } else { 0 };
            let parts: Vec<&str> = tool_model.split("::").collect();
            let tool = parts.get(0).unwrap_or(&"unknown");
            let model = parts.get(1).unwrap_or(&"");
            json!({
                "tool": tool,
                "model": model,
                "overall_score": score,
                "trend": "stable",
                "config_changes": [],
                "source": "derived_from_stats",
            })
        }).collect()
    } else {
        rows.iter().map(|(_, org_id, tool, model, score, trend, config_changes, start, end)| {
            json!({
                "org_id": org_id.map(|u| u.to_string()),
                "tool": tool,
                "model": model,
                "overall_score": score,
                "trend": trend,
                "config_changes": config_changes,
                "eval_period": { "start": start.to_string(), "end": end.to_string() },
            })
        }).collect()
    };

    Ok(Json(json!({ "agents": agents })))
}

// ================================================================
// AI code lifecycle query
// ================================================================

#[derive(Debug, Deserialize)]
pub struct LifecycleQuery {
    pub org: Option<String>,
    pub commit_sha: Option<String>,
}

/// GET /api/v1/ai-code-lifecycle — Full lifecycle tracking for AI code
pub async fn get_ai_code_lifecycle(
    State(state): State<AppState>,
    auth: AuthExtractor,
    Query(query): Query<LifecycleQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = crate::handlers::dashboard::build_data_filters(&auth.0);

    let commit_sha = match &query.commit_sha {
        Some(sha) => sha.as_str(),
        None => return Err(AppError::BadRequest("commit_sha is required".into())),
    };

    // Get metrics event for this commit (with data isolation)
    let commit_data: Option<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"SELECT author_email, ai_additions::bigint, human_additions::bigint
        FROM metrics_events
        WHERE commit_sha = $1 AND event_type = 1
          AND ($2::uuid IS NULL OR user_id = $2)
          AND ($3::uuid IS NULL OR org_id = $3)
        LIMIT 1"#
    )
    .bind(commit_sha)
    .bind(user_filter)
    .bind(org_filter)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let (author, ai_lines, human_lines) = match commit_data {
        Some(d) => d,
        None => return Err(AppError::NotFound(format!("No data found for commit {}", commit_sha))),
    };

    // Build lifecycle stages
    let mut lifecycle: Vec<Value> = Vec::new();

    // Stage 1: Written (from metrics)
    let tool_rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"SELECT tool, model FROM metrics_events WHERE commit_sha = $1 AND event_type = 1
          AND ($2::uuid IS NULL OR user_id = $2)
          AND ($3::uuid IS NULL OR org_id = $3)"#
    )
    .bind(commit_sha)
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let tool_desc: String = tool_rows.iter()
        .filter_map(|(t, m)| {
            match (t.as_deref(), m.as_deref()) {
                (Some(tool), Some(model)) if !model.is_empty() => Some(format!("{}::{}", tool, model)),
                (Some(tool), _) => Some(tool.to_string()),
                _ => None,
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    let written_ts: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        r#"SELECT MIN(created_at) FROM metrics_events WHERE commit_sha = $1
          AND ($2::uuid IS NULL OR user_id = $2)
          AND ($3::uuid IS NULL OR org_id = $3)"#
    )
    .bind(commit_sha)
    .bind(user_filter)
    .bind(org_filter)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    if let Some(ts) = written_ts {
        lifecycle.push(json!({
            "stage": "written",
            "timestamp": ts,
            "detail": if tool_desc.is_empty() { "AI coding session".into() } else { tool_desc },
        }));
    }

    // Stage 2: Committed
    let commit_ts: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        r#"SELECT MIN(to_timestamp(timestamp)) FROM metrics_events WHERE commit_sha = $1 AND event_type = 1
          AND ($2::uuid IS NULL OR user_id = $2)
          AND ($3::uuid IS NULL OR org_id = $3)"#
    )
    .bind(commit_sha)
    .bind(user_filter)
    .bind(org_filter)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    if let Some(ts) = commit_ts {
        lifecycle.push(json!({
            "stage": "committed",
            "timestamp": ts,
        }));
    }

    // Stage 3: CI events
    let ci_rows: Vec<(String, Option<chrono::DateTime<chrono::Utc>>, Option<String>, Option<String>)> = sqlx::query_as(
        r#"SELECT event_type, timestamp, deployment_env, status
        FROM ci_events WHERE commit_sha = $1
          AND ($2::uuid IS NULL OR org_id = $2)
        ORDER BY timestamp
        LIMIT $3"#
    )
    .bind(commit_sha)
    .bind(org_filter)
    .bind(LIFECYCLE_EVENT_FETCH_LIMIT)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let mut ci_rows = ci_rows;
    let ci_events_truncated = ci_rows.len() as i64 > LIFECYCLE_EVENT_LIMIT;
    if ci_events_truncated {
        ci_rows.truncate(LIFECYCLE_EVENT_LIMIT as usize);
    }

    for (event_type, ts, env, status) in &ci_rows {
        match event_type.as_str() {
            "deployment" => {
                lifecycle.push(json!({
                    "stage": "deployed",
                    "timestamp": ts,
                    "env": env,
                    "status": status,
                }));
            }
            "pr_review" => {
                lifecycle.push(json!({
                    "stage": "pr_reviewed",
                    "timestamp": ts,
                    "status": status,
                }));
            }
            _ => {
                lifecycle.push(json!({
                    "stage": event_type,
                    "timestamp": ts,
                    "status": status,
                }));
            }
        }
    }

    // Stage 4: Alerts
    let ai_code_involved_in_alert: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
            SELECT 1
            FROM alert_events
            WHERE commit_sha = $1
              AND ($2::uuid IS NULL OR org_id = $2)
              AND severity IN ('critical', 'warning')
        )"#,
    )
    .bind(commit_sha)
    .bind(org_filter)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let alert_rows: Vec<(String, Option<chrono::DateTime<chrono::Utc>>, String, Option<String>)> = sqlx::query_as(
        r#"SELECT alert_source, timestamp, severity, description
        FROM alert_events WHERE commit_sha = $1
          AND ($2::uuid IS NULL OR org_id = $2)
        ORDER BY timestamp
        LIMIT $3"#
    )
    .bind(commit_sha)
    .bind(org_filter)
    .bind(LIFECYCLE_EVENT_FETCH_LIMIT)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let mut alert_rows = alert_rows;
    let alert_events_truncated = alert_rows.len() as i64 > LIFECYCLE_EVENT_LIMIT;
    if alert_events_truncated {
        alert_rows.truncate(LIFECYCLE_EVENT_LIMIT as usize);
    }

    for (source, ts, severity, desc) in &alert_rows {
        lifecycle.push(json!({
            "stage": "alert",
            "timestamp": ts,
            "source": source,
            "severity": severity,
            "description": desc,
        }));
    }

    Ok(Json(json!({
        "commit_sha": commit_sha,
        "author": author,
        "ai_lines": ai_lines.unwrap_or(0),
        "human_lines": human_lines.unwrap_or(0),
        "lifecycle": lifecycle,
        "ai_code_involved_in_alert": ai_code_involved_in_alert,
        "truncated": ci_events_truncated || alert_events_truncated,
        "truncation": {
            "limit_per_event_type": LIFECYCLE_EVENT_LIMIT,
            "ci_events": ci_events_truncated,
            "alert_events": alert_events_truncated,
        },
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, MetricsRollupWriteMode};
    use crate::models::user::{AuthIdentity, AuthMethod};
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
                        "skipping lifecycle database test: could not connect to admin database: {error}"
                    );
                    return Ok(None);
                }
            };

            if let Err(error) = create_database(&admin_pool, &db_name).await {
                eprintln!(
                    "skipping lifecycle database test: could not create isolated database {db_name}: {error}"
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
    async fn pull_requests_cursor_paginates_and_keeps_full_summary() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id, org_slug) = insert_test_identity(&db.state.db).await?;
        let merged_at = fixed_timestamp("2026-07-09T10:00:00Z");
        let fixtures = [
            (uuid_tail(1), 50, 5, 45, 10.0),
            (uuid_tail(2), 150, 30, 120, 20.0),
            (uuid_tail(3), 600, 180, 420, 30.0),
            (uuid_tail(4), 10, 4, 6, 40.0),
            (uuid_tail(5), 700, 350, 350, 50.0),
        ];
        for (id, total, ai, human, pct) in fixtures {
            insert_pull_request(
                &db.state.db,
                id,
                org_id,
                "https://example.com/repo-a",
                &format!("pr-{id}"),
                Some(merged_at),
                total,
                ai,
                human,
                pct,
            )
            .await?;
        }
        insert_other_org_pull_request(&db.state.db, "https://example.com/repo-a").await?;

        let Json(first_page) = aggregate_pull_requests(
            State(db.state.clone()),
            auth_extractor(user_id, org_id, &org_slug),
            Query(PrAggregateQuery {
                org: Some(org_slug.clone()),
                repo: Some("https://example.com/repo-a".into()),
                since: Some("2026-07-01T00:00:00Z".into()),
                until: Some("2026-07-31T23:59:59Z".into()),
                limit: Some(2),
                cursor: None,
            }),
        )
        .await?;
        assert_eq!(
            object_ids(&first_page, "pull_requests"),
            vec![uuid_tail(5), uuid_tail(4)]
        );
        assert_eq!(first_page["pagination"]["has_more"].as_bool(), Some(true));
        assert_full_summary(&first_page);

        let cursor = required_next_cursor(&first_page);
        let Json(second_page) = aggregate_pull_requests(
            State(db.state.clone()),
            auth_extractor(user_id, org_id, &org_slug),
            Query(PrAggregateQuery {
                org: Some(org_slug.clone()),
                repo: Some("https://example.com/repo-a".into()),
                since: Some("2026-07-01T00:00:00Z".into()),
                until: Some("2026-07-31T23:59:59Z".into()),
                limit: Some(2),
                cursor: Some(cursor),
            }),
        )
        .await?;
        assert_eq!(
            object_ids(&second_page, "pull_requests"),
            vec![uuid_tail(3), uuid_tail(2)]
        );
        assert_full_summary(&second_page);

        let cursor = required_next_cursor(&second_page);
        let Json(third_page) = aggregate_pull_requests(
            State(db.state.clone()),
            auth_extractor(user_id, org_id, &org_slug),
            Query(PrAggregateQuery {
                org: Some(org_slug),
                repo: Some("https://example.com/repo-a".into()),
                since: Some("2026-07-01T00:00:00Z".into()),
                until: Some("2026-07-31T23:59:59Z".into()),
                limit: Some(2),
                cursor: Some(cursor),
            }),
        )
        .await?;
        assert_eq!(object_ids(&third_page, "pull_requests"), vec![uuid_tail(1)]);
        assert_eq!(third_page["pagination"]["has_more"].as_bool(), Some(false));
        assert_full_summary(&third_page);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pull_requests_cursor_pages_null_merged_at_after_dated_rows() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id, org_slug) = insert_test_identity(&db.state.db).await?;
        insert_pull_request(
            &db.state.db,
            uuid_tail(1),
            org_id,
            "https://example.com/repo-null",
            "dated",
            Some(fixed_timestamp("2026-07-09T10:00:00Z")),
            10,
            5,
            5,
            50.0,
        )
        .await?;
        insert_pull_request(
            &db.state.db,
            uuid_tail(2),
            org_id,
            "https://example.com/repo-null",
            "null-merged",
            None,
            20,
            5,
            15,
            25.0,
        )
        .await?;

        let Json(first_page) = aggregate_pull_requests(
            State(db.state.clone()),
            auth_extractor(user_id, org_id, &org_slug),
            Query(PrAggregateQuery {
                org: Some(org_slug.clone()),
                repo: Some("https://example.com/repo-null".into()),
                since: None,
                until: None,
                limit: Some(1),
                cursor: None,
            }),
        )
        .await?;
        assert_eq!(object_ids(&first_page, "pull_requests"), vec![uuid_tail(1)]);
        assert_eq!(first_page["pagination"]["has_more"].as_bool(), Some(true));

        let Json(second_page) = aggregate_pull_requests(
            State(db.state.clone()),
            auth_extractor(user_id, org_id, &org_slug),
            Query(PrAggregateQuery {
                org: Some(org_slug),
                repo: Some("https://example.com/repo-null".into()),
                since: None,
                until: None,
                limit: Some(1),
                cursor: Some(required_next_cursor(&first_page)),
            }),
        )
        .await?;
        assert_eq!(object_ids(&second_page, "pull_requests"), vec![uuid_tail(2)]);
        assert_eq!(second_page["pagination"]["has_more"].as_bool(), Some(false));

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn persistence_defaults_to_recent_year() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id, org_slug) = insert_test_identity(&db.state.db).await?;
        let today = chrono::Utc::now().date_naive();
        let old_date = today - chrono::Duration::days(PERSISTENCE_DEFAULT_DAYS + 35);
        let recent_date = today - chrono::Duration::days(10);

        insert_persistence_snapshot(
            &db.state.db,
            org_id,
            "https://example.com/persist",
            old_date,
            33.0,
        )
        .await?;
        insert_persistence_snapshot(
            &db.state.db,
            org_id,
            "https://example.com/persist",
            recent_date,
            81.5,
        )
        .await?;

        let Json(response) = get_ai_code_persistence(
            State(db.state.clone()),
            auth_extractor(user_id, org_id, &org_slug),
            Query(PersistenceQuery {
                org: Some(org_slug),
                repo: Some("https://example.com/persist".into()),
                since: None,
                until: None,
            }),
        )
        .await?;

        let expected_since = (today - chrono::Duration::days(PERSISTENCE_DEFAULT_DAYS)).to_string();
        let expected_until = today.to_string();
        let expected_snapshot_date = recent_date.to_string();
        assert_eq!(
            response["period"]["since"].as_str(),
            Some(expected_since.as_str())
        );
        assert_eq!(
            response["period"]["until"].as_str(),
            Some(expected_until.as_str())
        );
        assert_eq!(
            response["ai_code_snapshot"]["snapshot_date"].as_str(),
            Some(expected_snapshot_date.as_str())
        );
        let trend = response["trend"].as_array().expect("trend should be an array");
        assert_eq!(trend.len(), 1);
        assert_eq!(trend[0]["week"].as_str(), Some(expected_snapshot_date.as_str()));

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn lifecycle_truncates_ci_and_alert_events() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id, org_slug) = insert_test_identity(&db.state.db).await?;
        let commit_sha = "feedface";
        let base_ts = fixed_timestamp("2026-07-09T10:00:00Z");
        insert_lifecycle_metric_event(&db.state.db, user_id, org_id, commit_sha, base_ts).await?;

        for offset in 0..=LIFECYCLE_EVENT_LIMIT {
            let timestamp = base_ts + chrono::Duration::seconds(offset);
            insert_ci_event(&db.state.db, org_id, commit_sha, timestamp).await?;
            let severity = if offset == LIFECYCLE_EVENT_LIMIT {
                "warning"
            } else {
                "info"
            };
            insert_alert_event(&db.state.db, org_id, commit_sha, timestamp, severity).await?;
        }

        let Json(response) = get_ai_code_lifecycle(
            State(db.state.clone()),
            auth_extractor(user_id, org_id, &org_slug),
            Query(LifecycleQuery {
                org: Some(org_slug),
                commit_sha: Some(commit_sha.into()),
            }),
        )
        .await?;

        assert_eq!(response["truncated"].as_bool(), Some(true));
        assert_eq!(response["truncation"]["ci_events"].as_bool(), Some(true));
        assert_eq!(response["truncation"]["alert_events"].as_bool(), Some(true));
        assert_eq!(response["ai_code_involved_in_alert"].as_bool(), Some(true));
        assert_eq!(
            lifecycle_stage_count(&response, "ci_run"),
            LIFECYCLE_EVENT_LIMIT as usize
        );
        assert_eq!(
            lifecycle_stage_count(&response, "alert"),
            LIFECYCLE_EVENT_LIMIT as usize
        );

        db.cleanup().await?;
        Ok(())
    }

    async fn insert_test_identity(pool: &PgPool) -> anyhow::Result<(Uuid, Uuid, String)> {
        let user_id = Uuid::new_v4();
        let org_id = Uuid::new_v4();
        let org_slug = format!("lifecycle-test-{}", org_id.simple());

        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org_id)
            .bind("Lifecycle Test Org")
            .bind(&org_slug)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO users (id, email, name, default_org_id) VALUES ($1, $2, $3, $4)")
            .bind(user_id)
            .bind(format!("{user_id}@example.com"))
            .bind("Lifecycle Test User")
            .bind(org_id)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO org_members (user_id, org_id, role) VALUES ($1, $2, $3)")
            .bind(user_id)
            .bind(org_id)
            .bind("admin")
            .execute(pool)
            .await?;

        Ok((user_id, org_id, org_slug))
    }

    async fn insert_pull_request(
        pool: &PgPool,
        id: Uuid,
        org_id: Uuid,
        repo_url: &str,
        pr_id: &str,
        merged_at: Option<chrono::DateTime<chrono::Utc>>,
        total_lines: i32,
        ai_lines: i32,
        human_lines: i32,
        pct_ai: f32,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"INSERT INTO pull_requests (
                id, org_id, repo_url, pr_id, pr_url, title, author_email, merged_at,
                total_lines, ai_lines, human_lines, pct_ai, tools_used, files_changed, ai_files
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)"#,
        )
        .bind(id)
        .bind(org_id)
        .bind(repo_url)
        .bind(pr_id)
        .bind(format!("{repo_url}/pull/{pr_id}"))
        .bind(format!("PR {pr_id}"))
        .bind("dev@example.com")
        .bind(merged_at)
        .bind(total_lines)
        .bind(ai_lines)
        .bind(human_lines)
        .bind(pct_ai)
        .bind(vec!["codex::gpt-5".to_string()])
        .bind(2)
        .bind(1)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_other_org_pull_request(pool: &PgPool, repo_url: &str) -> anyhow::Result<()> {
        let org_id = Uuid::new_v4();
        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org_id)
            .bind("Other Lifecycle Test Org")
            .bind(format!("other-lifecycle-test-{}", org_id.simple()))
            .execute(pool)
            .await?;
        insert_pull_request(
            pool,
            uuid_tail(99),
            org_id,
            repo_url,
            "other-org",
            Some(fixed_timestamp("2026-07-09T10:00:00Z")),
            999,
            999,
            0,
            100.0,
        )
        .await
    }

    async fn insert_persistence_snapshot(
        pool: &PgPool,
        org_id: Uuid,
        repo_url: &str,
        snapshot_date: chrono::NaiveDate,
        survival_rate: f32,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"INSERT INTO ai_code_persistence_snapshots (
                org_id, repo_url, snapshot_date,
                total_ai_lines_introduced, lines_still_present, lines_modified, lines_deleted,
                survival_rate
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(org_id)
        .bind(repo_url)
        .bind(snapshot_date)
        .bind(100_i32)
        .bind(80_i32)
        .bind(10_i32)
        .bind(10_i32)
        .bind(survival_rate)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_lifecycle_metric_event(
        pool: &PgPool,
        user_id: Uuid,
        org_id: Uuid,
        commit_sha: &str,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"INSERT INTO metrics_events (
                event_type, timestamp, user_id, org_id, repo_url, author_email, tool, model,
                commit_sha, human_additions, ai_additions, git_diff_added_lines
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"#,
        )
        .bind(1_i16)
        .bind(timestamp.timestamp())
        .bind(user_id)
        .bind(org_id)
        .bind("https://example.com/lifecycle")
        .bind("dev@example.com")
        .bind("codex")
        .bind("gpt-5")
        .bind(commit_sha)
        .bind(3_i32)
        .bind(7_i32)
        .bind(10_i32)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_ci_event(
        pool: &PgPool,
        org_id: Uuid,
        commit_sha: &str,
        timestamp: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"INSERT INTO ci_events (org_id, event_type, timestamp, repo_url, commit_sha, status)
            VALUES ($1, $2, $3, $4, $5, $6)"#,
        )
        .bind(org_id)
        .bind("ci_run")
        .bind(timestamp)
        .bind("https://example.com/lifecycle")
        .bind(commit_sha)
        .bind("success")
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn insert_alert_event(
        pool: &PgPool,
        org_id: Uuid,
        commit_sha: &str,
        timestamp: chrono::DateTime<chrono::Utc>,
        severity: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"INSERT INTO alert_events (
                org_id, alert_source, event_type, timestamp, repo_url, commit_sha, severity,
                description
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
        )
        .bind(org_id)
        .bind("test")
        .bind("alert")
        .bind(timestamp)
        .bind("https://example.com/lifecycle")
        .bind(commit_sha)
        .bind(severity)
        .bind("test alert")
        .execute(pool)
        .await?;
        Ok(())
    }

    fn auth_extractor(user_id: Uuid, org_id: Uuid, org_slug: &str) -> AuthExtractor {
        AuthExtractor(AuthIdentity {
            user_id,
            email: format!("{user_id}@example.com"),
            name: "Lifecycle Test User".into(),
            org_id: Some(org_id),
            org_slug: Some(org_slug.to_string()),
            department_id: None,
            role: Some("admin".into()),
            scopes: vec![],
            auth_method: AuthMethod::BearerToken,
        })
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

    fn required_next_cursor(page: &Value) -> String {
        page["pagination"]["next_cursor"]
            .as_str()
            .expect("page should include next_cursor")
            .to_string()
    }

    fn lifecycle_stage_count(page: &Value, stage: &str) -> usize {
        page["lifecycle"]
            .as_array()
            .expect("lifecycle should be an array")
            .iter()
            .filter(|entry| entry["stage"].as_str() == Some(stage))
            .count()
    }

    fn assert_full_summary(page: &Value) {
        assert_eq!(page["summary"]["total_prs"].as_i64(), Some(5));
        assert_eq!(page["summary"]["avg_pct_ai"].as_f64(), Some(30.0));
        assert_eq!(page["summary"]["pr_size_distribution"]["small"].as_i64(), Some(2));
        assert_eq!(page["summary"]["pr_size_distribution"]["medium"].as_i64(), Some(1));
        assert_eq!(page["summary"]["pr_size_distribution"]["large"].as_i64(), Some(2));
    }

    fn fixed_timestamp(value: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn uuid_tail(value: u32) -> Uuid {
        Uuid::parse_str(&format!("00000000-0000-0000-0000-{value:012}")).unwrap()
    }

    fn test_config(database_url: &str) -> AppConfig {
        AppConfig {
            database_url: database_url.to_string(),
            database_max_connections: 20,
            database_min_connections: 1,
            database_acquire_timeout_seconds: 5,
            redis_url: "redis://127.0.0.1:6379".to_string(),
            jwt_secret: "lifecycle-test-secret".to_string(),
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
        format!("git_ai_lifecycle_test_{}", Uuid::new_v4().simple())
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
