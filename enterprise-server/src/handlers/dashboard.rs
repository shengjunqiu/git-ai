use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Json, Redirect};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Component, PathBuf};
use url::Url;
use uuid::Uuid;

use crate::auth::middleware::{DashboardAuth, OptionalAuth};
use crate::error::AppError;
use crate::routes::AppState;

/// GET /me — Dashboard home page
pub async fn dashboard_me(State(_state): State<AppState>, auth: OptionalAuth) -> impl IntoResponse {
    // If not authenticated, redirect to login page
    let auth = match auth.0 {
        Some(a) => a,
        None => return Redirect::to("/auth/login?return_to=/me").into_response(),
    };

    match render_dashboard_template(&auth) {
        Ok(html) => Html(html).into_response(),
        Err(error) => error.into_response(),
    }
}

/// GET /static/*path — Dashboard static assets.
pub async fn dashboard_static_asset(Path(asset_path): Path<String>) -> impl IntoResponse {
    let relative_path = match sanitize_static_asset_path(&asset_path) {
        Some(path) => path,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let static_dir = match dashboard_static_dir() {
        Ok(path) => path,
        Err(error) => return error.into_response(),
    };
    let file_path = static_dir.join(relative_path);
    let bytes = match std::fs::read(&file_path) {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };

    let content_type = dashboard_asset_content_type(&file_path);
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        bytes,
    )
        .into_response()
}

fn render_dashboard_template(auth: &crate::models::user::AuthIdentity) -> Result<String, AppError> {
    let template_path = dashboard_template_path()?;
    let template = std::fs::read_to_string(&template_path).map_err(|error| {
        AppError::Internal(format!(
            "Failed to read dashboard template {}: {}",
            template_path.display(),
            error
        ))
    })?;

    let is_admin = auth.is_admin();
    let user_initial = auth
        .name
        .chars()
        .find(|ch| !ch.is_whitespace())
        .unwrap_or('G')
        .to_string();
    let user_role_label = if is_admin { "管理员" } else { "开发者" };

    Ok(template
        .replace(
            "__GITAI_IS_ADMIN__",
            if is_admin { "true" } else { "false" },
        )
        .replace(
            "__GITAI_CURRENT_USER_ID_JSON__",
            &json_string_literal(&auth.user_id.to_string()),
        )
        .replace("__GITAI_USER_NAME__", &html_escape(&auth.name))
        .replace("__GITAI_USER_NAME_JSON__", &json_string_literal(&auth.name))
        .replace("__GITAI_USER_EMAIL__", &html_escape(&auth.email))
        .replace(
            "__GITAI_USER_EMAIL_JSON__",
            &json_string_literal(&auth.email),
        )
        .replace("__GITAI_USER_INITIAL__", &html_escape(&user_initial))
        .replace("__GITAI_USER_ROLE_LABEL__", &html_escape(user_role_label)))
}

pub fn dashboard_static_dir() -> Result<std::path::PathBuf, AppError> {
    if let Ok(path) = std::env::var("GIT_AI_DASHBOARD_TEMPLATE") {
        return std::path::PathBuf::from(path)
            .parent()
            .map(std::path::Path::to_path_buf)
            .ok_or_else(|| AppError::Internal("Invalid dashboard template path".to_string()));
    }

    let current_dir_path = std::env::current_dir()
        .map_err(|error| {
            AppError::Internal(format!("Failed to resolve current directory: {}", error))
        })?
        .join("static");
    if current_dir_path.join("dashboard.html").exists() {
        return Ok(current_dir_path);
    }

    Ok(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static"))
}

fn dashboard_template_path() -> Result<std::path::PathBuf, AppError> {
    if let Ok(path) = std::env::var("GIT_AI_DASHBOARD_TEMPLATE") {
        return Ok(std::path::PathBuf::from(path));
    }

    Ok(dashboard_static_dir()?.join("dashboard.html"))
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn json_string_literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn sanitize_static_asset_path(asset_path: &str) -> Option<PathBuf> {
    let mut sanitized = PathBuf::new();
    for component in std::path::Path::new(asset_path).components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    if sanitized.as_os_str().is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

fn dashboard_asset_content_type(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

#[derive(Debug, Deserialize)]
pub struct AggregateQuery {
    pub org: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
}

#[derive(Debug, Clone)]
struct ProjectAggregate {
    project_id: Option<i64>,
    repo_url: String,
    project_name: String,
    branch: Option<String>,
    organization: Option<String>,
    department: Option<String>,
    total_commits: i64,
    total_code: i64,
    total_ai: i64,
}

impl ProjectAggregate {
    fn total_human(&self) -> i64 {
        (self.total_code - self.total_ai).max(0)
    }

    fn pct_ai(&self) -> f64 {
        if self.total_code > 0 {
            (self.total_ai as f64 / self.total_code as f64) * 100.0
        } else {
            0.0
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "project_id": self.project_id,
            "repo_url": self.repo_url,
            "project_name": self.project_name,
            "branch": self.branch,
            "organization": self.organization,
            "department": self.department,
            "total_commits": self.total_commits,
            "total_code": self.total_code,
            "total_ai": self.total_ai,
            "total_human": self.total_human(),
            "pct_ai": self.pct_ai(),
        })
    }
}

fn repo_project_key(repo_url: &str) -> String {
    let normalized = normalize_repo_url(repo_url).unwrap_or_else(|_| repo_url.trim().to_string());
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn normalize_repo_url(url_str: &str) -> Result<String, String> {
    let url_str = url_str.trim();

    if !url_str.contains("://") {
        if let Some((user_host, path)) = url_str.split_once(':') {
            if let Some((_, host)) = user_host.rsplit_once('@') {
                return normalize_ssh_url(host, path);
            }
        }
    }

    let url = Url::parse(url_str).map_err(|e| format!("Invalid URL: {}", e))?;
    let scheme = url.scheme();
    if !["https", "http", "git", "ssh"].contains(&scheme) {
        return Err(format!("Unsupported URL scheme: {}", scheme));
    }

    let host = url.host_str().ok_or("URL must have a host")?;
    let path = url.path().trim_end_matches('/').trim_end_matches(".git");
    let canonical = format!("https://{}{}", host, path);
    validate_normalized_repo_url(&canonical)?;
    Ok(canonical)
}

fn normalize_ssh_url(host: &str, path: &str) -> Result<String, String> {
    if host.is_empty() || path.is_empty() {
        return Err("Invalid SSH URL format".to_string());
    }

    let path = path
        .trim_start_matches('/')
        .trim_end_matches('/')
        .trim_end_matches(".git");
    let canonical = format!("https://{}/{}", host, path);
    validate_normalized_repo_url(&canonical)?;
    Ok(canonical)
}

fn validate_normalized_repo_url(url_str: &str) -> Result<(), String> {
    let url = Url::parse(url_str).map_err(|e| format!("Failed to parse normalized URL: {}", e))?;
    if url.scheme() != "https" {
        return Err("Normalized URL must be HTTPS".to_string());
    }
    if url.host_str().is_none() {
        return Err("Normalized URL must have a valid host".to_string());
    }
    if url.path().is_empty() || url.path() == "/" {
        return Err("Normalized URL must have a path".to_string());
    }
    Ok(())
}

fn parse_epoch_seconds_param(name: &str, value: Option<&str>) -> Result<Option<i64>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }

    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(Some(dt.timestamp()));
    }

    if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .expect("midnight is a valid time")
            .and_utc();
        return Ok(Some(dt.timestamp()));
    }

    Err(AppError::BadRequest(format!(
        "{} must be an RFC3339 timestamp or YYYY-MM-DD date",
        name
    )))
}

fn parse_epoch_filters(
    since: Option<&str>,
    until: Option<&str>,
) -> Result<(Option<i64>, Option<i64>), AppError> {
    Ok((
        parse_epoch_seconds_param("since", since)?,
        parse_epoch_seconds_param("until", until)?,
    ))
}

