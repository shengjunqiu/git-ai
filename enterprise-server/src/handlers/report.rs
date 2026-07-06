use axum::extract::State;
use axum::response::Json;
use serde_json::Value;

use crate::auth::middleware::{AuthExtractor, HeaderExtractor as _HeaderExtractor};
use crate::error::AppError;
use crate::models::report::{ProjectSummaryReport, ReportDocument};
use crate::routes::AppState;

/// POST /api/v1/reports — Upload report data
pub async fn upload_report(
    State(state): State<AppState>,
    auth: AuthExtractor,
    _headers: _HeaderExtractor,
    Json(report): Json<ReportDocument>,
) -> Result<Json<Value>, AppError> {
    tracing::info!(
        "Report upload: schema={}, tool_version={}",
        report.schema_version,
        report.tool_version,
    );

    let remote_url_hash = report
        .repo
        .as_ref()
        .and_then(|r| r.remote_url_hash.clone())
        .unwrap_or_else(|| "unknown".to_string());

    let branch = report.repo.as_ref().and_then(|r| r.branch.clone());
    let head_commit = report.repo.as_ref().and_then(|r| r.head_commit.clone());

    // Upsert project (with org_id and user_id for data isolation)
    let project_row: (i64,) = sqlx::query_as(
        r#"INSERT INTO projects (remote_url_hash, branch, head_commit, org_id, user_id)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (remote_url_hash, org_id, user_id) DO UPDATE SET
            branch = COALESCE(EXCLUDED.branch, projects.branch),
            head_commit = COALESCE(EXCLUDED.head_commit, projects.head_commit),
            updated_at = now()
        RETURNING id"#
    )
    .bind(&remote_url_hash)
    .bind(&branch)
    .bind(&head_commit)
    .bind(auth.0.org_id)
    .bind(auth.0.user_id)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let project_id = project_row.0;

    // Create upload record
    let upload_row: (i64,) = sqlx::query_as(
        r#"INSERT INTO report_uploads (project_id, schema_version, generated_at, commit_count)
        VALUES ($1, $2, $3, $4)
        RETURNING id"#
    )
    .bind(project_id)
    .bind(&report.schema_version)
    .bind(&report.generated_at)
    .bind(report.commits.len() as i64)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let upload_id = upload_row.0;
    let mut inserted_commits = 0i64;
    let mut duplicate_commits = 0i64;

    for commit in &report.commits {
        if commit.sha.trim().is_empty() {
            continue;
        }

        let stats = &commit.stats;
        let result = sqlx::query(
            r#"INSERT INTO commit_stats (
                project_id, sha, author, author_time, subject, has_authorship_note,
                git_diff_added_lines, git_diff_deleted_lines, ai_additions, human_additions,
                mixed_additions, unknown_additions, ai_accepted, total_ai_additions,
                total_ai_deletions, time_waiting_for_ai
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            ON CONFLICT (project_id, sha) DO NOTHING"#,
        )
        .bind(project_id)
        .bind(&commit.sha)
        .bind(&commit.author)
        .bind(&commit.author_time)
        .bind(&commit.subject)
        .bind(commit.has_authorship_note)
        .bind(stats.git_diff_added_lines.unwrap_or(0))
        .bind(stats.git_diff_deleted_lines.unwrap_or(0))
        .bind(stats.ai_additions.unwrap_or(0))
        .bind(stats.human_additions.unwrap_or(0))
        .bind(stats.mixed_additions.unwrap_or(0))
        .bind(stats.unknown_additions.unwrap_or(0))
        .bind(stats.ai_accepted.unwrap_or(0))
        .bind(stats.total_ai_additions.unwrap_or(0))
        .bind(stats.total_ai_deletions.unwrap_or(0))
        .bind(stats.time_waiting_for_ai.unwrap_or(0))
        .execute(&state.db)
        .await
        .map_err(AppError::Database)?;

        if result.rows_affected() == 0 {
            duplicate_commits += 1;
        } else {
            inserted_commits += 1;
        }
    }

    // Store tool_model_breakdown
    if let Some(breakdown) = &report.tool_model_breakdown {
        for (tool_model, stats) in breakdown {
            sqlx::query(
                r#"INSERT INTO tool_model_stats (
                    project_id, tool_model, ai_additions, mixed_additions,
                    ai_accepted, total_ai_additions, total_ai_deletions, time_waiting_for_ai
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                ON CONFLICT (project_id, tool_model) DO UPDATE SET
                    ai_additions = EXCLUDED.ai_additions,
                    mixed_additions = EXCLUDED.mixed_additions"#
            )
            .bind(project_id)
            .bind(tool_model)
            .bind(stats.ai_additions.unwrap_or(0))
            .bind(stats.mixed_additions.unwrap_or(0))
            .bind(stats.ai_accepted.unwrap_or(0))
            .bind(stats.total_ai_additions.unwrap_or(0))
            .bind(stats.total_ai_deletions.unwrap_or(0))
            .bind(stats.time_waiting_for_ai.unwrap_or(0))
            .execute(&state.db)
            .await
            .ok();
        }
    }

    Ok(Json(serde_json::json!({
        "project_id": project_id,
        "upload_id": upload_id,
        "inserted_commits": inserted_commits,
        "duplicate_commits": duplicate_commits,
    })))
}

/// POST /api/v1/summaries — Upload summary data (no auth required!)
pub async fn upload_summary(
    State(state): State<AppState>,
    Json(summary): Json<ProjectSummaryReport>,
) -> Result<Json<Value>, AppError> {
    tracing::info!(
        "Summary upload: project={}, commits={}",
        summary.project_name,
        summary.total_commits,
    );

    let ratios_json = serde_json::to_value(&summary.project_ratios)
        .map_err(|e| AppError::BadRequest(format!("Invalid project_ratios: {}", e)))?;
    let developers_json = serde_json::to_value(&summary.developers)
        .map_err(|e| AppError::BadRequest(format!("Invalid developers: {}", e)))?;

    sqlx::query(
        r#"INSERT INTO summary_uploads (
            project_name, git_url, branch, total_commits,
            organization, department, reporter_name, reporter_email,
            report_period, project_ratios, developers
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#
    )
    .bind(&summary.project_name)
    .bind(&summary.git_url)
    .bind(&summary.branch)
    .bind(summary.total_commits)
    .bind(&summary.organization)
    .bind(&summary.department)
    .bind(&summary.reporter_name)
    .bind(&summary.reporter_email)
    .bind(&summary.report_period)
    .bind(&ratios_json)
    .bind(&developers_json)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    Ok(Json(serde_json::json!({ "status": "ok" })))
}
