use axum::extract::State;
use axum::response::Json;
use serde_json::Value;
use sqlx::{Postgres, QueryBuilder, Transaction};
use std::collections::{HashMap, HashSet};

use crate::auth::middleware::{AuthExtractor, HeaderExtractor as _HeaderExtractor};
use crate::error::AppError;
use crate::models::report::{
    ProjectSummaryReport, ReportCommit, ReportDocument, ToolModelBreakdown,
};
use crate::routes::AppState;

const REPORT_COMMIT_UPSERT_CHUNK_SIZE: usize = 1000;
const REPORT_TOOL_MODEL_UPSERT_CHUNK_SIZE: usize = 1000;

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

    let mut tx = state.db.begin().await.map_err(AppError::Database)?;

    // Upsert project (with org_id and user_id for data isolation)
    let project_row: (i64,) = sqlx::query_as(
        r#"INSERT INTO projects (remote_url_hash, branch, head_commit, org_id, user_id)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (remote_url_hash, org_id, user_id) DO UPDATE SET
            branch = COALESCE(EXCLUDED.branch, projects.branch),
            head_commit = COALESCE(EXCLUDED.head_commit, projects.head_commit),
            updated_at = now()
        RETURNING id"#,
    )
    .bind(&remote_url_hash)
    .bind(&branch)
    .bind(&head_commit)
    .bind(auth.0.org_id)
    .bind(auth.0.user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| AppError::Database(e))?;

    let project_id = project_row.0;

    // Create upload record
    let upload_row: (i64,) = sqlx::query_as(
        r#"INSERT INTO report_uploads (project_id, schema_version, generated_at, commit_count)
        VALUES ($1, $2, $3, $4)
        RETURNING id"#,
    )
    .bind(project_id)
    .bind(&report.schema_version)
    .bind(&report.generated_at)
    .bind(report.commits.len() as i64)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| AppError::Database(e))?;

    let upload_id = upload_row.0;
    let commit_summary = upsert_commit_stats(&mut tx, project_id, &report.commits).await?;
    upsert_tool_model_stats(&mut tx, project_id, report.tool_model_breakdown.as_ref()).await?;

    tx.commit().await.map_err(AppError::Database)?;

    Ok(Json(serde_json::json!({
        "project_id": project_id,
        "upload_id": upload_id,
        "inserted_commits": commit_summary.inserted_commits,
        "updated_commits": commit_summary.updated_commits,
        "duplicate_commits": commit_summary.updated_commits,
    })))
}

#[derive(Debug)]
struct CommitUpsertSummary {
    inserted_commits: i64,
    updated_commits: i64,
}

#[derive(Debug)]
struct PreparedCommitStat {
    sha: String,
    author: String,
    author_time: String,
    subject: String,
    has_authorship_note: bool,
    git_diff_added_lines: i64,
    git_diff_deleted_lines: i64,
    ai_additions: i64,
    human_additions: i64,
    mixed_additions: i64,
    unknown_additions: i64,
    ai_accepted: i64,
    total_ai_additions: i64,
    total_ai_deletions: i64,
    time_waiting_for_ai: i64,
}

impl PreparedCommitStat {
    fn from_commit(commit: &ReportCommit) -> Option<Self> {
        let sha = commit.sha.trim();
        if sha.is_empty() {
            return None;
        }

        let stats = &commit.stats;
        Some(Self {
            sha: sha.to_string(),
            author: commit.author.clone(),
            author_time: commit.author_time.clone(),
            subject: commit.subject.clone(),
            has_authorship_note: commit.has_authorship_note,
            git_diff_added_lines: stats.git_diff_added_lines.unwrap_or(0),
            git_diff_deleted_lines: stats.git_diff_deleted_lines.unwrap_or(0),
            ai_additions: stats.ai_additions.unwrap_or(0),
            human_additions: stats.human_additions.unwrap_or(0),
            mixed_additions: stats.mixed_additions.unwrap_or(0),
            unknown_additions: stats.unknown_additions.unwrap_or(0),
            ai_accepted: stats.ai_accepted.unwrap_or(0),
            total_ai_additions: stats.total_ai_additions.unwrap_or(0),
            total_ai_deletions: stats.total_ai_deletions.unwrap_or(0),
            time_waiting_for_ai: stats.time_waiting_for_ai.unwrap_or(0),
        })
    }
}

#[derive(Debug)]
struct PreparedToolModelStat {
    tool_model: String,
    ai_additions: i64,
    mixed_additions: i64,
    ai_accepted: i64,
    total_ai_additions: i64,
    total_ai_deletions: i64,
    time_waiting_for_ai: i64,
}