type SummaryMetricRow = (Option<i64>, Option<i64>, Option<i64>, Option<i64>);
type TrendMetricRow = (chrono::NaiveDate, Option<i64>, Option<i64>, Option<i64>);
type ToolMetricRow = (
    String,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

#[derive(Debug, Default)]
struct ToolAggregate {
    has_report: bool,
    has_metrics: bool,
    ai_additions: i64,
    mixed_additions: i64,
    ai_accepted: i64,
    total_ai_additions: i64,
    total_ai_deletions: i64,
    commits: i64,
}

impl ToolAggregate {
    fn add_report_row(
        &mut self,
        ai_additions: i64,
        mixed_additions: i64,
        ai_accepted: i64,
        total_ai_additions: i64,
        total_ai_deletions: i64,
    ) {
        self.has_report = true;
        self.add_values(
            ai_additions,
            mixed_additions,
            ai_accepted,
            total_ai_additions,
            total_ai_deletions,
        );
    }

    fn add_metrics_row(
        &mut self,
        ai_additions: i64,
        mixed_additions: i64,
        ai_accepted: i64,
        total_ai_additions: i64,
        total_ai_deletions: i64,
    ) {
        self.has_metrics = true;
        self.add_values(
            ai_additions,
            mixed_additions,
            ai_accepted,
            total_ai_additions,
            total_ai_deletions,
        );
    }

    fn add_values(
        &mut self,
        ai_additions: i64,
        mixed_additions: i64,
        ai_accepted: i64,
        total_ai_additions: i64,
        total_ai_deletions: i64,
    ) {
        self.ai_additions += ai_additions;
        self.mixed_additions += mixed_additions;
        self.ai_accepted += ai_accepted;
        self.total_ai_additions += total_ai_additions.max(ai_additions);
        self.total_ai_deletions += total_ai_deletions;
    }

    fn source(&self) -> &'static str {
        match (self.has_report, self.has_metrics) {
            (true, true) => "report+metrics",
            (true, false) => "report",
            (false, true) => "metrics",
            (false, false) => "metrics",
        }
    }
}
type AgentMetricRow = (
    String,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

async fn fetch_metrics_summary_row(
    pool: &sqlx::PgPool,
    use_rollups: bool,
    user_filter: Option<Uuid>,
    org_filter: Option<Uuid>,
    since_ts: Option<i64>,
    until_ts: Option<i64>,
) -> Result<SummaryMetricRow, AppError> {
    if use_rollups {
        return sqlx::query_as(
            r#"SELECT
                COALESCE(SUM(commits), 0)::bigint as total_commits,
                COALESCE(SUM(total_lines), 0)::bigint as total_code_lines,
                COALESCE(SUM(ai_lines), 0)::bigint as total_ai_lines,
                COALESCE(SUM(human_lines), 0)::bigint as total_human_lines
            FROM metrics_daily_rollups
            WHERE tool_model = ''
              AND ($1::uuid IS NULL OR user_id = $1)
              AND ($2::uuid IS NULL OR org_id = $2)
              AND ($3::bigint IS NULL OR day >= (to_timestamp($3) AT TIME ZONE 'UTC')::date)
              AND ($4::bigint IS NULL OR day <= (to_timestamp($4) AT TIME ZONE 'UTC')::date)"#,
        )
        .bind(user_filter)
        .bind(org_filter)
        .bind(since_ts)
        .bind(until_ts)
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Database(e));
    }

    sqlx::query_as(
        r#"SELECT
            COUNT(*) as total_commits,
            COALESCE(SUM(git_diff_added_lines), 0) as total_code_lines,
            COALESCE(SUM(ai_additions), 0) as total_ai_lines,
            COALESCE(SUM(GREATEST(COALESCE(git_diff_added_lines, 0) - COALESCE(ai_additions, 0), 0)), 0) as total_human_lines
        FROM metrics_events WHERE event_type = 1
          AND ($1::uuid IS NULL OR user_id = $1)
          AND ($2::uuid IS NULL OR org_id = $2)
          AND ($3::bigint IS NULL OR timestamp >= $3)
          AND ($4::bigint IS NULL OR timestamp <= $4)"#,
    )
    .bind(user_filter)
    .bind(org_filter)
    .bind(since_ts)
    .bind(until_ts)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::Database(e))
}

async fn fetch_total_developers(
    pool: &sqlx::PgPool,
    use_rollups: bool,
    user_filter: Option<Uuid>,
    org_filter: Option<Uuid>,
    since_ts: Option<i64>,
    until_ts: Option<i64>,
    since_text: &Option<String>,
    until_text: &Option<String>,
) -> Result<i64, AppError> {
    let metrics_source = if use_rollups {
        r#"SELECT user_id
            FROM metrics_daily_rollups
            WHERE tool_model = ''
              AND user_id <> '00000000-0000-0000-0000-000000000000'::uuid
              AND ($1::uuid IS NULL OR user_id = $1)
              AND ($2::uuid IS NULL OR org_id = $2)
              AND ($3::bigint IS NULL OR day >= (to_timestamp($3) AT TIME ZONE 'UTC')::date)
              AND ($4::bigint IS NULL OR day <= (to_timestamp($4) AT TIME ZONE 'UTC')::date)"#
    } else {
        r#"SELECT user_id
            FROM metrics_events
            WHERE event_type = 1
              AND user_id IS NOT NULL
              AND ($1::uuid IS NULL OR user_id = $1)
              AND ($2::uuid IS NULL OR org_id = $2)
              AND ($3::bigint IS NULL OR timestamp >= $3)
              AND ($4::bigint IS NULL OR timestamp <= $4)"#
    };

    sqlx::query_scalar::<_, i64>(&format!(
        r#"SELECT COUNT(DISTINCT user_id)::bigint
        FROM (
            {metrics_source}

            UNION ALL

            SELECT p.user_id
            FROM projects p
            JOIN commit_stats cs ON cs.project_id = p.id
            WHERE p.user_id IS NOT NULL
              AND ($1::uuid IS NULL OR p.user_id = $1)
              AND ($2::uuid IS NULL OR p.org_id = $2)
              AND ($5::timestamptz IS NULL OR cs.author_time_at >= $5::timestamptz)
              AND ($6::timestamptz IS NULL OR cs.author_time_at <= $6::timestamptz)
              AND NOT EXISTS (
                  SELECT 1 FROM metrics_events m
                  WHERE m.event_type = 1
                    AND m.commit_sha = cs.sha
                    AND ($1::uuid IS NULL OR m.user_id = $1)
                    AND ($2::uuid IS NULL OR m.org_id = $2)
              )
        ) combined
        WHERE user_id IS NOT NULL"#
    ))
    .bind(user_filter)
    .bind(org_filter)
    .bind(since_ts)
    .bind(until_ts)
    .bind(since_text)
    .bind(until_text)
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::Database(e))
}

async fn fetch_metrics_project_urls(
    pool: &sqlx::PgPool,
    use_rollups: bool,
    user_filter: Option<Uuid>,
    org_filter: Option<Uuid>,
    since_ts: Option<i64>,
    until_ts: Option<i64>,
) -> Result<Vec<String>, AppError> {
    if use_rollups {
        return sqlx::query_scalar::<_, String>(
            r#"SELECT DISTINCT repo_url
            FROM metrics_daily_rollups
            WHERE tool_model = '' AND repo_url != ''
              AND ($1::uuid IS NULL OR user_id = $1)
              AND ($2::uuid IS NULL OR org_id = $2)
              AND ($3::bigint IS NULL OR day >= (to_timestamp($3) AT TIME ZONE 'UTC')::date)
              AND ($4::bigint IS NULL OR day <= (to_timestamp($4) AT TIME ZONE 'UTC')::date)"#,
        )
        .bind(user_filter)
        .bind(org_filter)
        .bind(since_ts)
        .bind(until_ts)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e));
    }

    sqlx::query_scalar::<_, String>(
        r#"SELECT DISTINCT repo_url
        FROM metrics_events
        WHERE event_type = 1 AND repo_url IS NOT NULL AND repo_url != ''
          AND ($1::uuid IS NULL OR user_id = $1)
          AND ($2::uuid IS NULL OR org_id = $2)
          AND ($3::bigint IS NULL OR timestamp >= $3)
          AND ($4::bigint IS NULL OR timestamp <= $4)"#,
    )
    .bind(user_filter)
    .bind(org_filter)
    .bind(since_ts)
    .bind(until_ts)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Database(e))
}

