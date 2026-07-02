//! Phase 6: Enterprise lifecycle APIs
//!
//! PR-level aggregation, AI code persistence tracking, agent readiness, and lifecycle queries.

use axum::extract::{Query, State};
use axum::response::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::middleware::AuthExtractor;
use crate::error::AppError;
use crate::routes::AppState;

// ================================================================
// PR-level aggregation
// ================================================================

#[derive(Debug, Deserialize)]
pub struct PrAggregateQuery {
    pub org: Option<String>,
    pub repo: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
}

/// GET /api/v1/aggregate/pull-requests — PR-level AI attribution aggregation
pub async fn aggregate_pull_requests(
    State(state): State<AppState>,
    auth: AuthExtractor,
    Query(query): Query<PrAggregateQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = crate::handlers::dashboard::build_data_filters(&auth.0);

    let rows: Vec<(Uuid, Option<Uuid>, String, String, Option<String>, Option<String>, Option<String>, Option<chrono::DateTime<chrono::Utc>>, i32, i32, i32, f32, Option<Vec<String>>, i32, i32)> = sqlx::query_as(
        r#"SELECT id, org_id, repo_url, pr_id, pr_url, title, author_email, merged_at,
                  total_lines, ai_lines, human_lines, pct_ai, tools_used, files_changed, ai_files
        FROM pull_requests
        WHERE ($1::text IS NULL OR repo_url = $1)
          AND ($2::text IS NULL OR org_id = (SELECT id FROM organizations WHERE slug = $2))
          AND ($3::timestamptz IS NULL OR merged_at >= $3)
          AND ($4::timestamptz IS NULL OR merged_at <= $4)
          AND ($5::uuid IS NULL OR org_id = $5)
        ORDER BY merged_at DESC"#
    )
    .bind(&query.repo)
    .bind(&query.org)
    .bind(&query.since)
    .bind(&query.until)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

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

    // Summary
    let total_prs = prs.len() as i64;
    let avg_pct_ai = if !prs.is_empty() {
        prs.iter().map(|p| p.get("pct_ai").and_then(|v| v.as_f64()).unwrap_or(0.0))
            .sum::<f64>() / prs.len() as f64
    } else {
        0.0
    };

    // Size distribution
    let mut small = 0i64;
    let mut medium = 0i64;
    let mut large = 0i64;
    for pr in &prs {
        let lines = pr.get("total_lines").and_then(|v| v.as_i64()).unwrap_or(0);
        if lines <= 100 { small += 1; }
        else if lines <= 500 { medium += 1; }
        else { large += 1; }
    }

    Ok(Json(json!({
        "pull_requests": prs,
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
}

/// GET /api/v1/ai-code-persistence — AI code survival rate tracking
pub async fn get_ai_code_persistence(
    State(state): State<AppState>,
    auth: AuthExtractor,
    Query(query): Query<PersistenceQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = crate::handlers::dashboard::build_data_filters(&auth.0);

    // Get the latest snapshot
    let snapshot: Option<(Uuid, Option<Uuid>, String, chrono::NaiveDate, i32, i32, i32, i32, f32, Option<serde_json::Value>)> = sqlx::query_as(
        r#"SELECT id, org_id, repo_url, snapshot_date,
                  total_ai_lines_introduced, lines_still_present, lines_modified, lines_deleted, survival_rate, by_tool
        FROM ai_code_persistence_snapshots
        WHERE ($1::text IS NULL OR repo_url = $1)
          AND ($2::text IS NULL OR org_id = (SELECT id FROM organizations WHERE slug = $2))
          AND ($3::date IS NULL OR snapshot_date >= $3)
          AND ($4::uuid IS NULL OR org_id = $4)
        ORDER BY snapshot_date DESC
        LIMIT 1"#
    )
    .bind(&query.repo)
    .bind(&query.org)
    .bind(&query.since)
    .bind(org_filter)
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
        ORDER BY snapshot_date"#
    )
    .bind(&query.repo)
    .bind(&query.org)
    .bind(org_filter)
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
              AND ($2::uuid IS NULL OR org_id = $2)"#
        )
        .bind(user_filter)
        .bind(org_filter)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

        let (ai_lines, _human_lines) = fallback.unwrap_or((Some(0), Some(0)));

        return Ok(Json(json!({
            "period": { "since": query.since, "until": Option::<String>::None },
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
        "period": { "since": query.since, "until": Option::<String>::None },
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
        r#"SELECT author_email, ai_additions, human_additions
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
        r#"SELECT MIN(timestamp) FROM metrics_events WHERE commit_sha = $1 AND event_type = 1
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
        ORDER BY timestamp"#
    )
    .bind(commit_sha)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

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
    let alert_rows: Vec<(String, Option<chrono::DateTime<chrono::Utc>>, String, Option<String>)> = sqlx::query_as(
        r#"SELECT alert_source, timestamp, severity, description
        FROM alert_events WHERE commit_sha = $1
          AND ($2::uuid IS NULL OR org_id = $2)
        ORDER BY timestamp"#
    )
    .bind(commit_sha)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let mut ai_code_involved_in_alert = false;
    for (source, ts, severity, desc) in &alert_rows {
        if severity == "critical" || severity == "warning" {
            ai_code_involved_in_alert = true;
        }
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
    })))
}