impl PreparedToolModelStat {
    fn from_breakdown(tool_model: &str, stats: &ToolModelBreakdown) -> Self {
        Self {
            tool_model: tool_model.to_string(),
            ai_additions: stats.ai_additions.unwrap_or(0),
            mixed_additions: stats.mixed_additions.unwrap_or(0),
            ai_accepted: stats.ai_accepted.unwrap_or(0),
            total_ai_additions: stats.total_ai_additions.unwrap_or(0),
            total_ai_deletions: stats.total_ai_deletions.unwrap_or(0),
            time_waiting_for_ai: stats.time_waiting_for_ai.unwrap_or(0),
        }
    }
}

async fn upsert_commit_stats(
    tx: &mut Transaction<'_, Postgres>,
    project_id: i64,
    commits: &[ReportCommit],
) -> Result<CommitUpsertSummary, AppError> {
    let rows = prepare_commit_stats(commits);
    if rows.is_empty() {
        return Ok(CommitUpsertSummary {
            inserted_commits: 0,
            updated_commits: 0,
        });
    }

    let shas: Vec<String> = rows.iter().map(|row| row.sha.clone()).collect();
    let existing_shas = fetch_existing_commit_shas(tx, project_id, &shas).await?;

    for chunk in rows.chunks(REPORT_COMMIT_UPSERT_CHUNK_SIZE) {
        upsert_commit_stats_chunk(tx, project_id, chunk).await?;
    }

    let updated_commits = rows
        .iter()
        .filter(|row| existing_shas.contains(&row.sha))
        .count() as i64;
    let inserted_commits = rows.len() as i64 - updated_commits;

    Ok(CommitUpsertSummary {
        inserted_commits,
        updated_commits,
    })
}

fn prepare_commit_stats(commits: &[ReportCommit]) -> Vec<PreparedCommitStat> {
    let mut rows = Vec::new();
    let mut index_by_sha = HashMap::new();

    for commit in commits {
        let Some(row) = PreparedCommitStat::from_commit(commit) else {
            continue;
        };

        if let Some(index) = index_by_sha.get(&row.sha).copied() {
            rows[index] = row;
        } else {
            index_by_sha.insert(row.sha.clone(), rows.len());
            rows.push(row);
        }
    }

    rows
}

async fn fetch_existing_commit_shas(
    tx: &mut Transaction<'_, Postgres>,
    project_id: i64,
    shas: &[String],
) -> Result<HashSet<String>, AppError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT sha FROM commit_stats WHERE project_id = $1 AND sha = ANY($2)")
            .bind(project_id)
            .bind(shas)
            .fetch_all(&mut **tx)
            .await
            .map_err(AppError::Database)?;

    Ok(rows.into_iter().map(|(sha,)| sha).collect())
}

async fn upsert_commit_stats_chunk(
    tx: &mut Transaction<'_, Postgres>,
    project_id: i64,
    rows: &[PreparedCommitStat],
) -> Result<(), AppError> {
    let mut builder: QueryBuilder<Postgres> = QueryBuilder::new(
        r#"INSERT INTO commit_stats (
            project_id, sha, author, author_time, subject, has_authorship_note,
            git_diff_added_lines, git_diff_deleted_lines, ai_additions, human_additions,
            mixed_additions, unknown_additions, ai_accepted, total_ai_additions,
            total_ai_deletions, time_waiting_for_ai
        ) "#,
    );

    builder.push_values(rows, |mut row_builder, row| {
        row_builder
            .push_bind(project_id)
            .push_bind(&row.sha)
            .push_bind(&row.author)
            .push_bind(&row.author_time)
            .push_bind(&row.subject)
            .push_bind(row.has_authorship_note)
            .push_bind(row.git_diff_added_lines)
            .push_bind(row.git_diff_deleted_lines)
            .push_bind(row.ai_additions)
            .push_bind(row.human_additions)
            .push_bind(row.mixed_additions)
            .push_bind(row.unknown_additions)
            .push_bind(row.ai_accepted)
            .push_bind(row.total_ai_additions)
            .push_bind(row.total_ai_deletions)
            .push_bind(row.time_waiting_for_ai);
    });

    builder.push(
        r#" ON CONFLICT (project_id, sha) DO UPDATE SET
            author = EXCLUDED.author,
            author_time = EXCLUDED.author_time,
            subject = EXCLUDED.subject,
            has_authorship_note = EXCLUDED.has_authorship_note,
            git_diff_added_lines = EXCLUDED.git_diff_added_lines,
            git_diff_deleted_lines = EXCLUDED.git_diff_deleted_lines,
            ai_additions = EXCLUDED.ai_additions,
            human_additions = EXCLUDED.human_additions,
            mixed_additions = EXCLUDED.mixed_additions,
            unknown_additions = EXCLUDED.unknown_additions,
            ai_accepted = EXCLUDED.ai_accepted,
            total_ai_additions = EXCLUDED.total_ai_additions,
            total_ai_deletions = EXCLUDED.total_ai_deletions,
            time_waiting_for_ai = EXCLUDED.time_waiting_for_ai"#,
    );

    builder
        .build()
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

    Ok(())
}