async fn fetch_trend_rows(
    pool: &sqlx::PgPool,
    use_rollups: bool,
    user_filter: Option<Uuid>,
    org_filter: Option<Uuid>,
    org_slug: &Option<String>,
    since_ts: Option<i64>,
    until_ts: Option<i64>,
    since_text: &Option<String>,
    until_text: &Option<String>,
    date_trunc: &str,
) -> Result<Vec<TrendMetricRow>, AppError> {
    let metrics_source = if use_rollups {
        format!(
            r#"SELECT
                DATE_TRUNC('{0}', day::timestamp)::date AS period,
                COALESCE(SUM(ai_lines), 0)::bigint AS ai_lines,
                COALESCE(SUM(human_lines), 0)::bigint AS human_lines,
                COALESCE(SUM(commits), 0)::bigint AS commits
            FROM metrics_daily_rollups
            WHERE tool_model = ''
              AND ($1::uuid IS NULL OR user_id = $1)
              AND ($2::uuid IS NULL OR org_id = $2)
              AND ($3::text IS NULL OR org_id = (SELECT id FROM organizations WHERE slug = $3))
              AND ($4::bigint IS NULL OR day >= (to_timestamp($4) AT TIME ZONE 'UTC')::date)
              AND ($5::bigint IS NULL OR day <= (to_timestamp($5) AT TIME ZONE 'UTC')::date)
            GROUP BY DATE_TRUNC('{0}', day::timestamp)"#,
            date_trunc
        )
    } else {
        format!(
            r#"SELECT
                DATE_TRUNC('{0}', to_timestamp(timestamp) AT TIME ZONE 'UTC')::date AS period,
                COALESCE(SUM(ai_additions), 0)::bigint AS ai_lines,
                COALESCE(SUM(GREATEST(COALESCE(git_diff_added_lines, 0) - COALESCE(ai_additions, 0), 0)), 0)::bigint AS human_lines,
                COUNT(*)::bigint AS commits
            FROM metrics_events
            WHERE event_type = 1
              AND ($1::uuid IS NULL OR user_id = $1)
              AND ($2::uuid IS NULL OR org_id = $2)
              AND ($3::text IS NULL OR org_id = (SELECT id FROM organizations WHERE slug = $3))
              AND ($4::bigint IS NULL OR timestamp >= $4)
              AND ($5::bigint IS NULL OR timestamp <= $5)
            GROUP BY DATE_TRUNC('{0}', to_timestamp(timestamp) AT TIME ZONE 'UTC')"#,
            date_trunc
        )
    };

    let sql = format!(
        r#"SELECT
            period,
            COALESCE(SUM(ai_lines), 0)::bigint AS ai_lines,
            COALESCE(SUM(human_lines), 0)::bigint AS human_lines,
            COALESCE(SUM(commits), 0)::bigint AS commits
        FROM (
            {metrics_source}

            UNION ALL

            SELECT
                DATE_TRUNC('{0}', cs.author_time_at AT TIME ZONE 'UTC')::date AS period,
                COALESCE(SUM(cs.ai_additions), 0)::bigint AS ai_lines,
                COALESCE(SUM(GREATEST(COALESCE(cs.git_diff_added_lines, 0) - COALESCE(cs.ai_additions, 0), 0)), 0)::bigint AS human_lines,
                COUNT(*)::bigint AS commits
            FROM projects p
            JOIN commit_stats cs ON cs.project_id = p.id
            WHERE cs.author_time_at IS NOT NULL
              AND ($1::uuid IS NULL OR p.user_id = $1)
              AND ($2::uuid IS NULL OR p.org_id = $2)
              AND ($3::text IS NULL OR p.org_id = (SELECT id FROM organizations WHERE slug = $3))
              AND ($6::timestamptz IS NULL OR cs.author_time_at >= $6::timestamptz)
              AND ($7::timestamptz IS NULL OR cs.author_time_at <= $7::timestamptz)
              AND NOT EXISTS (
                  SELECT 1 FROM metrics_events m
                  WHERE m.event_type = 1
                    AND m.commit_sha = cs.sha
                    AND ($1::uuid IS NULL OR m.user_id = $1)
                    AND ($2::uuid IS NULL OR m.org_id = $2)
              )
            GROUP BY DATE_TRUNC('{0}', cs.author_time_at AT TIME ZONE 'UTC')
        ) combined
        GROUP BY period
        ORDER BY period"#,
        date_trunc
    );

    sqlx::query_as(&sql)
        .bind(user_filter)
        .bind(org_filter)
        .bind(org_slug)
        .bind(since_ts)
        .bind(until_ts)
        .bind(since_text)
        .bind(until_text)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e))
}

async fn fetch_metrics_tool_rows(
    pool: &sqlx::PgPool,
    use_rollups: bool,
    user_filter: Option<Uuid>,
    org_filter: Option<Uuid>,
) -> Result<Vec<ToolMetricRow>, AppError> {
    if use_rollups {
        return sqlx::query_as(
            r#"SELECT
                tool_model,
                COALESCE(SUM(ai_lines), 0)::bigint AS ai_additions,
                COALESCE(SUM(mixed_lines), 0)::bigint AS mixed_additions,
                COALESCE(SUM(ai_accepted), 0)::bigint AS ai_accepted,
                COALESCE(SUM(ai_lines), 0)::bigint AS total_ai_additions,
                0::bigint AS total_ai_deletions
            FROM metrics_daily_rollups
            WHERE tool_model != ''
              AND ($1::uuid IS NULL OR user_id = $1)
              AND ($2::uuid IS NULL OR org_id = $2)
            GROUP BY tool_model
            ORDER BY SUM(ai_lines) DESC"#,
        )
        .bind(user_filter)
        .bind(org_filter)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e));
    }

    sqlx::query_as(
        r#"SELECT
            tool_model,
            COALESCE(SUM(ai_additions), 0)::bigint AS ai_additions,
            COALESCE(SUM(mixed_additions), 0)::bigint AS mixed_additions,
            COALESCE(SUM(ai_accepted), 0)::bigint AS ai_accepted,
            COALESCE(SUM(total_ai_additions), 0)::bigint AS total_ai_additions,
            COALESCE(SUM(total_ai_deletions), 0)::bigint AS total_ai_deletions
        FROM metrics_tool_model_events
        WHERE ($1::uuid IS NULL OR user_id = $1)
          AND ($2::uuid IS NULL OR org_id = $2)
        GROUP BY tool_model
        ORDER BY SUM(ai_additions) DESC"#,
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Database(e))
}

async fn fetch_metrics_agent_rows(
    pool: &sqlx::PgPool,
    use_rollups: bool,
    user_filter: Option<Uuid>,
    org_filter: Option<Uuid>,
    org_slug: &Option<String>,
) -> Result<Vec<AgentMetricRow>, AppError> {
    if use_rollups {
        return sqlx::query_as(
            r#"SELECT
                tool_model,
                COALESCE(SUM(ai_lines), 0)::bigint AS ai_additions,
                COALESCE(SUM(mixed_lines), 0)::bigint AS mixed_additions,
                COALESCE(SUM(ai_accepted), 0)::bigint AS ai_accepted,
                COALESCE(SUM(ai_lines), 0)::bigint AS total_ai_additions,
                0::bigint AS total_ai_deletions,
                COALESCE(SUM(commits), 0)::bigint AS commits
            FROM metrics_daily_rollups
            WHERE tool_model != ''
              AND ($1::uuid IS NULL OR user_id = $1)
              AND ($2::uuid IS NULL OR org_id = $2)
              AND ($3::text IS NULL OR org_id = (SELECT id FROM organizations WHERE slug = $3))
            GROUP BY tool_model
            ORDER BY SUM(ai_lines) DESC"#,
        )
        .bind(user_filter)
        .bind(org_filter)
        .bind(org_slug)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Database(e));
    }

    sqlx::query_as(
        r#"SELECT
            tool_model,
            COALESCE(SUM(ai_additions), 0)::bigint AS ai_additions,
            COALESCE(SUM(mixed_additions), 0)::bigint AS mixed_additions,
            COALESCE(SUM(ai_accepted), 0)::bigint AS ai_accepted,
            COALESCE(SUM(total_ai_additions), 0)::bigint AS total_ai_additions,
            COALESCE(SUM(total_ai_deletions), 0)::bigint AS total_ai_deletions,
            COUNT(DISTINCT metric_event_id)::bigint AS commits
        FROM metrics_tool_model_events
        WHERE ($1::uuid IS NULL OR user_id = $1)
          AND ($2::uuid IS NULL OR org_id = $2)
          AND ($3::text IS NULL OR org_id = (SELECT id FROM organizations WHERE slug = $3))
        GROUP BY tool_model
        ORDER BY SUM(ai_additions) DESC"#,
    )
    .bind(user_filter)
    .bind(org_filter)
    .bind(org_slug)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Database(e))
}