async fn upsert_tool_model_stats(
    tx: &mut Transaction<'_, Postgres>,
    project_id: i64,
    breakdown: Option<&HashMap<String, ToolModelBreakdown>>,
) -> Result<(), AppError> {
    let Some(breakdown) = breakdown else {
        return Ok(());
    };

    let rows: Vec<PreparedToolModelStat> = breakdown
        .iter()
        .map(|(tool_model, stats)| PreparedToolModelStat::from_breakdown(tool_model, stats))
        .collect();

    for chunk in rows.chunks(REPORT_TOOL_MODEL_UPSERT_CHUNK_SIZE) {
        upsert_tool_model_stats_chunk(tx, project_id, chunk).await?;
    }

    Ok(())
}

async fn upsert_tool_model_stats_chunk(
    tx: &mut Transaction<'_, Postgres>,
    project_id: i64,
    rows: &[PreparedToolModelStat],
) -> Result<(), AppError> {
    if rows.is_empty() {
        return Ok(());
    }

    let mut builder: QueryBuilder<Postgres> = QueryBuilder::new(
        r#"INSERT INTO tool_model_stats (
            project_id, tool_model, ai_additions, mixed_additions,
            ai_accepted, total_ai_additions, total_ai_deletions, time_waiting_for_ai
        ) "#,
    );

    builder.push_values(rows, |mut row_builder, row| {
        row_builder
            .push_bind(project_id)
            .push_bind(&row.tool_model)
            .push_bind(row.ai_additions)
            .push_bind(row.mixed_additions)
            .push_bind(row.ai_accepted)
            .push_bind(row.total_ai_additions)
            .push_bind(row.total_ai_deletions)
            .push_bind(row.time_waiting_for_ai);
    });

    builder.push(
        r#" ON CONFLICT (project_id, tool_model) DO UPDATE SET
            ai_additions = EXCLUDED.ai_additions,
            mixed_additions = EXCLUDED.mixed_additions,
            ai_accepted = EXCLUDED.ai_accepted,
            total_ai_additions = EXCLUDED.total_ai_additions,
            total_ai_deletions = EXCLUDED.total_ai_deletions,
            time_waiting_for_ai = EXCLUDED.time_waiting_for_ai"#,
    );

    builder
        .build()
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

    Ok(())
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
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::middleware::HeaderExtractor;
    use crate::models::report::{ReportCommit, ReportCommitStats, ReportRepo, ToolModelBreakdown};
    use crate::models::user::{AuthIdentity, AuthMethod, RequestHeaders};
    use sqlx::PgPool;
    use sqlx::postgres::PgPoolOptions;
    use std::collections::HashMap;
    use uuid::Uuid;

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
                    eprintln!("skipping report test: could not connect to admin database: {error}");
                    return Ok(None);
                }
            };

            if let Err(error) = create_database(&admin_pool, &db_name).await {
                eprintln!(
                    "skipping report test: could not create isolated database {db_name}: {error}"
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
            let cas_store = crate::services::cas::CasStore::new(&config)?;
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
    async fn upload_report_rolls_back_when_tool_model_stats_fails() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let report = report_document(
            "rollback-repo",
            report_commit("rollback-sha", "Rollback Author", "first subject", stats(1)),
            Some(tool_model_breakdown(
                "Codex gpt-5",
                ToolModelBreakdown {
                    ai_additions: Some(i64::from(i32::MAX) + 1),
                    ..tool_stats(1)
                },
            )),
        );

        let result = upload_report(
            State(db.state.clone()),
            auth_extractor(user_id, org_id),
            HeaderExtractor(RequestHeaders::default()),
            Json(report),
        )
        .await;

        assert!(matches!(result, Err(AppError::Database(_))));
        assert_eq!(
            table_count(
                &db.state.db,
                "projects",
                Some("remote_url_hash = 'rollback-repo'")
            )
            .await?,
            0
        );
        assert_eq!(table_count(&db.state.db, "report_uploads", None).await?, 0);
        assert_eq!(table_count(&db.state.db, "commit_stats", None).await?, 0);
        assert_eq!(
            table_count(&db.state.db, "tool_model_stats", None).await?,
            0
        );

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_report_updates_existing_commit_and_tool_model_stats() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;

        let first = upload_report(
            State(db.state.clone()),
            auth_extractor(user_id, org_id),
            HeaderExtractor(RequestHeaders::default()),
            Json(report_document(
                "upsert-repo",
                report_commit("same-sha", "First Author", "first subject", stats(1)),
                Some(tool_model_breakdown("Codex gpt-5", tool_stats(10))),
            )),
        )
        .await?;
        assert_eq!(first.0["inserted_commits"], 1);
        assert_eq!(first.0["updated_commits"], 0);

        let second = upload_report(
            State(db.state.clone()),
            auth_extractor(user_id, org_id),
            HeaderExtractor(RequestHeaders::default()),
            Json(report_document(
                "upsert-repo",
                report_commit("same-sha", "Second Author", "second subject", stats(2)),
                Some(tool_model_breakdown("Codex gpt-5", tool_stats(20))),
            )),
        )
        .await?;

        assert_eq!(second.0["inserted_commits"], 0);
        assert_eq!(second.0["updated_commits"], 1);
        assert_eq!(second.0["duplicate_commits"], 1);

        let project_id = second.0["project_id"]
            .as_i64()
            .expect("project_id should be numeric");
        let commit_row: (
            String,
            String,
            bool,
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
            i64,
        ) = sqlx::query_as(
            "SELECT author, subject, has_authorship_note, \
                    git_diff_added_lines::bigint, git_diff_deleted_lines::bigint, \
                    ai_additions::bigint, human_additions::bigint, mixed_additions::bigint, \
                    unknown_additions::bigint, ai_accepted::bigint, \
                    total_ai_additions::bigint, total_ai_deletions::bigint \
                 FROM commit_stats WHERE project_id = $1 AND sha = $2",
        )
        .bind(project_id)
        .bind("same-sha")
        .fetch_one(&db.state.db)
        .await?;
        assert_eq!(
            commit_row,
            (
                "Second Author".into(),
                "second subject".into(),
                true,
                21,
                22,
                23,
                24,
                25,
                26,
                27,
                28,
                29,
            )
        );

        let tool_row: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT ai_additions::bigint, mixed_additions::bigint, ai_accepted::bigint, \
                total_ai_additions::bigint, total_ai_deletions::bigint, time_waiting_for_ai::bigint \
             FROM tool_model_stats WHERE project_id = $1 AND tool_model = $2",
        )
        .bind(project_id)
        .bind("Codex gpt-5")
        .fetch_one(&db.state.db)
        .await?;
        assert_eq!(tool_row, (201, 202, 203, 204, 205, 206));

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn upload_report_bulk_upserts_large_report_in_chunks() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.state.db).await?;
        let commits: Vec<ReportCommit> = (0..=REPORT_COMMIT_UPSERT_CHUNK_SIZE)
            .map(|idx| {
                report_commit(
                    &format!("large-sha-{idx:04}"),
                    "Bulk Author",
                    &format!("bulk subject {idx}"),
                    stats(idx as i64),
                )
            })
            .collect();
        let breakdown = Some(HashMap::from([
            ("Codex gpt-5".to_string(), tool_stats(1)),
            ("Cursor claude-sonnet".to_string(), tool_stats(2)),
        ]));

        let response = upload_report(
            State(db.state.clone()),
            auth_extractor(user_id, org_id),
            HeaderExtractor(RequestHeaders::default()),
            Json(report_document_with_commits(
                "large-report-repo",
                commits,
                breakdown,
            )),
        )
        .await?;

        assert_eq!(
            response.0["inserted_commits"],
            (REPORT_COMMIT_UPSERT_CHUNK_SIZE + 1) as i64
        );
        assert_eq!(response.0["updated_commits"], 0);
        assert_eq!(
            table_count(&db.state.db, "commit_stats", None).await?,
            (REPORT_COMMIT_UPSERT_CHUNK_SIZE + 1) as i64
        );
        assert_eq!(
            table_count(&db.state.db, "tool_model_stats", None).await?,
            2
        );

        db.cleanup().await?;
        Ok(())
    }

    fn report_document(
        remote_url_hash: &str,
        commit: ReportCommit,
        tool_model_breakdown: Option<HashMap<String, ToolModelBreakdown>>,
    ) -> ReportDocument {
        report_document_with_commits(remote_url_hash, vec![commit], tool_model_breakdown)
    }

    fn report_document_with_commits(
        remote_url_hash: &str,
        commits: Vec<ReportCommit>,
        tool_model_breakdown: Option<HashMap<String, ToolModelBreakdown>>,
    ) -> ReportDocument {
        let head_commit = commits.last().map(|commit| commit.sha.clone());
        ReportDocument {
            schema_version: "3.0.0".into(),
            generated_at: "2026-07-07T00:00:00Z".into(),
            tool_version: "test".into(),
            repo: Some(ReportRepo {
                workdir: Some("/tmp/repo".into()),
                remote_url_hash: Some(remote_url_hash.into()),
                branch: Some("main".into()),
                head_commit,
            }),
            range: None,
            summary: None,
            ratios: None,
            tool_model_breakdown,
            commits,
        }
    }

    fn report_commit(
        sha: &str,
        author: &str,
        subject: &str,
        stats: ReportCommitStats,
    ) -> ReportCommit {
        ReportCommit {
            sha: sha.into(),
            author: author.into(),
            author_time: "2026-07-07T00:00:00Z".into(),
            subject: subject.into(),
            has_authorship_note: true,
            stats,
        }
    }

    fn stats(seed: i64) -> ReportCommitStats {
        ReportCommitStats {
            git_diff_added_lines: Some(seed * 10 + 1),
            git_diff_deleted_lines: Some(seed * 10 + 2),
            ai_additions: Some(seed * 10 + 3),
            human_additions: Some(seed * 10 + 4),
            mixed_additions: Some(seed * 10 + 5),
            unknown_additions: Some(seed * 10 + 6),
            ai_accepted: Some(seed * 10 + 7),
            total_ai_additions: Some(seed * 10 + 8),
            total_ai_deletions: Some(seed * 10 + 9),
            time_waiting_for_ai: Some(seed * 10 + 10),
        }
    }

    fn tool_model_breakdown(
        tool_model: &str,
        stats: ToolModelBreakdown,
    ) -> HashMap<String, ToolModelBreakdown> {
        HashMap::from([(tool_model.into(), stats)])
    }

    fn tool_stats(seed: i64) -> ToolModelBreakdown {
        ToolModelBreakdown {
            ai_additions: Some(seed * 10 + 1),
            human_additions: Some(seed * 10),
            mixed_additions: Some(seed * 10 + 2),
            ai_accepted: Some(seed * 10 + 3),
            total_ai_additions: Some(seed * 10 + 4),
            total_ai_deletions: Some(seed * 10 + 5),
            time_waiting_for_ai: Some(seed * 10 + 6),
        }
    }

    async fn insert_test_identity(pool: &PgPool) -> anyhow::Result<(Uuid, Uuid)> {
        let user_id = Uuid::new_v4();
        let org_id = Uuid::new_v4();

        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org_id)
            .bind("Report Test Org")
            .bind(format!("report-test-{}", org_id.simple()))
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO users (id, email, name, default_org_id) VALUES ($1, $2, $3, $4)")
            .bind(user_id)
            .bind(format!("{user_id}@example.com"))
            .bind("Report Test User")
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
            name: "Report Test User".into(),
            org_id: Some(org_id),
            org_slug: Some(format!("report-test-{}", org_id.simple())),
            department_id: None,
            role: Some("member".into()),
            scopes: vec!["reports:write".into()],
            auth_method: AuthMethod::ApiKey,
        })
    }

    async fn table_count(
        pool: &PgPool,
        table: &str,
        where_clause: Option<&str>,
    ) -> anyhow::Result<i64> {
        let sql = match where_clause {
            Some(where_clause) => format!("SELECT COUNT(*) FROM {table} WHERE {where_clause}"),
            None => format!("SELECT COUNT(*) FROM {table}"),
        };
        Ok(sqlx::query_scalar(&sql).fetch_one(pool).await?)
    }

    fn test_config(database_url: &str) -> crate::config::AppConfig {
        crate::config::AppConfig {
            database_url: database_url.to_string(),
            database_max_connections: 20,
            database_min_connections: 1,
            database_acquire_timeout_seconds: 5,
            redis_url: "redis://127.0.0.1:6379".to_string(),
            jwt_secret: "report-test-secret".to_string(),
            s3_endpoint: "http://localhost:9000".to_string(),
            s3_bucket: "git-ai-cas".to_string(),
            s3_access_key: "minioadmin".to_string(),
            s3_secret_key: "minioadmin".to_string(),
            s3_region: "us-east-1".to_string(),
            cas_upload_concurrency: 8,
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
        format!("git_ai_report_test_{}", Uuid::new_v4().simple())
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