/// GET /api/v1/aggregate/summary — Global aggregate summary
pub async fn aggregate_summary(
    State(state): State<AppState>,
    auth: DashboardAuth,
    Query(query): Query<AggregateQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);
    let (since_ts, until_ts) = parse_epoch_filters(query.since.as_deref(), query.until.as_deref())?;

    let row = fetch_metrics_summary_row(
        &state.db,
        state.config.dashboard_use_rollups,
        user_filter,
        org_filter,
        since_ts,
        until_ts,
    )
    .await?;

    let report_row: (Option<i64>, Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(
        r#"SELECT
            COUNT(cs.sha) as total_commits,
            COALESCE(SUM(cs.git_diff_added_lines), 0) as total_code_lines,
            COALESCE(SUM(cs.ai_additions), 0) as total_ai_lines,
            COALESCE(SUM(GREATEST(COALESCE(cs.git_diff_added_lines, 0) - COALESCE(cs.ai_additions, 0), 0)), 0) as total_human_lines
        FROM projects p
        JOIN commit_stats cs ON cs.project_id = p.id
        WHERE ($1::uuid IS NULL OR p.user_id = $1)
          AND ($2::uuid IS NULL OR p.org_id = $2)
          AND ($3::timestamptz IS NULL OR cs.author_time_at >= $3::timestamptz)
          AND ($4::timestamptz IS NULL OR cs.author_time_at <= $4::timestamptz)
          AND NOT EXISTS (
              SELECT 1 FROM metrics_events m
              WHERE m.event_type = 1
                AND m.commit_sha = cs.sha
                AND ($1::uuid IS NULL OR m.user_id = $1)
                AND ($2::uuid IS NULL OR m.org_id = $2)
          )"#,
    )
    .bind(user_filter)
    .bind(org_filter)
    .bind(&query.since)
    .bind(&query.until)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let total_commits = row.0.unwrap_or(0) + report_row.0.unwrap_or(0);
    let total_code = row.1.unwrap_or(0) + report_row.1.unwrap_or(0);
    let total_ai = row.2.unwrap_or(0) + report_row.2.unwrap_or(0);
    let total_human = row.3.unwrap_or(0) + report_row.3.unwrap_or(0);
    let total_developers = fetch_total_developers(
        &state.db,
        state.config.dashboard_use_rollups,
        user_filter,
        org_filter,
        since_ts,
        until_ts,
        &query.since,
        &query.until,
    )
    .await?;
    let metrics_project_urls = fetch_metrics_project_urls(
        &state.db,
        state.config.dashboard_use_rollups,
        user_filter,
        org_filter,
        since_ts,
        until_ts,
    )
    .await?;

    let report_project_hashes: Vec<String> = sqlx::query_scalar::<_, String>(
        r#"SELECT DISTINCT p.remote_url_hash
        FROM projects p
        JOIN commit_stats cs ON cs.project_id = p.id
        WHERE p.remote_url_hash IS NOT NULL AND p.remote_url_hash != ''
          AND ($1::uuid IS NULL OR p.user_id = $1)
          AND ($2::uuid IS NULL OR p.org_id = $2)
          AND ($3::timestamptz IS NULL OR cs.author_time_at >= $3::timestamptz)
          AND ($4::timestamptz IS NULL OR cs.author_time_at <= $4::timestamptz)
          AND NOT EXISTS (
              SELECT 1 FROM metrics_events m
              WHERE m.event_type = 1
                AND m.commit_sha = cs.sha
                AND ($1::uuid IS NULL OR m.user_id = $1)
                AND ($2::uuid IS NULL OR m.org_id = $2)
          )"#,
    )
    .bind(user_filter)
    .bind(org_filter)
    .bind(&query.since)
    .bind(&query.until)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let mut project_keys = std::collections::HashSet::new();
    for repo_url in metrics_project_urls {
        project_keys.insert(repo_project_key(&repo_url));
    }
    for remote_url_hash in report_project_hashes {
        project_keys.insert(remote_url_hash);
    }
    let total_projects = project_keys.len() as i64;
    let pct_ai = if total_code > 0 {
        (total_ai as f64 / total_code as f64) * 100.0
    } else {
        0.0
    };

    Ok(Json(json!({
        "total_commits": total_commits,
        "total_code_lines": total_code,
        "total_ai_lines": total_ai,
        "total_human_lines": total_human,
        "pct_ai_lines": pct_ai,
        "total_developers": total_developers,
        "total_projects": total_projects,
    })))
}

/// GET /api/v1/aggregate/organizations
pub async fn aggregate_organizations(
    State(state): State<AppState>,
    auth: DashboardAuth,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);

    let rows: Vec<(String, String, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"SELECT
            o.name, o.slug,
            COALESCE(stats.commits, 0),
            COALESCE(stats.total_lines, 0),
            COALESCE(stats.ai_lines, 0)
        FROM organizations o
        LEFT JOIN (
            SELECT
                org_id,
                SUM(commits)::bigint AS commits,
                SUM(total_lines)::bigint AS total_lines,
                SUM(ai_lines)::bigint AS ai_lines
            FROM (
                SELECT
                    org_id,
                    COUNT(*)::bigint AS commits,
                    COALESCE(SUM(git_diff_added_lines), 0)::bigint AS total_lines,
                    COALESCE(SUM(ai_additions), 0)::bigint AS ai_lines
                FROM metrics_events
                WHERE event_type = 1
                  AND ($1::uuid IS NULL OR user_id = $1)
                  AND ($2::uuid IS NULL OR org_id = $2)
                GROUP BY org_id

                UNION ALL

                SELECT
                    p.org_id,
                    COUNT(cs.sha)::bigint AS commits,
                    COALESCE(SUM(cs.git_diff_added_lines), 0)::bigint AS total_lines,
                    COALESCE(SUM(cs.ai_additions), 0)::bigint AS ai_lines
                FROM projects p
                JOIN commit_stats cs ON cs.project_id = p.id
                WHERE ($1::uuid IS NULL OR p.user_id = $1)
                  AND ($2::uuid IS NULL OR p.org_id = $2)
                  AND NOT EXISTS (
                      SELECT 1 FROM metrics_events m
                      WHERE m.event_type = 1
                        AND m.commit_sha = cs.sha
                        AND ($1::uuid IS NULL OR m.user_id = $1)
                        AND ($2::uuid IS NULL OR m.org_id = $2)
                  )
                GROUP BY p.org_id
            ) combined
            WHERE org_id IS NOT NULL
            GROUP BY org_id
        ) stats ON stats.org_id = o.id
        WHERE ($2::uuid IS NULL OR o.id = $2)
        ORDER BY o.name"#,
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let result: Vec<Value> = rows
        .iter()
        .map(|(name, slug, commits, total, ai)| {
            let ai = ai.unwrap_or(0);
            let total = total.unwrap_or(0);
            let human = (total - ai).max(0);
            json!({
                "organization": name,
                "org_slug": slug,
                "total_commits": commits.unwrap_or(0),
                "w_total": total,
                "w_ai": ai,
                "w_human": human,
                "pct_ai": if total > 0 { (ai as f64 / total as f64) * 100.0 } else { 0.0 },
            })
        })
        .collect();

    Ok(Json(json!({ "organizations": result })))
}

/// GET /api/v1/aggregate/departments
pub async fn aggregate_departments(
    State(state): State<AppState>,
    auth: DashboardAuth,
    Query(query): Query<AggregateQuery>,
) -> Result<Json<Value>, AppError> {
    let (scope_user_filter, org_filter) = build_data_filters(&auth.0);
    let restrict_department = auth.0.should_filter_department_scope();
    let department_filter = auth.0.department_id;
    let user_filter = if restrict_department {
        None
    } else {
        scope_user_filter
    };

    let rows: Vec<(
        String,
        String,
        String,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    )> = sqlx::query_as(
        r#"SELECT
            d.name, d.slug, o.name as org_name,
            COALESCE(stats.commits, 0),
            COALESCE(stats.total_lines, 0),
            COALESCE(stats.ai_lines, 0)
        FROM departments d
        JOIN organizations o ON d.org_id = o.id
        LEFT JOIN (
            SELECT
                org_id,
                department_id,
                SUM(commits)::bigint AS commits,
                SUM(total_lines)::bigint AS total_lines,
                SUM(ai_lines)::bigint AS ai_lines
            FROM (
                SELECT
                    m.org_id,
                    om.department_id,
                    COUNT(m.id)::bigint AS commits,
                    COALESCE(SUM(m.git_diff_added_lines), 0)::bigint AS total_lines,
                    COALESCE(SUM(m.ai_additions), 0)::bigint AS ai_lines
                FROM org_members om
                JOIN metrics_events m ON m.user_id = om.user_id
                  AND m.org_id = om.org_id
                  AND m.event_type = 1
                WHERE ($1::uuid IS NULL OR m.user_id = $1)
                  AND ($3::uuid IS NULL OR m.org_id = $3)
                  AND ($4::boolean = FALSE OR om.department_id = $5::uuid)
                GROUP BY m.org_id, om.department_id

                UNION ALL

                SELECT
                    p.org_id,
                    om.department_id,
                    COUNT(cs.sha)::bigint AS commits,
                    COALESCE(SUM(cs.git_diff_added_lines), 0)::bigint AS total_lines,
                    COALESCE(SUM(cs.ai_additions), 0)::bigint AS ai_lines
                FROM projects p
                JOIN commit_stats cs ON cs.project_id = p.id
                JOIN org_members om ON om.user_id = p.user_id AND om.org_id = p.org_id
                WHERE ($1::uuid IS NULL OR p.user_id = $1)
                  AND ($3::uuid IS NULL OR p.org_id = $3)
                  AND ($4::boolean = FALSE OR om.department_id = $5::uuid)
                  AND NOT EXISTS (
                      SELECT 1 FROM metrics_events m
                      WHERE m.event_type = 1
                        AND m.commit_sha = cs.sha
                        AND ($1::uuid IS NULL OR m.user_id = $1)
                        AND ($3::uuid IS NULL OR m.org_id = $3)
                  )
                GROUP BY p.org_id, om.department_id
            ) combined
            WHERE org_id IS NOT NULL AND department_id IS NOT NULL
            GROUP BY org_id, department_id
        ) stats ON stats.org_id = d.org_id AND stats.department_id = d.id
        WHERE ($2::text IS NULL OR o.slug = $2)
          AND ($3::uuid IS NULL OR o.id = $3)
          AND ($4::boolean = FALSE OR d.id = $5::uuid)
        ORDER BY o.name, d.name"#,
    )
    .bind(user_filter)
    .bind(&query.org)
    .bind(org_filter)
    .bind(restrict_department)
    .bind(department_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let result: Vec<Value> = rows
        .iter()
        .map(|(name, slug, org_name, commits, total, ai)| {
            let ai = ai.unwrap_or(0);
            let total = total.unwrap_or(0);
            let human = (total - ai).max(0);
            json!({
                "department": name,
                "dept_slug": slug,
                "organization": org_name,
                "total_commits": commits.unwrap_or(0),
                "w_total": total,
                "w_ai": ai,
                "w_human": human,
            })
        })
        .collect();

    Ok(Json(json!({ "departments": result })))
}

/// GET /api/v1/aggregate/projects
pub async fn aggregate_projects(
    State(state): State<AppState>,
    auth: DashboardAuth,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);

    // Aggregate from metrics_events (primary source from client auto-upload)
    let metrics_rows: Vec<(
        String,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    )> = sqlx::query_as(
        r#"WITH committed_events AS (
            SELECT
                repo_url,
                COALESCE(
                    NULLIF(BTRIM(raw_attrs->>'branch'), ''),
                    NULLIF(BTRIM(raw_attrs->>'5'), '')
                ) AS branch,
                git_diff_added_lines,
                ai_additions
            FROM metrics_events
            WHERE event_type = 1 AND repo_url IS NOT NULL AND repo_url != ''
              AND ($1::uuid IS NULL OR user_id = $1)
              AND ($2::uuid IS NULL OR org_id = $2)
        )
        SELECT
            repo_url,
            STRING_AGG(DISTINCT branch, ', ' ORDER BY branch) AS branch,
            COUNT(*) as total_commits,
            COALESCE(SUM(git_diff_added_lines), 0),
            COALESCE(SUM(ai_additions), 0)
        FROM committed_events
        GROUP BY repo_url
        ORDER BY repo_url"#,
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Also aggregate from projects + commit_stats (legacy report upload source)
    let report_rows: Vec<(
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    )> = sqlx::query_as(
        r#"SELECT
            p.id, p.remote_url_hash, p.branch, p.organization, p.department,
            COUNT(cs.sha),
            COALESCE(SUM(cs.git_diff_added_lines), 0),
            COALESCE(SUM(cs.ai_additions), 0)
        FROM projects p
        JOIN commit_stats cs ON cs.project_id = p.id
        WHERE ($1::uuid IS NULL OR p.user_id = $1)
          AND ($2::uuid IS NULL OR p.org_id = $2)
          AND NOT EXISTS (
              SELECT 1 FROM metrics_events m
              WHERE m.event_type = 1
                AND m.commit_sha = cs.sha
                AND ($1::uuid IS NULL OR m.user_id = $1)
                AND ($2::uuid IS NULL OR m.org_id = $2)
          )
        GROUP BY p.id, p.remote_url_hash, p.branch, p.organization, p.department
        ORDER BY p.organization, p.department"#,
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Merge both sources by the same project key used by report uploads: sha256(normalized repo URL).
    let mut projects: std::collections::HashMap<String, ProjectAggregate> =
        std::collections::HashMap::new();

    // First pass: metrics_events data
    for (repo_url, branch, commits, total, ai) in &metrics_rows {
        let ai = ai.unwrap_or(0);
        let total = total.unwrap_or(0);
        let key = repo_project_key(repo_url);
        // Extract a human-readable project name from the repo URL
        let project_name = repo_url
            .trim_end_matches('/')
            .split('/')
            .last()
            .unwrap_or(repo_url)
            .trim_end_matches(".git")
            .to_string();
        projects.insert(
            key,
            ProjectAggregate {
                project_id: None,
                repo_url: repo_url.clone(),
                project_name,
                branch: branch.clone(),
                organization: None,
                department: None,
                total_commits: commits.unwrap_or(0),
                total_code: total,
                total_ai: ai,
            },
        );
    }

    // Second pass: report data, merged into the same project when the hash matches.
    for (id, url_hash, branch, org, dept, commits, total, ai) in &report_rows {
        let ai = ai.unwrap_or(0);
        let total = total.unwrap_or(0);
        let entry = projects.entry(url_hash.clone()).or_insert_with(|| {
            let project_name = url_hash
                .trim_end_matches('/')
                .split('/')
                .last()
                .unwrap_or(url_hash)
                .trim_end_matches(".git")
                .to_string();

            ProjectAggregate {
                project_id: Some(*id),
                repo_url: url_hash.clone(),
                project_name,
                branch: branch.clone(),
                organization: org.clone(),
                department: dept.clone(),
                total_commits: 0,
                total_code: 0,
                total_ai: 0,
            }
        });

        entry.project_id.get_or_insert(*id);
        if entry.branch.is_none() {
            entry.branch = branch.clone();
        }
        if entry.organization.is_none() {
            entry.organization = org.clone();
        }
        if entry.department.is_none() {
            entry.department = dept.clone();
        }
        entry.total_commits += commits.unwrap_or(0);
        entry.total_code += total;
        entry.total_ai += ai;
    }

    let mut result: Vec<Value> = projects.values().map(ProjectAggregate::to_json).collect();
    result.sort_by(|a, b| {
        let a_name = a.get("project_name").and_then(|v| v.as_str()).unwrap_or("");
        let b_name = b.get("project_name").and_then(|v| v.as_str()).unwrap_or("");
        a_name.cmp(b_name)
    });

    Ok(Json(json!({ "projects": result })))
}

/// GET /api/v1/aggregate/developers
pub async fn aggregate_developers(
    State(state): State<AppState>,
    auth: DashboardAuth,
    Query(query): Query<AggregateQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);
    let (since_ts, until_ts) = parse_epoch_filters(query.since.as_deref(), query.until.as_deref())?;

    let rows: Vec<(
        Uuid,
        String,
        String,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Value,
    )> = sqlx::query_as(
        r#"WITH developer_stats AS (
            SELECT
                user_id,
                org_id,
                SUM(commits)::bigint AS total_commits,
                SUM(added)::bigint AS total_added_lines,
                SUM(ai)::bigint AS ai_added_lines,
                SUM(human)::bigint AS human_added_lines
            FROM (
                SELECT
                    user_id,
                    org_id,
                    COUNT(*) AS commits,
                    COALESCE(SUM(git_diff_added_lines), 0) AS added,
                    COALESCE(SUM(ai_additions), 0) AS ai,
                    COALESCE(SUM(GREATEST(COALESCE(git_diff_added_lines, 0) - COALESCE(ai_additions, 0), 0)), 0) AS human
                FROM metrics_events
                WHERE event_type = 1
                  AND user_id IS NOT NULL
                  AND ($1::uuid IS NULL OR user_id = $1)
                  AND ($2::uuid IS NULL OR org_id = $2)
                  AND ($3::bigint IS NULL OR timestamp >= $3)
                  AND ($4::bigint IS NULL OR timestamp <= $4)
                GROUP BY user_id, org_id

                UNION ALL

                SELECT
                    p.user_id,
                    p.org_id,
                    COUNT(*) AS commits,
                    COALESCE(SUM(cs.git_diff_added_lines), 0) AS added,
                    COALESCE(SUM(cs.ai_additions), 0) AS ai,
                    COALESCE(SUM(GREATEST(COALESCE(cs.git_diff_added_lines, 0) - COALESCE(cs.ai_additions, 0), 0)), 0) AS human
                FROM projects p
                JOIN commit_stats cs ON cs.project_id = p.id
                WHERE p.user_id IS NOT NULL
                  AND ($1::uuid IS NULL OR p.user_id = $1)
                  AND ($2::uuid IS NULL OR p.org_id = $2)
                  AND ($5::timestamptz IS NULL OR cs.author_time_at >= $5::timestamptz)
                  AND ($6::timestamptz IS NULL OR cs.author_time_at <= $6::timestamptz)
                  AND NOT EXISTS (
                      SELECT 1 FROM metrics_events m
                      WHERE m.event_type = 1
                        AND m.commit_sha = cs.sha
                        AND ($1::uuid IS NULL OR m.user_id = $1)
                        AND ($2::uuid IS NULL OR m.org_id = $2)
                  )
                GROUP BY p.user_id, p.org_id
            ) combined
            GROUP BY user_id, org_id
        ),
        git_identities AS (
            SELECT
                user_id,
                org_id,
                jsonb_agg(DISTINCT jsonb_build_object('name', git_name, 'email', git_email))
                    FILTER (WHERE git_name != '' OR git_email != '') AS identities
            FROM (
                SELECT
                    user_id,
                    org_id,
                    TRIM(CASE
                        WHEN author_email ~ '<[^>]+>' THEN split_part(author_email, '<', 1)
                        WHEN author_email LIKE '%@%' THEN ''
                        ELSE author_email
                    END) AS git_name,
                    TRIM(CASE
                        WHEN author_email ~ '<[^>]+>' THEN substring(author_email from '<([^>]+)>')
                        WHEN author_email LIKE '%@%' THEN author_email
                        ELSE ''
                    END) AS git_email
                FROM metrics_events
                WHERE event_type = 1
                  AND user_id IS NOT NULL
                  AND author_email IS NOT NULL
                  AND author_email != ''
                  AND ($1::uuid IS NULL OR user_id = $1)
                  AND ($2::uuid IS NULL OR org_id = $2)
                  AND ($3::bigint IS NULL OR timestamp >= $3)
                  AND ($4::bigint IS NULL OR timestamp <= $4)

                UNION

                SELECT
                    p.user_id,
                    p.org_id,
                    TRIM(CASE
                        WHEN cs.author ~ '<[^>]+>' THEN split_part(cs.author, '<', 1)
                        WHEN cs.author LIKE '%@%' THEN ''
                        ELSE cs.author
                    END) AS git_name,
                    TRIM(CASE
                        WHEN cs.author ~ '<[^>]+>' THEN substring(cs.author from '<([^>]+)>')
                        WHEN cs.author LIKE '%@%' THEN cs.author
                        ELSE ''
                    END) AS git_email
                FROM projects p
                JOIN commit_stats cs ON cs.project_id = p.id
                WHERE p.user_id IS NOT NULL
                  AND cs.author IS NOT NULL
                  AND cs.author != ''
                  AND ($1::uuid IS NULL OR p.user_id = $1)
                  AND ($2::uuid IS NULL OR p.org_id = $2)
                  AND ($5::timestamptz IS NULL OR cs.author_time_at >= $5::timestamptz)
                  AND ($6::timestamptz IS NULL OR cs.author_time_at <= $6::timestamptz)
                  AND NOT EXISTS (
                      SELECT 1 FROM metrics_events m
                      WHERE m.event_type = 1
                        AND m.commit_sha = cs.sha
                        AND ($1::uuid IS NULL OR m.user_id = $1)
                        AND ($2::uuid IS NULL OR m.org_id = $2)
                  )
            ) identities
            GROUP BY user_id, org_id
        )
        SELECT
            u.id,
            u.name,
            u.email,
            d.name AS department_name,
            ds.total_commits,
            ds.total_added_lines,
            ds.ai_added_lines,
            ds.human_added_lines,
            COALESCE(gi.identities, '[]'::jsonb) AS git_identities
        FROM developer_stats ds
        JOIN users u ON u.id = ds.user_id
        LEFT JOIN org_members om ON om.user_id = u.id
          AND om.org_id = COALESCE(ds.org_id, u.default_org_id)
        LEFT JOIN departments d ON d.id = om.department_id AND d.org_id = om.org_id
        LEFT JOIN git_identities gi ON gi.user_id = ds.user_id
          AND gi.org_id IS NOT DISTINCT FROM ds.org_id
        ORDER BY ds.ai_added_lines DESC, ds.total_commits DESC, u.name ASC"#
    )
    .bind(user_filter)
    .bind(org_filter)
    .bind(since_ts)
    .bind(until_ts)
    .bind(&query.since)
    .bind(&query.until)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let result: Vec<Value> = rows
        .iter()
        .map(
            |(user_id, name, email, department, commits, added, ai, human, git_identities)| {
                let ai = ai.unwrap_or(0);
                let total = added.unwrap_or(0);
                json!({
                    "id": user_id.to_string(),
                    "email": email,
                    "name": name,
                    "department": department,
                    "total_commits": commits.unwrap_or(0),
                    "total_added_lines": total,
                    "ai_added_lines": ai,
                    "human_added_lines": human.unwrap_or(0),
                    "pct_ai": if total > 0 { (ai as f64 / total as f64) * 100.0 } else { 0.0 },
                    "git_identities": git_identities,
                })
            },
        )
        .collect();

    Ok(Json(json!({ "developers": result })))
}

/// GET /api/v1/aggregate/tools — Tool/Model breakdown statistics
pub async fn aggregate_tools(
    State(state): State<AppState>,
    auth: DashboardAuth,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);

    // First try tool_model_stats from report uploads (richer data)
    let report_rows: Vec<(
        String,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    )> = sqlx::query_as(
        r#"SELECT
            tms.tool_model,
            COALESCE(SUM(tms.ai_additions), 0),
            COALESCE(SUM(tms.mixed_additions), 0),
            COALESCE(SUM(tms.ai_accepted), 0),
            COALESCE(SUM(tms.total_ai_additions), 0),
            COALESCE(SUM(tms.total_ai_deletions), 0)
        FROM tool_model_stats tms
        JOIN projects p ON tms.project_id = p.id
        WHERE ($1::uuid IS NULL OR p.user_id = $1)
          AND ($2::uuid IS NULL OR p.org_id = $2)
        GROUP BY tms.tool_model
        ORDER BY SUM(tms.ai_additions) DESC"#,
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // From committed metrics events or daily rollups.
    let metrics_rows = fetch_metrics_tool_rows(
        &state.db,
        state.config.dashboard_use_rollups,
        user_filter,
        org_filter,
    )
    .await?;

    // Also get Checkpoint events (type 4) which have tool/model directly
    let checkpoint_rows: Vec<(Option<String>, Option<String>, Option<i64>)> = sqlx::query_as(
        r#"SELECT
            tool,
            model,
            COALESCE(SUM(ai_additions), 0)
        FROM metrics_events
        WHERE event_type IN (2, 4) AND tool IS NOT NULL AND tool != ''
          AND ($1::uuid IS NULL OR user_id = $1)
          AND ($2::uuid IS NULL OR org_id = $2)
        GROUP BY tool, model
        ORDER BY SUM(ai_additions) DESC"#,
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let mut tools_by_model: std::collections::BTreeMap<String, ToolAggregate> =
        std::collections::BTreeMap::new();

    // Add report-based tool stats
    for (tool_model, ai_add, mixed_add, ai_accept, total_ai_add, total_ai_del) in &report_rows {
        tools_by_model
            .entry(tool_model.clone())
            .or_default()
            .add_report_row(
                ai_add.unwrap_or(0),
                mixed_add.unwrap_or(0),
                ai_accept.unwrap_or(0),
                total_ai_add.unwrap_or(0),
                total_ai_del.unwrap_or(0),
            );
    }

    // Add metrics-based committed-event stats.
    for (tool_model, ai_add, mixed_add, ai_accept, total_ai_add, total_ai_del) in &metrics_rows {
        tools_by_model
            .entry(tool_model.clone())
            .or_default()
            .add_metrics_row(
                ai_add.unwrap_or(0),
                mixed_add.unwrap_or(0),
                ai_accept.unwrap_or(0),
                total_ai_add.unwrap_or(0),
                total_ai_del.unwrap_or(0),
            );
    }

    // Also add Checkpoint/AgentUsage events which have tool/model directly.
    for (tool, model, ai_add) in &checkpoint_rows {
        let tool_name = tool.as_deref().unwrap_or("unknown");
        let model_name = model.as_deref().unwrap_or("");
        let tool_model = if model_name.is_empty() {
            tool_name.to_string()
        } else {
            format!("{}::{}", tool_name, model_name)
        };

        let ai_additions = ai_add.unwrap_or(0);
        tools_by_model
            .entry(tool_model)
            .or_default()
            .add_metrics_row(ai_additions, 0, 0, ai_additions, 0);
    }

    let mut tools: Vec<Value> = tools_by_model
        .into_iter()
        .map(|(tool_model, stats)| {
            json!({
                "tool_model": tool_model,
                "source": stats.source(),
                "ai_additions": stats.ai_additions,
                "mixed_additions": stats.mixed_additions,
                "ai_accepted": stats.ai_accepted,
                "total_ai_additions": stats.total_ai_additions,
                "total_ai_deletions": stats.total_ai_deletions,
            })
        })
        .collect();

    // Sort by ai_additions descending
    tools.sort_by(|a, b| {
        let a_val = a.get("ai_additions").and_then(|v| v.as_i64()).unwrap_or(0);
        let b_val = b.get("ai_additions").and_then(|v| v.as_i64()).unwrap_or(0);
        b_val.cmp(&a_val)
    });

    Ok(Json(json!({ "tools": tools })))
}

// ================================================================
// Phase 6: Advanced Dashboard Enhancement APIs
// ================================================================

#[derive(Debug, Deserialize)]
pub struct TrendsQuery {
    pub metric: Option<String>, // "ai_ratio", "ai_lines", "human_lines", "commits"
    pub granularity: Option<String>, // "day", "week", "month"
    pub org: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
}

/// GET /api/v1/aggregate/trends — AI code attribution trends over time
pub async fn aggregate_trends(
    State(state): State<AppState>,
    auth: DashboardAuth,
    Query(query): Query<TrendsQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);
    let (since_ts, until_ts) = parse_epoch_filters(query.since.as_deref(), query.until.as_deref())?;

    let metric = query.metric.as_deref().unwrap_or("ai_ratio");
    let granularity = query.granularity.as_deref().unwrap_or("week");

    let valid_metrics = ["ai_ratio", "ai_lines", "human_lines", "commits"];
    if !valid_metrics.contains(&metric) {
        return Err(AppError::BadRequest(format!(
            "metric must be one of: {}",
            valid_metrics.join(", ")
        )));
    }

    let valid_granularities = ["day", "week", "month"];
    if !valid_granularities.contains(&granularity) {
        return Err(AppError::BadRequest(format!(
            "granularity must be one of: {}",
            valid_granularities.join(", ")
        )));
    }

    let date_trunc = match granularity {
        "day" => "day",
        "week" => "week",
        "month" => "month",
        _ => "week",
    };

    let rows = fetch_trend_rows(
        &state.db,
        state.config.dashboard_use_rollups,
        user_filter,
        org_filter,
        &query.org,
        since_ts,
        until_ts,
        &query.since,
        &query.until,
        date_trunc,
    )
    .await?;

    let data: Vec<Value> = rows
        .iter()
        .map(|(period, ai, human, commits)| {
            let ai = ai.unwrap_or(0);
            let human = human.unwrap_or(0);
            let total = ai + human;
            let ai_ratio = if total > 0 {
                (ai as f64 / total as f64) * 100.0
            } else {
                0.0
            };

            let value = match metric {
                "ai_ratio" => ai_ratio,
                "ai_lines" => ai as f64,
                "human_lines" => human as f64,
                "commits" => commits.unwrap_or(0) as f64,
                _ => 0.0,
            };

            json!({
                "period": period.to_string(),
                "granularity": granularity,
                "value": (value * 100.0).round() / 100.0,
                "ai_lines": ai,
                "human_lines": human,
                "commits": commits.unwrap_or(0),
                "ai_ratio": (ai_ratio * 100.0).round() / 100.0,
            })
        })
        .collect();

    Ok(Json(json!({
        "metric": metric,
        "granularity": granularity,
        "data": data,
    })))
}

#[derive(Debug, Deserialize)]
pub struct AgentComparisonQuery {
    pub org: Option<String>,
}

/// GET /api/v1/aggregate/agent-comparison — Compare AI tools/models
pub async fn aggregate_agent_comparison(
    State(state): State<AppState>,
    auth: DashboardAuth,
    Query(query): Query<AgentComparisonQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);
    // From report data
    let report_rows: Vec<(
        String,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    )> = sqlx::query_as(
        r#"SELECT
            tms.tool_model,
            COALESCE(SUM(tms.ai_additions), 0),
            COALESCE(SUM(tms.mixed_additions), 0),
            COALESCE(SUM(tms.ai_accepted), 0),
            COALESCE(SUM(tms.total_ai_additions), 0),
            COALESCE(SUM(tms.total_ai_deletions), 0)
        FROM tool_model_stats tms
        JOIN projects p ON tms.project_id = p.id
        WHERE ($1::uuid IS NULL OR p.user_id = $1)
          AND ($2::uuid IS NULL OR p.org_id = $2)
        GROUP BY tms.tool_model
        ORDER BY SUM(tms.ai_additions) DESC"#,
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // From committed metrics events or daily rollups.
    let metrics_rows = fetch_metrics_agent_rows(
        &state.db,
        state.config.dashboard_use_rollups,
        user_filter,
        org_filter,
        &query.org,
    )
    .await?;

    let mut comparisons_by_model: std::collections::BTreeMap<String, ToolAggregate> =
        std::collections::BTreeMap::new();

    // Report-based
    for (tool_model, ai_add, mixed_add, ai_accept, total_ai_add, total_ai_del) in &report_rows {
        comparisons_by_model
            .entry(tool_model.clone())
            .or_default()
            .add_report_row(
                ai_add.unwrap_or(0),
                mixed_add.unwrap_or(0),
                ai_accept.unwrap_or(0),
                total_ai_add.unwrap_or(0),
                total_ai_del.unwrap_or(0),
            );
    }

    // Metrics-based (supplementary)
    for (tool_model, ai_add, mixed_add, ai_accept, total_ai_add, total_ai_del, commits) in
        &metrics_rows
    {
        let aggregate = comparisons_by_model.entry(tool_model.clone()).or_default();
        aggregate.add_metrics_row(
            ai_add.unwrap_or(0),
            mixed_add.unwrap_or(0),
            ai_accept.unwrap_or(0),
            total_ai_add.unwrap_or(0),
            total_ai_del.unwrap_or(0),
        );
        aggregate.commits += commits.unwrap_or(0);
    }

    let mut comparisons: Vec<Value> = comparisons_by_model
        .into_iter()
        .map(|(tool_model, stats)| {
            let acceptance_rate = if stats.ai_additions > 0 {
                (stats.ai_accepted as f64 / stats.ai_additions as f64) * 100.0
            } else {
                0.0
            };
            json!({
                "tool_model": tool_model,
                "source": stats.source(),
                "ai_additions": stats.ai_additions,
                "mixed_additions": stats.mixed_additions,
                "ai_accepted": stats.ai_accepted,
                "total_ai_additions": stats.total_ai_additions,
                "total_ai_deletions": stats.total_ai_deletions,
                "net_ai_lines": stats.total_ai_additions - stats.total_ai_deletions,
                "commits": stats.commits,
                "acceptance_rate": (acceptance_rate * 100.0).round() / 100.0,
            })
        })
        .collect();

    // Sort by ai_additions descending
    comparisons.sort_by(|a, b| {
        let a_val = a.get("ai_additions").and_then(|v| v.as_i64()).unwrap_or(0);
        let b_val = b.get("ai_additions").and_then(|v| v.as_i64()).unwrap_or(0);
        b_val.cmp(&a_val)
    });

    Ok(Json(json!({ "comparisons": comparisons })))
}

#[derive(Debug, Deserialize)]
pub struct TeamComparisonQuery {
    pub org: Option<String>,
}

/// GET /api/v1/aggregate/team-comparison — Compare AI adoption across teams/departments
pub async fn aggregate_team_comparison(
    State(state): State<AppState>,
    auth: DashboardAuth,
    Query(query): Query<TeamComparisonQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);

    let rows: Vec<(String, String, String, Option<i64>, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"SELECT
            d.name AS dept_name,
            d.slug AS dept_slug,
            o.name AS org_name,
            COUNT(m.id) AS total_commits,
            COALESCE(SUM(m.git_diff_added_lines), 0) AS total_lines,
            COALESCE(SUM(m.ai_additions), 0) AS ai_lines,
            COALESCE(SUM(GREATEST(COALESCE(m.git_diff_added_lines, 0) - COALESCE(m.ai_additions, 0), 0)), 0) AS human_lines
        FROM departments d
        JOIN organizations o ON d.org_id = o.id
        LEFT JOIN org_members om ON om.department_id = d.id AND om.org_id = d.org_id
        LEFT JOIN metrics_events m ON m.user_id = om.user_id AND m.org_id = om.org_id AND m.event_type = 1
          AND ($1::uuid IS NULL OR m.user_id = $1)
        WHERE ($2::text IS NULL OR o.slug = $2)
          AND ($3::uuid IS NULL OR o.id = $3)
        GROUP BY d.id, d.name, d.slug, o.name
        ORDER BY o.name, d.name"#
    )
    .bind(user_filter)
    .bind(&query.org)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let teams: Vec<Value> = rows.iter().map(|(dept_name, dept_slug, org_name, commits, total, ai, human)| {
        let ai = ai.unwrap_or(0);
        let human = human.unwrap_or(0);
        let total = total.unwrap_or(0);
        let pct_ai = if total > 0 { (ai as f64 / total as f64) * 100.0 } else { 0.0 };

        json!({
            "department": dept_name,
            "dept_slug": dept_slug,
            "organization": org_name,
            "total_commits": commits.unwrap_or(0),
            "total_lines": total,
            "ai_lines": ai,
            "human_lines": human,
            "pct_ai": (pct_ai * 100.0).round() / 100.0,
            "adoption_level": if pct_ai >= 60.0 { "high" } else if pct_ai >= 30.0 { "medium" } else { "low" },
        })
    }).collect();

    Ok(Json(json!({ "teams": teams })))
}

/// Build data filter parameters based on the user's role.
/// Returns (user_id_filter, org_id_filter):
/// - Admin users: (None, Some(org_id)) — sees all data within their organization
/// - Non-admin users: (Some(user_id), Some(org_id)) — sees only their own data within their organization
/// - If org_id is not available, falls back to no org filter (should not happen in practice)
pub fn build_data_filters(
    auth: &crate::models::user::AuthIdentity,
) -> (Option<uuid::Uuid>, Option<uuid::Uuid>) {
    if auth.is_admin() {
        // Admin sees all data within their organization (no user filter, but org filter applies)
        (None, auth.org_id)
    } else {
        // Non-admin sees only their own data within their organization
        (Some(auth.user_id), auth.org_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use sqlx::PgPool;
    use uuid::Uuid;

    #[test]
    fn parse_epoch_seconds_param_accepts_rfc3339() {
        let parsed = parse_epoch_seconds_param("since", Some("2024-01-01T00:00:00Z")).unwrap();

        assert_eq!(parsed, Some(1_704_067_200));
    }

    #[test]
    fn parse_epoch_seconds_param_accepts_date() {
        let parsed = parse_epoch_seconds_param("until", Some("2024-01-02")).unwrap();

        assert_eq!(parsed, Some(1_704_153_600));
    }

    #[test]
    fn parse_epoch_seconds_param_ignores_empty_values() {
        assert_eq!(parse_epoch_seconds_param("since", None).unwrap(), None);
        assert_eq!(
            parse_epoch_seconds_param("since", Some("  ")).unwrap(),
            None
        );
    }

    #[test]
    fn parse_epoch_seconds_param_rejects_invalid_values() {
        assert!(matches!(
            parse_epoch_seconds_param("since", Some("not-a-date")),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn tool_aggregate_merges_sources_and_fills_missing_totals() {
        let mut aggregate = ToolAggregate::default();

        aggregate.add_report_row(10, 2, 8, 12, 1);
        aggregate.add_metrics_row(5, 1, 4, 0, 0);

        assert_eq!(aggregate.source(), "report+metrics");
        assert_eq!(aggregate.ai_additions, 15);
        assert_eq!(aggregate.mixed_additions, 3);
        assert_eq!(aggregate.ai_accepted, 12);
        assert_eq!(aggregate.total_ai_additions, 17);
        assert_eq!(aggregate.total_ai_deletions, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn summary_rollup_matches_metrics_detail_path() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.pool).await?;
        insert_dashboard_metrics_fixture(&db.pool, user_id, org_id).await?;

        let detail =
            fetch_metrics_summary_row(&db.pool, false, Some(user_id), Some(org_id), None, None)
                .await?;
        let rollup =
            fetch_metrics_summary_row(&db.pool, true, Some(user_id), Some(org_id), None, None)
                .await?;
        assert_eq!(detail, (Some(2), Some(70), Some(30), Some(40)));
        assert_eq!(rollup, detail);

        let detail_developers = fetch_total_developers(
            &db.pool,
            false,
            Some(user_id),
            Some(org_id),
            None,
            None,
            &None,
            &None,
        )
        .await?;
        let rollup_developers = fetch_total_developers(
            &db.pool,
            true,
            Some(user_id),
            Some(org_id),
            None,
            None,
            &None,
            &None,
        )
        .await?;
        assert_eq!(detail_developers, 1);
        assert_eq!(rollup_developers, detail_developers);

        let mut detail_projects =
            fetch_metrics_project_urls(&db.pool, false, Some(user_id), Some(org_id), None, None)
                .await?;
        let mut rollup_projects =
            fetch_metrics_project_urls(&db.pool, true, Some(user_id), Some(org_id), None, None)
                .await?;
        detail_projects.sort();
        rollup_projects.sort();
        assert_eq!(rollup_projects, detail_projects);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn trend_and_tool_rollups_match_metrics_detail_path() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.pool).await?;
        insert_dashboard_metrics_fixture(&db.pool, user_id, org_id).await?;

        let detail_trends = fetch_trend_rows(
            &db.pool,
            false,
            Some(user_id),
            Some(org_id),
            &None,
            None,
            None,
            &None,
            &None,
            "day",
        )
        .await?;
        let rollup_trends = fetch_trend_rows(
            &db.pool,
            true,
            Some(user_id),
            Some(org_id),
            &None,
            None,
            None,
            &None,
            &None,
            "day",
        )
        .await?;
        assert_eq!(rollup_trends, detail_trends);

        let detail_tools =
            fetch_metrics_tool_rows(&db.pool, false, Some(user_id), Some(org_id)).await?;
        let rollup_tools =
            fetch_metrics_tool_rows(&db.pool, true, Some(user_id), Some(org_id)).await?;
        assert_eq!(detail_tools, rollup_tools);
        assert_eq!(rollup_tools[0].0, "codex::gpt-5");
        assert_eq!(rollup_tools[0].1, Some(9));
        assert_eq!(rollup_tools[0].2, Some(3));
        assert_eq!(rollup_tools[0].3, Some(5));

        let detail_agents =
            fetch_metrics_agent_rows(&db.pool, false, Some(user_id), Some(org_id), &None).await?;
        let rollup_agents =
            fetch_metrics_agent_rows(&db.pool, true, Some(user_id), Some(org_id), &None).await?;
        assert_eq!(rollup_agents, detail_agents);
        assert_eq!(rollup_agents[0].6, Some(2));

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
                        "skipping dashboard database test: could not connect to admin database: {error}"
                    );
                    return Ok(None);
                }
            };

            if let Err(error) = create_database(&admin_pool, &db_name).await {
                eprintln!(
                    "skipping dashboard database test: could not create isolated database {db_name}: {error}"
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
            .bind("Dashboard Test Org")
            .bind(format!("dashboard-test-{}", org_id.simple()))
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO users (id, email, name, default_org_id) VALUES ($1, $2, $3, $4)")
            .bind(user_id)
            .bind(format!("{user_id}@example.com"))
            .bind("Dashboard Test User")
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

    async fn insert_dashboard_metrics_fixture(
        pool: &PgPool,
        user_id: Uuid,
        org_id: Uuid,
    ) -> anyhow::Result<()> {
        insert_dashboard_metric_row(
            pool,
            user_id,
            org_id,
            1_700_000_000,
            "repo-a",
            "abc1",
            30,
            20,
            5,
            2,
            3,
        )
        .await?;
        insert_dashboard_metric_row(
            pool,
            user_id,
            org_id,
            1_700_086_400,
            "repo-b",
            "abc2",
            40,
            10,
            4,
            1,
            2,
        )
        .await?;
        Ok(())
    }

    async fn insert_dashboard_metric_row(
        pool: &PgPool,
        user_id: Uuid,
        org_id: Uuid,
        timestamp: i64,
        repo_url: &str,
        commit_sha: &str,
        total_lines: i32,
        total_ai_lines: i32,
        tool_ai_lines: i32,
        tool_mixed_lines: i32,
        tool_accepted: i32,
    ) -> anyhow::Result<()> {
        let raw_values = serde_json::json!({
            "3": ["all", "codex::gpt-5"],
            "4": [tool_mixed_lines, tool_mixed_lines],
            "5": [total_ai_lines, tool_ai_lines],
            "6": [tool_accepted, tool_accepted],
            "7": [total_ai_lines, tool_ai_lines],
            "8": [0, 0],
        });
        let tool_model_pairs = serde_json::json!(["all", "codex::gpt-5"]);

        let metric_event_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO metrics_events (
                event_type, timestamp, user_id, org_id, repo_url, commit_sha,
                human_additions, ai_additions, mixed_additions, ai_accepted,
                git_diff_added_lines, tool_model_pairs, raw_values
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            RETURNING id"#,
        )
        .bind(1_i16)
        .bind(timestamp)
        .bind(user_id)
        .bind(org_id)
        .bind(repo_url)
        .bind(commit_sha)
        .bind(total_lines - total_ai_lines)
        .bind(total_ai_lines)
        .bind(tool_mixed_lines)
        .bind(tool_accepted)
        .bind(total_lines)
        .bind(&tool_model_pairs)
        .bind(&raw_values)
        .fetch_one(pool)
        .await?;

        sqlx::query(
            r#"INSERT INTO metrics_tool_model_events (
                metric_event_id, org_id, user_id, timestamp, tool_model,
                ai_additions, mixed_additions, ai_accepted,
                total_ai_additions, total_ai_deletions
            ) VALUES ($1, $2, $3, $4, 'codex::gpt-5', $5, $6, $7, $8, 0)"#,
        )
        .bind(metric_event_id)
        .bind(org_id)
        .bind(user_id)
        .bind(timestamp)
        .bind(i64::from(tool_ai_lines))
        .bind(i64::from(tool_mixed_lines))
        .bind(i64::from(tool_accepted))
        .bind(i64::from(tool_ai_lines))
        .execute(pool)
        .await?;

        sqlx::query(
            r#"INSERT INTO metrics_daily_rollups (
                day, org_id, user_id, repo_url, tool_model,
                commits, total_lines, ai_lines, human_lines, mixed_lines, ai_accepted
            ) VALUES
            ((to_timestamp($1) AT TIME ZONE 'UTC')::date, $2, $3, $4, '', 1, $5, $6, $7, $8, $9),
            ((to_timestamp($1) AT TIME ZONE 'UTC')::date, $2, $3, $4, 'codex::gpt-5', 1, 0, $10, 0, $11, $12)"#,
        )
        .bind(timestamp)
        .bind(org_id)
        .bind(user_id)
        .bind(repo_url)
        .bind(i64::from(total_lines))
        .bind(i64::from(total_ai_lines))
        .bind(i64::from(total_lines - total_ai_lines))
        .bind(i64::from(tool_mixed_lines))
        .bind(i64::from(tool_accepted))
        .bind(i64::from(tool_ai_lines))
        .bind(i64::from(tool_mixed_lines))
        .bind(i64::from(tool_accepted))
        .execute(pool)
        .await?;

        Ok(())
    }

    fn test_database_url() -> String {
        dotenvy::dotenv().ok();
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://gitai:gitai@localhost:5433/gitai_enterprise".into())
    }

    fn unique_test_database_name() -> String {
        format!("git_ai_dashboard_test_{}", Uuid::new_v4().simple())
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
