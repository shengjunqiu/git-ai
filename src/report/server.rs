use crate::authorship::stats::ToolModelHeadlineStats;
use crate::error::GitAiError;
use crate::report::model::{
    DEVELOPER_SUMMARY_SCHEMA_VERSION, REPORT_SCHEMA_VERSION, DeveloperSummary, ProjectRatios,
    ProjectSummaryReport, ReportDocument, ReportSummary,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;

// ---------------------------------------------------------------------------
// Aggregate response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct GlobalAggregateSummary {
    pub total_reports: usize,
    pub total_projects: usize,
    pub total_developers: usize,
    pub total_organizations: usize,
    pub total_departments: usize,
    pub weighted_ai_ratio: f64,
    pub weighted_human_ratio: f64,
}

#[derive(Debug, Serialize)]
pub struct OrgAggregateSummary {
    pub organization: String,
    pub project_count: usize,
    pub developer_count: usize,
    pub total_commits: i64,
    pub weighted_ai_ratio: f64,
    pub weighted_human_ratio: f64,
}

#[derive(Debug, Serialize)]
pub struct DeptAggregateSummary {
    pub organization: String,
    pub department: String,
    pub project_count: usize,
    pub developer_count: usize,
    pub total_commits: i64,
    pub weighted_ai_ratio: f64,
    pub weighted_human_ratio: f64,
}

#[derive(Debug, Serialize)]
pub struct ProjectAggregateSummary {
    pub project_name: String,
    pub git_url: Option<String>,
    pub branch: Option<String>,
    pub organization: Option<String>,
    pub department: Option<String>,
    pub report_count: usize,
    pub total_commits: i64,
    pub weighted_ai_ratio: f64,
    pub weighted_human_ratio: f64,
}

#[derive(Debug, Serialize)]
pub struct DeveloperAggregateSummary {
    pub name: String,
    pub email: String,
    pub organization: Option<String>,
    pub department: Option<String>,
    pub project_count: usize,
    pub total_commits: i64,
    pub total_added_lines: i64,
    pub total_ai_additions: i64,
    pub total_human_additions: i64,
    pub weighted_ai_ratio: f64,
    pub weighted_human_ratio: f64,
}

#[derive(Debug, Serialize)]
pub struct IngestResponse {
    pub project_id: i64,
    pub upload_id: i64,
    pub inserted_commits: usize,
    pub duplicate_commits: usize,
}

#[derive(Debug, Serialize)]
pub struct ProjectSummaryResponse {
    pub project_id: i64,
    pub commit_count: usize,
    pub summary: ReportSummary,
}

#[derive(Debug, Serialize)]
pub struct SummaryIngestResponse {
    pub summary_id: i64,
    pub project_name: String,
    pub developer_count: usize,
    pub organization: Option<String>,
    pub department: Option<String>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

pub struct ReportStore {
    conn: Connection,
}

impl ReportStore {
    pub fn open(path: &Path) -> Result<Self, GitAiError> {
        let conn = Connection::open(path)?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self, GitAiError> {
        let conn = Connection::open_in_memory()?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), GitAiError> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS projects (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                remote_url_hash TEXT NOT NULL UNIQUE,
                branch TEXT,
                head_commit TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS report_uploads (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL,
                schema_version TEXT NOT NULL,
                generated_at TEXT NOT NULL,
                commit_count INTEGER NOT NULL,
                uploaded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(project_id) REFERENCES projects(id)
            );

            CREATE TABLE IF NOT EXISTS commit_stats (
                project_id INTEGER NOT NULL,
                sha TEXT NOT NULL,
                author TEXT NOT NULL,
                author_time TEXT NOT NULL,
                subject TEXT NOT NULL,
                has_authorship_note INTEGER NOT NULL,
                git_diff_added_lines INTEGER NOT NULL,
                git_diff_deleted_lines INTEGER NOT NULL,
                ai_additions INTEGER NOT NULL,
                human_additions INTEGER NOT NULL,
                mixed_additions INTEGER NOT NULL,
                unknown_additions INTEGER NOT NULL,
                ai_accepted INTEGER NOT NULL,
                total_ai_additions INTEGER NOT NULL,
                total_ai_deletions INTEGER NOT NULL,
                time_waiting_for_ai INTEGER NOT NULL,
                PRIMARY KEY(project_id, sha),
                FOREIGN KEY(project_id) REFERENCES projects(id)
            );

            CREATE TABLE IF NOT EXISTS tool_model_stats (
                project_id INTEGER NOT NULL,
                tool_model TEXT NOT NULL,
                ai_additions INTEGER NOT NULL,
                mixed_additions INTEGER NOT NULL,
                ai_accepted INTEGER NOT NULL,
                total_ai_additions INTEGER NOT NULL,
                total_ai_deletions INTEGER NOT NULL,
                time_waiting_for_ai INTEGER NOT NULL,
                PRIMARY KEY(project_id, tool_model),
                FOREIGN KEY(project_id) REFERENCES projects(id)
            );

            -- 组织字典表
            CREATE TABLE IF NOT EXISTS organizations (
                id   INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE
            );

            -- 部门字典表
            CREATE TABLE IF NOT EXISTS departments (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                organization_id INTEGER NOT NULL,
                name            TEXT NOT NULL,
                UNIQUE(organization_id, name),
                FOREIGN KEY(organization_id) REFERENCES organizations(id)
            );

            -- 每次上报独立一行，不同上报人之间不相互覆盖
            -- 唯一约束：同一上报人 + 同一项目(name+url+branch) + 同一统计周期 只保留最新一条
            -- 注意：git_url/branch/report_period/reporter_email 统一用空字符串哨兵，不存 NULL
            CREATE TABLE IF NOT EXISTS project_summaries (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                organization_id INTEGER,
                department_id   INTEGER,
                project_name    TEXT NOT NULL,
                git_url         TEXT NOT NULL DEFAULT '',
                branch          TEXT NOT NULL DEFAULT '',
                report_period   TEXT NOT NULL DEFAULT '',
                reporter_name   TEXT,
                reporter_email  TEXT NOT NULL DEFAULT '',
                total_commits   INTEGER NOT NULL,
                ai_ratio        REAL NOT NULL,
                human_ratio     REAL NOT NULL,
                schema_version  TEXT NOT NULL,
                created_at      TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at      TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(organization_id) REFERENCES organizations(id),
                FOREIGN KEY(department_id)   REFERENCES departments(id),
                UNIQUE(reporter_email, project_name, git_url, branch, report_period)
            );

            CREATE TABLE IF NOT EXISTS developer_summaries (
                id                 INTEGER PRIMARY KEY AUTOINCREMENT,
                project_summary_id INTEGER NOT NULL,
                name               TEXT NOT NULL,
                email              TEXT NOT NULL,
                commits            INTEGER NOT NULL,
                added_lines        INTEGER NOT NULL,
                ai_additions       INTEGER NOT NULL,
                human_additions    INTEGER NOT NULL,
                ai_ratio           REAL NOT NULL,
                human_ratio        REAL NOT NULL,
                FOREIGN KEY(project_summary_id) REFERENCES project_summaries(id)
            );
            "#,
        )?;
        Ok(())
    }

    pub fn ingest_report(&mut self, report: &ReportDocument) -> Result<IngestResponse, GitAiError> {
        if report.schema_version != REPORT_SCHEMA_VERSION {
            return Err(GitAiError::Generic(format!(
                "Unsupported report schema '{}'",
                report.schema_version
            )));
        }

        let project_key = report
            .repo
            .remote_url_hash
            .clone()
            .or_else(|| {
                report
                    .repo
                    .head_commit
                    .as_ref()
                    .map(|sha| format!("local:{}", sha))
            })
            .unwrap_or_else(|| "local:unknown".to_string());

        let tx = self.conn.transaction()?;
        tx.execute(
            r#"
            INSERT INTO projects (remote_url_hash, branch, head_commit)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(remote_url_hash) DO UPDATE SET
                branch = excluded.branch,
                head_commit = excluded.head_commit,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![project_key, report.repo.branch, report.repo.head_commit],
        )?;
        let project_id: i64 = tx.query_row(
            "SELECT id FROM projects WHERE remote_url_hash = ?1",
            params![project_key],
            |row| row.get(0),
        )?;

        tx.execute(
            r#"
            INSERT INTO report_uploads (project_id, schema_version, generated_at, commit_count)
            VALUES (?1, ?2, ?3, ?4)
            "#,
            params![
                project_id,
                report.schema_version,
                report.generated_at,
                report.commits.len() as i64
            ],
        )?;
        let upload_id = tx.last_insert_rowid();

        let mut inserted_commits = 0usize;
        let mut duplicate_commits = 0usize;
        for commit in &report.commits {
            let changed = tx.execute(
                r#"
                INSERT OR IGNORE INTO commit_stats (
                    project_id, sha, author, author_time, subject, has_authorship_note,
                    git_diff_added_lines, git_diff_deleted_lines, ai_additions, human_additions,
                    mixed_additions, unknown_additions, ai_accepted, total_ai_additions,
                    total_ai_deletions, time_waiting_for_ai
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                "#,
                params![
                    project_id,
                    commit.sha,
                    commit.author,
                    commit.author_time,
                    commit.subject,
                    commit.has_authorship_note as i64,
                    commit.stats.git_diff_added_lines,
                    commit.stats.git_diff_deleted_lines,
                    commit.stats.ai_additions,
                    commit.stats.human_additions,
                    commit.stats.mixed_additions,
                    commit.stats.unknown_additions,
                    commit.stats.ai_accepted,
                    commit.stats.total_ai_additions,
                    commit.stats.total_ai_deletions,
                    commit.stats.time_waiting_for_ai,
                ],
            )?;
            if changed == 0 {
                duplicate_commits += 1;
            } else {
                inserted_commits += 1;
            }
        }

        tx.execute(
            "DELETE FROM tool_model_stats WHERE project_id = ?1",
            params![project_id],
        )?;
        for (tool_model, stats) in &report.tool_model_breakdown {
            insert_tool_model_stats(&tx, project_id, tool_model, stats)?;
        }

        tx.commit()?;
        Ok(IngestResponse {
            project_id,
            upload_id,
            inserted_commits,
            duplicate_commits,
        })
    }

    pub fn list_projects(&self) -> Result<Vec<serde_json::Value>, GitAiError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT p.id, p.remote_url_hash, p.branch, p.head_commit, COUNT(c.sha) AS commit_count
            FROM projects p
            LEFT JOIN commit_stats c ON c.project_id = p.id
            GROUP BY p.id
            ORDER BY p.updated_at DESC, p.id DESC
            "#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "remote_url_hash": row.get::<_, String>(1)?,
                "branch": row.get::<_, Option<String>>(2)?,
                "head_commit": row.get::<_, Option<String>>(3)?,
                "commit_count": row.get::<_, i64>(4)?,
            }))
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(GitAiError::from)
    }

    pub fn project_summary(&self, project_id: i64) -> Result<ProjectSummaryResponse, GitAiError> {
        ensure_project_exists(&self.conn, project_id)?;
        let (commit_count, summary) = self.conn.query_row(
            r#"
            SELECT
                COUNT(*),
                COALESCE(SUM(git_diff_added_lines), 0),
                COALESCE(SUM(git_diff_deleted_lines), 0),
                COALESCE(SUM(ai_additions), 0),
                COALESCE(SUM(human_additions), 0),
                COALESCE(SUM(mixed_additions), 0),
                COALESCE(SUM(unknown_additions), 0),
                COALESCE(SUM(ai_accepted), 0),
                COALESCE(SUM(total_ai_additions), 0),
                COALESCE(SUM(total_ai_deletions), 0),
                COALESCE(SUM(time_waiting_for_ai), 0)
            FROM commit_stats
            WHERE project_id = ?1
            "#,
            params![project_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as usize,
                    ReportSummary {
                        git_diff_added_lines: row.get::<_, i64>(1)? as u32,
                        git_diff_deleted_lines: row.get::<_, i64>(2)? as u32,
                        ai_additions: row.get::<_, i64>(3)? as u32,
                        human_additions: row.get::<_, i64>(4)? as u32,
                        mixed_additions: row.get::<_, i64>(5)? as u32,
                        unknown_additions: row.get::<_, i64>(6)? as u32,
                        ai_accepted: row.get::<_, i64>(7)? as u32,
                        total_ai_additions: row.get::<_, i64>(8)? as u32,
                        total_ai_deletions: row.get::<_, i64>(9)? as u32,
                        time_waiting_for_ai: row.get::<_, i64>(10)? as u64,
                    },
                ))
            },
        )?;
        Ok(ProjectSummaryResponse {
            project_id,
            commit_count,
            summary,
        })
    }

    pub fn project_commits(&self, project_id: i64) -> Result<Vec<serde_json::Value>, GitAiError> {
        ensure_project_exists(&self.conn, project_id)?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT sha, author, author_time, subject, has_authorship_note,
                   git_diff_added_lines, git_diff_deleted_lines,
                   ai_additions, human_additions, mixed_additions, unknown_additions
            FROM commit_stats
            WHERE project_id = ?1
            ORDER BY author_time ASC, sha ASC
            "#,
        )?;
        let rows = stmt.query_map(params![project_id], |row| {
            Ok(serde_json::json!({
                "sha": row.get::<_, String>(0)?,
                "author": row.get::<_, String>(1)?,
                "author_time": row.get::<_, String>(2)?,
                "subject": row.get::<_, String>(3)?,
                "has_authorship_note": row.get::<_, i64>(4)? != 0,
                "git_diff_added_lines": row.get::<_, i64>(5)?,
                "git_diff_deleted_lines": row.get::<_, i64>(6)?,
                "ai_additions": row.get::<_, i64>(7)?,
                "human_additions": row.get::<_, i64>(8)?,
                "mixed_additions": row.get::<_, i64>(9)?,
                "unknown_additions": row.get::<_, i64>(10)?,
            }))
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(GitAiError::from)
    }

    // -----------------------------------------------------------------------
    // Summary report ingestion & retrieval
    // -----------------------------------------------------------------------

    pub fn ingest_summary(
        &mut self,
        summary: &ProjectSummaryReport,
    ) -> Result<SummaryIngestResponse, GitAiError> {
        let tx = self.conn.transaction()?;

        // --- 1. 解析组织/部门，自动 upsert 字典表 ---
        let org_id: Option<i64> = if let Some(org_name) = summary.organization.as_deref()
            && !org_name.trim().is_empty()
        {
            tx.execute(
                "INSERT INTO organizations (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
                params![org_name],
            )?;
            let id: i64 = tx.query_row(
                "SELECT id FROM organizations WHERE name = ?1",
                params![org_name],
                |row| row.get(0),
            )?;
            Some(id)
        } else {
            None
        };

        let dept_id: Option<i64> = if let Some(dept_name) = summary.department.as_deref()
            && !dept_name.trim().is_empty()
        {
            if let Some(oid) = org_id {
                tx.execute(
                    "INSERT INTO departments (organization_id, name) VALUES (?1, ?2) ON CONFLICT(organization_id, name) DO NOTHING",
                    params![oid, dept_name],
                )?;
                let id: i64 = tx.query_row(
                    "SELECT id FROM departments WHERE organization_id = ?1 AND name = ?2",
                    params![oid, dept_name],
                    |row| row.get(0),
                )?;
                Some(id)
            } else {
                // 无组织时，部门挂在 org_id=NULL 下，仍可按 name 查询，但不写字典表
                None
            }
        } else {
            None
        };

        // --- 2. 规范化 NULL 值（用空字符串哨兵，与 UNIQUE 约束对齐） ---
        let git_url = summary.git_url.clone().unwrap_or_default();
        let branch = summary.branch.clone().unwrap_or_default();
        let report_period = summary.report_period.clone().unwrap_or_default();
        let reporter_email = summary.reporter_email.clone().unwrap_or_default();
        let reporter_name = summary.reporter_name.clone().unwrap_or_default();

        let reporter_name_sql: Option<&str> = if reporter_name.is_empty() { None } else { Some(reporter_name.as_str()) };

        // --- 3. 查找同一上报人的同项目同周期已有记录 ---
        let existing_id: Option<i64> = tx
            .query_row(
                r#"SELECT id FROM project_summaries
                   WHERE reporter_email = ?1
                     AND project_name   = ?2
                     AND git_url        = ?3
                     AND branch         = ?4
                     AND report_period  = ?5"#,
                params![reporter_email, summary.project_name, git_url, branch, report_period],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        let summary_id = if let Some(id) = existing_id {
            // 同一上报人重复上传 → UPDATE
            tx.execute(
                r#"UPDATE project_summaries SET
                    organization_id = ?1,
                    department_id   = ?2,
                    reporter_name   = ?3,
                    total_commits   = ?4,
                    ai_ratio        = ?5,
                    human_ratio     = ?6,
                    schema_version  = ?7,
                    updated_at      = CURRENT_TIMESTAMP
                   WHERE id = ?8"#,
                params![
                    org_id,
                    dept_id,
                    reporter_name_sql,
                    summary.total_commits as i64,
                    summary.project_ratios.ai,
                    summary.project_ratios.human,
                    DEVELOPER_SUMMARY_SCHEMA_VERSION,
                    id,
                ],
            )?;
            id
        } else {
            // 新上报人或新周期 → INSERT
            tx.execute(
                r#"INSERT INTO project_summaries
                    (organization_id, department_id, project_name, git_url, branch,
                     report_period, reporter_name, reporter_email,
                     total_commits, ai_ratio, human_ratio, schema_version)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
                params![
                    org_id,
                    dept_id,
                    summary.project_name,
                    git_url,
                    branch,
                    report_period,
                    reporter_name_sql,
                    reporter_email,
                    summary.total_commits as i64,
                    summary.project_ratios.ai,
                    summary.project_ratios.human,
                    DEVELOPER_SUMMARY_SCHEMA_VERSION,
                ],
            )?;
            tx.last_insert_rowid()
        };

        // --- 4. 替换该条上报记录的开发者列表 ---
        tx.execute(
            "DELETE FROM developer_summaries WHERE project_summary_id = ?1",
            params![summary_id],
        )?;
        for dev in &summary.developers {
            tx.execute(
                r#"INSERT INTO developer_summaries
                    (project_summary_id, name, email, commits, added_lines,
                     ai_additions, human_additions, ai_ratio, human_ratio)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"#,
                params![
                    summary_id,
                    dev.name,
                    dev.email,
                    dev.commits as i64,
                    dev.added_lines as i64,
                    dev.ai_additions as i64,
                    dev.human_additions as i64,
                    dev.ai_ratio,
                    dev.human_ratio,
                ],
            )?;
        }

        tx.commit()?;

        Ok(SummaryIngestResponse {
            summary_id,
            project_name: summary.project_name.clone(),
            developer_count: summary.developers.len(),
            organization: summary.organization.clone(),
            department: summary.department.clone(),
        })
    }

    pub fn list_project_summaries(&self) -> Result<Vec<serde_json::Value>, GitAiError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                ps.id, ps.project_name, ps.git_url, ps.branch,
                ps.report_period, ps.reporter_name, ps.reporter_email,
                ps.total_commits, ps.ai_ratio, ps.human_ratio,
                ps.created_at, ps.updated_at,
                o.name AS organization,
                d.name AS department
            FROM project_summaries ps
            LEFT JOIN organizations o ON o.id = ps.organization_id
            LEFT JOIN departments   d ON d.id = ps.department_id
            ORDER BY ps.updated_at DESC, ps.id DESC
            "#,
        )?;
        let rows = stmt.query_map([], |row| {
            let git_url: String = row.get(2)?;
            let branch: String = row.get(3)?;
            Ok(serde_json::json!({
                "id":             row.get::<_, i64>(0)?,
                "project_name":   row.get::<_, String>(1)?,
                "git_url":        if git_url.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(git_url) },
                "branch":         if branch.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(branch) },
                "report_period":  row.get::<_, Option<String>>(4)?,
                "reporter_name":  row.get::<_, Option<String>>(5)?,
                "reporter_email": row.get::<_, Option<String>>(6)?,
                "total_commits":  row.get::<_, i64>(7)?,
                "ai_ratio":       row.get::<_, f64>(8)?,
                "human_ratio":    row.get::<_, f64>(9)?,
                "created_at":     row.get::<_, String>(10)?,
                "updated_at":     row.get::<_, String>(11)?,
                "organization":   row.get::<_, Option<String>>(12)?,
                "department":     row.get::<_, Option<String>>(13)?,
            }))
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(GitAiError::from)
    }

    pub fn get_project_summary_detail(
        &self,
        summary_id: i64,
    ) -> Result<ProjectSummaryReport, GitAiError> {
        let row = self.conn.query_row(
            r#"
            SELECT
                ps.project_name, ps.git_url, ps.branch, ps.total_commits,
                ps.ai_ratio, ps.human_ratio,
                ps.organization_id, ps.department_id,
                ps.reporter_name, ps.reporter_email, ps.report_period,
                o.name AS organization, d.name AS department
            FROM project_summaries ps
            LEFT JOIN organizations o ON o.id = ps.organization_id
            LEFT JOIN departments   d ON d.id = ps.department_id
            WHERE ps.id = ?1
            "#,
            params![summary_id],
            |row| {
                let git_url: String = row.get(1)?;
                let branch: String = row.get(2)?;
                let reporter_email: String = row.get(9)?;
                Ok((
                    row.get::<_, String>(0)?,
                    if git_url.is_empty() { None } else { Some(git_url) },
                    if branch.is_empty() { None } else { Some(branch) },
                    row.get::<_, i64>(3)? as usize,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, Option<String>>(8)?,  // reporter_name
                    if reporter_email.is_empty() { None } else { Some(reporter_email) },
                    row.get::<_, Option<String>>(10)?, // report_period
                    row.get::<_, Option<String>>(11)?, // organization
                    row.get::<_, Option<String>>(12)?, // department
                ))
            },
        )?;

        let (project_name, git_url, branch, total_commits, ai_ratio, human_ratio,
             reporter_name, reporter_email, report_period, organization, department) = row;

        let mut stmt = self.conn.prepare(
            r#"
            SELECT name, email, commits, added_lines, ai_additions, human_additions, ai_ratio, human_ratio
            FROM developer_summaries
            WHERE project_summary_id = ?1
            ORDER BY added_lines DESC
            "#,
        )?;
        let developers = stmt
            .query_map(params![summary_id], |row| {
                Ok(DeveloperSummary {
                    name: row.get::<_, String>(0)?,
                    email: row.get::<_, String>(1)?,
                    commits: row.get::<_, i64>(2)? as usize,
                    added_lines: row.get::<_, i64>(3)? as u32,
                    ai_additions: row.get::<_, i64>(4)? as u32,
                    human_additions: row.get::<_, i64>(5)? as u32,
                    ai_ratio: row.get::<_, f64>(6)?,
                    human_ratio: row.get::<_, f64>(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(GitAiError::from)?;

        Ok(ProjectSummaryReport {
            project_name,
            git_url,
            branch,
            total_commits,
            developers,
            project_ratios: ProjectRatios { ai: ai_ratio, human: human_ratio },
            organization,
            department,
            reporter_name,
            reporter_email,
            report_period,
        })
    }

    // -----------------------------------------------------------------------
    // 聚合查询
    // -----------------------------------------------------------------------

    /// 全局汇总：总报告数、项目数、开发者数、组织数、加权 AI 比率
    pub fn aggregate_global(&self) -> Result<GlobalAggregateSummary, GitAiError> {
        let (total_reports, total_projects, total_commits, w_ai, w_human): (i64, i64, i64, f64, f64) =
            self.conn.query_row(
                r#"
                SELECT
                    COUNT(*),
                    COUNT(DISTINCT project_name || COALESCE(git_url, '') || COALESCE(branch, '')),
                    COALESCE(SUM(total_commits), 0),
                    COALESCE(SUM(ai_ratio    * total_commits), 0.0),
                    COALESCE(SUM(human_ratio * total_commits), 0.0)
                FROM project_summaries
                "#,
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )?;

        let total_developers: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT email) FROM developer_summaries",
            [],
            |row| row.get(0),
        )?;

        let total_organizations: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM organizations",
            [],
            |row| row.get(0),
        )?;

        let total_departments: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM departments",
            [],
            |row| row.get(0),
        )?;

        let (weighted_ai_ratio, weighted_human_ratio) = if total_commits > 0 {
            (w_ai / total_commits as f64, w_human / total_commits as f64)
        } else {
            (0.0, 0.0)
        };

        Ok(GlobalAggregateSummary {
            total_reports: total_reports as usize,
            total_projects: total_projects as usize,
            total_developers: total_developers as usize,
            total_organizations: total_organizations as usize,
            total_departments: total_departments as usize,
            weighted_ai_ratio,
            weighted_human_ratio,
        })
    }

    /// 按组织聚合
    pub fn aggregate_by_org(&self) -> Result<Vec<OrgAggregateSummary>, GitAiError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                o.name,
                COUNT(DISTINCT ps.id),
                COUNT(DISTINCT ds.email),
                COALESCE(SUM(ps.total_commits), 0),
                COALESCE(SUM(ps.ai_ratio    * ps.total_commits), 0.0),
                COALESCE(SUM(ps.human_ratio * ps.total_commits), 0.0)
            FROM organizations o
            LEFT JOIN project_summaries ps ON ps.organization_id = o.id
            LEFT JOIN developer_summaries ds ON ds.project_summary_id = ps.id
            GROUP BY o.id, o.name
            ORDER BY o.name ASC
            "#,
        )?;
        let rows = stmt.query_map([], |row| {
            let total_commits: i64 = row.get(3)?;
            let w_ai: f64 = row.get(4)?;
            let w_human: f64 = row.get(5)?;
            let (ai_ratio, human_ratio) = if total_commits > 0 {
                (w_ai / total_commits as f64, w_human / total_commits as f64)
            } else {
                (0.0, 0.0)
            };
            Ok(OrgAggregateSummary {
                organization: row.get(0)?,
                project_count: row.get::<_, i64>(1)? as usize,
                developer_count: row.get::<_, i64>(2)? as usize,
                total_commits,
                weighted_ai_ratio: ai_ratio,
                weighted_human_ratio: human_ratio,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(GitAiError::from)
    }

    /// 按部门聚合，可按 org 名称过滤
    pub fn aggregate_by_dept(&self, org_filter: Option<&str>) -> Result<Vec<DeptAggregateSummary>, GitAiError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                o.name,
                d.name,
                COUNT(DISTINCT ps.id),
                COUNT(DISTINCT ds.email),
                COALESCE(SUM(ps.total_commits), 0),
                COALESCE(SUM(ps.ai_ratio    * ps.total_commits), 0.0),
                COALESCE(SUM(ps.human_ratio * ps.total_commits), 0.0)
            FROM departments d
            JOIN organizations o ON o.id = d.organization_id
            LEFT JOIN project_summaries ps ON ps.department_id = d.id
            LEFT JOIN developer_summaries ds ON ds.project_summary_id = ps.id
            WHERE (?1 IS NULL OR o.name = ?1)
            GROUP BY d.id, o.name, d.name
            ORDER BY o.name ASC, d.name ASC
            "#,
        )?;
        let rows = stmt.query_map(params![org_filter], |row| {
            let total_commits: i64 = row.get(4)?;
            let w_ai: f64 = row.get(5)?;
            let w_human: f64 = row.get(6)?;
            let (ai_ratio, human_ratio) = if total_commits > 0 {
                (w_ai / total_commits as f64, w_human / total_commits as f64)
            } else {
                (0.0, 0.0)
            };
            Ok(DeptAggregateSummary {
                organization: row.get(0)?,
                department: row.get(1)?,
                project_count: row.get::<_, i64>(2)? as usize,
                developer_count: row.get::<_, i64>(3)? as usize,
                total_commits,
                weighted_ai_ratio: ai_ratio,
                weighted_human_ratio: human_ratio,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(GitAiError::from)
    }

    /// 按项目聚合（跨多个上报人），可按 org/dept 过滤
    pub fn aggregate_by_project(
        &self,
        org_filter: Option<&str>,
        dept_filter: Option<&str>,
    ) -> Result<Vec<ProjectAggregateSummary>, GitAiError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                ps.project_name,
                ps.git_url,
                ps.branch,
                o.name AS organization,
                d.name AS department,
                COUNT(DISTINCT ps.id),
                COALESCE(SUM(ps.total_commits), 0),
                COALESCE(SUM(ps.ai_ratio    * ps.total_commits), 0.0),
                COALESCE(SUM(ps.human_ratio * ps.total_commits), 0.0)
            FROM project_summaries ps
            LEFT JOIN organizations o ON o.id = ps.organization_id
            LEFT JOIN departments   d ON d.id = ps.department_id
            WHERE (?1 IS NULL OR o.name = ?1)
              AND (?2 IS NULL OR d.name = ?2)
            GROUP BY ps.project_name, COALESCE(ps.git_url,''), COALESCE(ps.branch,''),
                     ps.organization_id, ps.department_id
            ORDER BY SUM(ps.total_commits) DESC
            "#,
        )?;
        let rows = stmt.query_map(params![org_filter, dept_filter], |row| {
            let total_commits: i64 = row.get(6)?;
            let w_ai: f64 = row.get(7)?;
            let w_human: f64 = row.get(8)?;
            let (ai_ratio, human_ratio) = if total_commits > 0 {
                (w_ai / total_commits as f64, w_human / total_commits as f64)
            } else {
                (0.0, 0.0)
            };
            let git_url: String = row.get(1)?;
            let branch: String = row.get(2)?;
            Ok(ProjectAggregateSummary {
                project_name: row.get(0)?,
                git_url: if git_url.is_empty() { None } else { Some(git_url) },
                branch: if branch.is_empty() { None } else { Some(branch) },
                organization: row.get(3)?,
                department: row.get(4)?,
                report_count: row.get::<_, i64>(5)? as usize,
                total_commits,
                weighted_ai_ratio: ai_ratio,
                weighted_human_ratio: human_ratio,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(GitAiError::from)
    }

    /// 按开发者跨项目聚合，可按 org/dept 过滤
    pub fn aggregate_by_developer(
        &self,
        org_filter: Option<&str>,
        dept_filter: Option<&str>,
    ) -> Result<Vec<DeveloperAggregateSummary>, GitAiError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                ds.name,
                ds.email,
                o.name AS organization,
                d.name AS department,
                COUNT(DISTINCT ps.id),
                COALESCE(SUM(ds.commits), 0),
                COALESCE(SUM(ds.added_lines), 0),
                COALESCE(SUM(ds.ai_additions), 0),
                COALESCE(SUM(ds.human_additions), 0)
            FROM developer_summaries ds
            JOIN project_summaries ps ON ps.id = ds.project_summary_id
            LEFT JOIN organizations o ON o.id = ps.organization_id
            LEFT JOIN departments   d ON d.id = ps.department_id
            WHERE (?1 IS NULL OR o.name = ?1)
              AND (?2 IS NULL OR d.name = ?2)
            GROUP BY ds.email, ps.organization_id, ps.department_id
            ORDER BY SUM(ds.added_lines) DESC
            "#,
        )?;
        let rows = stmt.query_map(params![org_filter, dept_filter], |row| {
            let total_ai: i64 = row.get(7)?;
            let total_human: i64 = row.get(8)?;
            let total = total_ai + total_human;
            let (ai_ratio, human_ratio) = if total > 0 {
                (total_ai as f64 / total as f64, total_human as f64 / total as f64)
            } else {
                (0.0, 0.0)
            };
            Ok(DeveloperAggregateSummary {
                name: row.get(0)?,
                email: row.get(1)?,
                organization: row.get(2)?,
                department: row.get(3)?,
                project_count: row.get::<_, i64>(4)? as usize,
                total_commits: row.get(5)?,
                total_added_lines: row.get(6)?,
                total_ai_additions: total_ai,
                total_human_additions: total_human,
                weighted_ai_ratio: ai_ratio,
                weighted_human_ratio: human_ratio,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(GitAiError::from)
    }
}

fn insert_tool_model_stats(
    conn: &Connection,
    project_id: i64,
    tool_model: &str,
    stats: &ToolModelHeadlineStats,
) -> Result<(), GitAiError> {
    conn.execute(
        r#"
        INSERT INTO tool_model_stats (
            project_id, tool_model, ai_additions, mixed_additions, ai_accepted,
            total_ai_additions, total_ai_deletions, time_waiting_for_ai
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            project_id,
            tool_model,
            stats.ai_additions,
            stats.mixed_additions,
            stats.ai_accepted,
            stats.total_ai_additions,
            stats.total_ai_deletions,
            stats.time_waiting_for_ai,
        ],
    )?;
    Ok(())
}

fn ensure_project_exists(conn: &Connection, project_id: i64) -> Result<(), GitAiError> {
    let exists = conn
        .query_row(
            "SELECT 1 FROM projects WHERE id = ?1",
            params![project_id],
            |_| Ok(()),
        )
        .optional()?;
    exists.ok_or_else(|| GitAiError::Generic(format!("Project {} not found", project_id)))
}

pub fn serve(addr: &str, db_path: &Path) -> Result<(), GitAiError> {
    let listener = TcpListener::bind(addr)?;
    eprintln!("git-ai report server listening on http://{}", addr);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(error) = handle_stream(stream, db_path) {
                    eprintln!("report server request failed: {}", error);
                }
            }
            Err(error) => eprintln!("report server connection failed: {}", error),
        }
    }

    Ok(())
}

fn handle_stream(mut stream: TcpStream, db_path: &Path) -> Result<(), GitAiError> {
    let buffer = read_http_request(&mut stream)?;
    let response = handle_http_request(&buffer, db_path);
    stream.write_all(&response)?;
    Ok(())
}

fn read_http_request(stream: &mut TcpStream) -> Result<Vec<u8>, GitAiError> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_marker = b"\r\n\r\n";

    loop {
        let bytes_read = stream.read(&mut chunk)?;
        if bytes_read == 0 {
            return Ok(buffer);
        }
        buffer.extend_from_slice(&chunk[..bytes_read]);
        if let Some(header_end) = buffer
            .windows(header_marker.len())
            .position(|window| window == header_marker)
        {
            let headers = std::str::from_utf8(&buffer[..header_end])
                .map_err(|e| GitAiError::Generic(format!("Invalid HTTP headers: {}", e)))?;
            let content_length = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            let expected_len = header_end + header_marker.len() + content_length;
            while buffer.len() < expected_len {
                let bytes_read = stream.read(&mut chunk)?;
                if bytes_read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..bytes_read]);
            }
            buffer.truncate(expected_len.min(buffer.len()));
            return Ok(buffer);
        }
    }
}

fn handle_http_request(request: &[u8], db_path: &Path) -> Vec<u8> {
    let Ok((method, path, body)) = parse_http_request(request) else {
        return json_response(
            400,
            &ErrorResponse {
                error: "Malformed HTTP request".to_string(),
            },
        );
    };

    let result = (|| -> Result<Vec<u8>, GitAiError> {
        let mut store = ReportStore::open(db_path)?;
        match (method.as_str(), path.as_str()) {
            ("POST", "/api/v1/reports") => {
                let report: ReportDocument = serde_json::from_slice(body)?;
                let response = store.ingest_report(&report)?;
                Ok(json_response(201, &response))
            }
            ("POST", "/api/v1/summaries") => {
                let summary: ProjectSummaryReport = serde_json::from_slice(body)?;
                let response = store.ingest_summary(&summary)?;
                Ok(json_response(201, &response))
            }
            ("GET", "/api/v1/projects") => {
                let projects = store.list_projects()?;
                Ok(json_response(200, &projects))
            }
            ("GET", "/api/v1/summaries") => {
                let summaries = store.list_project_summaries()?;
                Ok(json_response(200, &summaries))
            }
            // 聚合 API
            ("GET", "/api/v1/aggregate/summary") => {
                let summary = store.aggregate_global()?;
                Ok(json_response(200, &summary))
            }
            ("GET", "/api/v1/aggregate/organizations") => {
                let list = store.aggregate_by_org()?;
                Ok(json_response(200, &list))
            }
            ("GET", path) if path.starts_with("/api/v1/aggregate/departments") => {
                let org_filter = parse_query_param(path, "org");
                let list = store.aggregate_by_dept(org_filter.as_deref())?;
                Ok(json_response(200, &list))
            }
            ("GET", path) if path.starts_with("/api/v1/aggregate/projects") => {
                let org_filter = parse_query_param(path, "org");
                let dept_filter = parse_query_param(path, "dept");
                let list = store.aggregate_by_project(org_filter.as_deref(), dept_filter.as_deref())?;
                Ok(json_response(200, &list))
            }
            ("GET", path) if path.starts_with("/api/v1/aggregate/developers") => {
                let org_filter = parse_query_param(path, "org");
                let dept_filter = parse_query_param(path, "dept");
                let list = store.aggregate_by_developer(org_filter.as_deref(), dept_filter.as_deref())?;
                Ok(json_response(200, &list))
            }
            ("GET", path)
                if path.starts_with("/api/v1/projects/") && path.ends_with("/summary") =>
            {
                let project_id = parse_project_id(path, "/summary")?;
                let summary = store.project_summary(project_id)?;
                Ok(json_response(200, &summary))
            }
            ("GET", path)
                if path.starts_with("/api/v1/projects/") && path.ends_with("/commits") =>
            {
                let project_id = parse_project_id(path, "/commits")?;
                let commits = store.project_commits(project_id)?;
                Ok(json_response(200, &commits))
            }
            ("GET", path)
                if path.starts_with("/api/v1/summaries/")
                    && path.trim_start_matches("/api/v1/summaries/")
                        .trim_end_matches('/')
                        .parse::<i64>()
                        .is_ok() =>
            {
                let summary_id = parse_summary_id(path)?;
                let detail = store.get_project_summary_detail(summary_id)?;
                Ok(json_response(200, &detail))
            }
            ("GET", "/") | ("GET", "/dashboard") => Ok(html_response(200, DASHBOARD_HTML)),
            _ => Ok(json_response(
                404,
                &ErrorResponse {
                    error: "Not found".to_string(),
                },
            )),
        }
    })();

    match result {
        Ok(response) => response,
        Err(error) => json_response(
            400,
            &ErrorResponse {
                error: error.to_string(),
            },
        ),
    }
}

fn parse_http_request(request: &[u8]) -> Result<(String, String, &[u8]), GitAiError> {
    let marker = b"\r\n\r\n";
    let header_end = request
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or_else(|| GitAiError::Generic("HTTP headers not terminated".to_string()))?;
    let headers = std::str::from_utf8(&request[..header_end])
        .map_err(|e| GitAiError::Generic(format!("Invalid HTTP headers: {}", e)))?;
    let mut lines = headers.lines();
    let request_line = lines
        .next()
        .ok_or_else(|| GitAiError::Generic("Missing HTTP request line".to_string()))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| GitAiError::Generic("Missing HTTP method".to_string()))?
        .to_string();
    let path = request_parts
        .next()
        .ok_or_else(|| GitAiError::Generic("Missing HTTP path".to_string()))?
        .to_string();
    let body = &request[header_end + marker.len()..];
    Ok((method, path, body))
}

fn parse_query_param(path: &str, key: &str) -> Option<String> {
    let query = path.splitn(2, '?').nth(1)?;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key && !v.is_empty() {
                // 简单 URL decode：将 + 替换为空格，%XX 保持原样（足够用于组织/部门名称）
                return Some(v.replace('+', " "));
            }
        }
    }
    None
}

fn parse_project_id(path: &str, suffix: &str) -> Result<i64, GitAiError> {
    path.trim_start_matches("/api/v1/projects/")
        .trim_end_matches(suffix)
        .parse::<i64>()
        .map_err(|e| GitAiError::Generic(format!("Invalid project id: {}", e)))
}

fn parse_summary_id(path: &str) -> Result<i64, GitAiError> {
    path.trim_start_matches("/api/v1/summaries/")
        .trim_end_matches('/')
        .parse::<i64>()
        .map_err(|e| GitAiError::Generic(format!("Invalid summary id: {}", e)))
}

fn json_response<T: Serialize>(status: u16, value: &T) -> Vec<u8> {
    let status_text = match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    let body = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
    format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
        status,
        status_text,
        body.len(),
        body
    )
    .into_bytes()
}

fn html_response(status: u16, html: &str) -> Vec<u8> {
    let status_text = match status {
        200 => "OK",
        _ => "OK",
    };
    format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        status_text,
        html.len(),
        html
    )
    .into_bytes()
}

const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Git-AI 编码分析仪表盘</title>
<script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.0/dist/chart.umd.min.js"></script>
<style>
  :root {
    font-size: 112.5%;
    --primary: #6366f1;
    --primary-light: #818cf8;
    --secondary: #06b6d4;
    --success: #22c55e;
    --warning: #f59e0b;
    --danger: #ef4444;
    --bg: #0f172a;
    --bg-card: #1e293b;
    --bg-card2: #263146;
    --border: #334155;
    --text: #e2e8f0;
    --text-muted: #94a3b8;
    --ai-color: #6366f1;
    --human-color: #22c55e;
  }
  * { box-sizing: border-box; margin: 0; padding: 0; }
  body { font-family: 'Inter', -apple-system, sans-serif; background: var(--bg); color: var(--text); min-height: 100vh; }
  .header { background: linear-gradient(135deg, #1e293b 0%, #0f172a 100%); border-bottom: 1px solid var(--border); padding: 16px 24px; display: flex; align-items: center; justify-content: space-between; }
  .header h1 { font-size: 1.25rem; font-weight: 700; background: linear-gradient(90deg, var(--primary-light), var(--secondary)); -webkit-background-clip: text; -webkit-text-fill-color: transparent; }
  .header-right { display: flex; align-items: center; gap: 12px; }
  .badge { font-size: 0.6875rem; padding: 3px 8px; border-radius: 9999px; background: #1e293b; border: 1px solid var(--border); color: var(--text-muted); }
  .refresh-btn { background: var(--primary); color: white; border: none; padding: 8px 16px; border-radius: 8px; cursor: pointer; font-size: 0.8125rem; font-weight: 500; transition: opacity .2s; }
  .refresh-btn:hover { opacity: .85; }
  .main { padding: 24px; max-width: 1400px; margin: 0 auto; }
  /* stat cards */
  .stats-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 16px; margin-bottom: 24px; }
  .stat-card { background: var(--bg-card); border: 1px solid var(--border); border-radius: 12px; padding: 20px; position: relative; overflow: hidden; }
  .stat-card::before { content: ''; position: absolute; top: 0; left: 0; right: 0; height: 3px; border-radius: 12px 12px 0 0; }
  .stat-card.ai::before { background: linear-gradient(90deg, var(--ai-color), var(--primary-light)); }
  .stat-card.human::before { background: linear-gradient(90deg, var(--human-color), var(--secondary)); }
  .stat-card.total::before { background: linear-gradient(90deg, var(--warning), var(--danger)); }
  .stat-card.org::before { background: linear-gradient(90deg, var(--secondary), var(--primary)); }
  .stat-label { font-size: 0.75rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: .05em; margin-bottom: 8px; }
  .stat-value { font-size: 2rem; font-weight: 700; line-height: 1; }
  .stat-value.ai { color: var(--ai-color); }
  .stat-value.human { color: var(--human-color); }
  .stat-value.total { color: var(--warning); }
  .stat-value.org { color: var(--secondary); }
  .stat-sub { font-size: 0.75rem; color: var(--text-muted); margin-top: 6px; }
  /* tabs */
  .tabs { display: flex; gap: 4px; background: var(--bg-card); border: 1px solid var(--border); border-radius: 10px; padding: 4px; margin-bottom: 20px; width: fit-content; }
  .tab { padding: 8px 20px; border-radius: 7px; border: none; background: transparent; color: var(--text-muted); cursor: pointer; font-size: 0.875rem; font-weight: 500; transition: all .2s; }
  .tab.active { background: var(--primary); color: white; }
  .tab:hover:not(.active) { background: var(--bg-card2); color: var(--text); }
  /* panels */
  .panel { display: none; }
  .panel.active { display: block; }
  /* charts grid */
  .charts-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
  .charts-grid.full { grid-template-columns: 1fr; }
  @media (max-width: 900px) { .charts-grid { grid-template-columns: 1fr; } }
  .chart-card { background: var(--bg-card); border: 1px solid var(--border); border-radius: 12px; padding: 20px; }
  .chart-card h3 { font-size: 0.9375rem; font-weight: 600; color: var(--text); margin-bottom: 4px; }
  .chart-card p { font-size: 0.75rem; color: var(--text-muted); margin-bottom: 16px; }
  .chart-wrap { position: relative; height: 280px; }
  /* breadcrumb */
  .breadcrumb { display: flex; align-items: center; gap: 6px; font-size: 0.8125rem; color: var(--text-muted); margin-bottom: 16px; flex-wrap: wrap; }
  .breadcrumb .crumb { cursor: pointer; color: var(--primary-light); text-decoration: underline; text-underline-offset: 2px; }
  .breadcrumb .sep { color: var(--border); }
  .breadcrumb .current { color: var(--text); }
  /* table */
  .table-wrap { overflow-x: auto; }
  table { width: 100%; border-collapse: collapse; font-size: 0.8125rem; }
  thead tr { border-bottom: 1px solid var(--border); }
  th { text-align: left; padding: 10px 12px; color: var(--text-muted); font-weight: 500; white-space: nowrap; }
  td { padding: 10px 12px; border-bottom: 1px solid rgba(51,65,85,.5); }
  tr:hover td { background: var(--bg-card2); }
  .ratio-bar { display: flex; border-radius: 4px; overflow: hidden; height: 6px; width: 100px; background: var(--border); }
  .ratio-bar .ai-seg { background: var(--ai-color); }
  .ratio-bar .human-seg { background: var(--human-color); }
  .tag { display: inline-block; padding: 2px 8px; border-radius: 9999px; font-size: 0.6875rem; font-weight: 600; }
  .tag.high { background: rgba(99,102,241,.2); color: var(--primary-light); }
  .tag.med { background: rgba(34,197,94,.15); color: var(--human-color); }
  .tag.low { background: rgba(148,163,184,.1); color: var(--text-muted); }
  /* loading */
  .loading { display: flex; align-items: center; justify-content: center; height: 200px; color: var(--text-muted); gap: 8px; }
  .spinner { width: 20px; height: 20px; border: 2px solid var(--border); border-top-color: var(--primary); border-radius: 50%; animation: spin .8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .empty { text-align: center; padding: 48px; color: var(--text-muted); }
  .empty-icon { font-size: 2.25rem; margin-bottom: 8px; }
  /* filter bar */
  .filter-bar { display: flex; align-items: center; gap: 10px; margin-bottom: 16px; flex-wrap: wrap; }
  .filter-bar label { font-size: 0.8125rem; color: var(--text-muted); }
  .filter-bar select { background: var(--bg-card2); border: 1px solid var(--border); color: var(--text); padding: 6px 12px; border-radius: 8px; font-size: 0.8125rem; }
  .filter-bar .clear-btn { background: transparent; border: 1px solid var(--border); color: var(--text-muted); padding: 6px 12px; border-radius: 8px; cursor: pointer; font-size: 0.8125rem; }
  .filter-bar .clear-btn:hover { border-color: var(--primary); color: var(--primary-light); }
  /* clickable rows */
  tr.clickable { cursor: pointer; }
  tr.clickable:hover td { background: rgba(99,102,241,.08); }
  /* footer */
  .footer { text-align: center; color: var(--text-muted); font-size: 0.75rem; padding: 24px; border-top: 1px solid var(--border); margin-top: 32px; }
</style>
</head>
<body>
<div class="header">
  <h1>⚡ Git-AI 编码分析仪表盘</h1>
  <div class="header-right">
    <span class="badge" id="last-updated">-</span>
    <button class="refresh-btn" onclick="loadAll()">↻ 刷新</button>
  </div>
</div>

<div class="main">
  <!-- Global stats -->
  <div class="stats-grid" id="global-stats">
    <div class="stat-card ai"><div class="stat-label">全局 AI 编码率</div><div class="stat-value ai" id="g-ai">-</div><div class="stat-sub">加权平均（按提交数）</div></div>
    <div class="stat-card human"><div class="stat-label">全局人工编码率</div><div class="stat-value human" id="g-human">-</div><div class="stat-sub">加权平均（按提交数）</div></div>
    <div class="stat-card total"><div class="stat-label">项目数</div><div class="stat-value total" id="g-projects">-</div><div class="stat-sub" id="g-reports">0 条上报记录</div></div>
    <div class="stat-card org"><div class="stat-label">组织 / 部门</div><div class="stat-value org" id="g-orgs">-</div><div class="stat-sub" id="g-depts">0 个部门</div></div>
    <div class="stat-card ai"><div class="stat-label">开发者数</div><div class="stat-value ai" id="g-devs">-</div><div class="stat-sub">活跃开发者</div></div>
  </div>

  <!-- Tabs -->
  <div class="tabs">
    <button class="tab active" onclick="switchTab('overview')">总览</button>
    <button class="tab" onclick="switchTab('orgs')">组织</button>
    <button class="tab" onclick="switchTab('depts')">部门</button>
    <button class="tab" onclick="switchTab('projects')">项目</button>
    <button class="tab" onclick="switchTab('developers')">开发者</button>
  </div>

  <!-- Overview panel -->
  <div class="panel active" id="panel-overview">
    <div class="charts-grid">
      <div class="chart-card">
        <h3>AI vs 人工编码占比（全局）</h3>
        <p>所有项目加权后的代码来源分布</p>
        <div class="chart-wrap"><canvas id="chart-global-pie"></canvas></div>
      </div>
      <div class="chart-card">
        <h3>各组织 AI 编码率对比</h3>
        <p>点击柱子可下钻查看部门详情</p>
        <div class="chart-wrap"><canvas id="chart-org-bar"></canvas></div>
      </div>
      <div class="chart-card">
        <h3>项目 AI 编码率 TOP 10</h3>
        <p>按加权 AI 比率降序</p>
        <div class="chart-wrap"><canvas id="chart-project-top"></canvas></div>
      </div>
      <div class="chart-card">
        <h3>开发者 AI 编码率 TOP 10</h3>
        <p>按加权 AI 比率降序</p>
        <div class="chart-wrap"><canvas id="chart-dev-top"></canvas></div>
      </div>
    </div>
  </div>

  <!-- Orgs panel -->
  <div class="panel" id="panel-orgs">
    <div class="breadcrumb" id="bc-orgs"><span class="current">所有组织</span></div>
    <div class="chart-card" style="margin-bottom:16px">
      <h3>按组织聚合统计</h3>
      <p>点击行可下钻查看该组织的部门详情</p>
      <div class="chart-wrap" style="height:200px"><canvas id="chart-orgs-bar"></canvas></div>
    </div>
    <div class="chart-card">
      <div class="table-wrap" id="orgs-table-wrap"><div class="loading"><div class="spinner"></div>加载中...</div></div>
    </div>
  </div>

  <!-- Depts panel -->
  <div class="panel" id="panel-depts">
    <div class="filter-bar">
      <label>组织过滤：</label>
      <select id="dept-org-filter" onchange="loadDepts()"><option value="">全部组织</option></select>
      <button class="clear-btn" onclick="document.getElementById('dept-org-filter').value='';loadDepts()">清除</button>
    </div>
    <div class="breadcrumb" id="bc-depts"><span class="current">所有部门</span></div>
    <div class="chart-card" style="margin-bottom:16px">
      <div class="chart-wrap" style="height:200px"><canvas id="chart-depts-bar"></canvas></div>
    </div>
    <div class="chart-card">
      <div class="table-wrap" id="depts-table-wrap"><div class="loading"><div class="spinner"></div>加载中...</div></div>
    </div>
  </div>

  <!-- Projects panel -->
  <div class="panel" id="panel-projects">
    <div class="filter-bar">
      <label>组织：</label>
      <select id="proj-org-filter" onchange="onProjOrgChange()"><option value="">全部</option></select>
      <label>部门：</label>
      <select id="proj-dept-filter" onchange="loadProjects()"><option value="">全部</option></select>
      <button class="clear-btn" onclick="clearProjFilters()">清除</button>
    </div>
    <div class="chart-card" style="margin-bottom:16px">
      <div class="chart-wrap" style="height:220px"><canvas id="chart-projects-bar"></canvas></div>
    </div>
    <div class="chart-card">
      <div class="table-wrap" id="projects-table-wrap"><div class="loading"><div class="spinner"></div>加载中...</div></div>
    </div>
  </div>

  <!-- Developers panel -->
  <div class="panel" id="panel-developers">
    <div class="filter-bar">
      <label>组织：</label>
      <select id="dev-org-filter" onchange="onDevOrgChange()"><option value="">全部</option></select>
      <label>部门：</label>
      <select id="dev-dept-filter" onchange="loadDevelopers()"><option value="">全部</option></select>
      <button class="clear-btn" onclick="clearDevFilters()">清除</button>
    </div>
    <div class="chart-card" style="margin-bottom:16px">
      <div class="chart-wrap" style="height:220px"><canvas id="chart-devs-bar"></canvas></div>
    </div>
    <div class="chart-card">
      <div class="table-wrap" id="devs-table-wrap"><div class="loading"><div class="spinner"></div>加载中...</div></div>
    </div>
  </div>
</div>

<div class="footer">Git-AI Report Dashboard &mdash; 数据实时从 API 获取 &mdash; <span id="server-addr"></span></div>

<script>
const BASE = '';
let charts = {};
let orgsData = [], deptsData = [], projectsData = [], devsData = [];

// ---- helpers ----
async function apiFetch(url) {
  const r = await fetch(BASE + url);
  if (!r.ok) throw new Error('HTTP ' + r.status);
  return r.json();
}
function pct(v) { return (v * 100).toFixed(1) + '%'; }
function aiTag(v) {
  if (v >= 0.6) return '<span class="tag high">高 ' + pct(v) + '</span>';
  if (v >= 0.3) return '<span class="tag med">中 ' + pct(v) + '</span>';
  return '<span class="tag low">低 ' + pct(v) + '</span>';
}
function ratioBar(ai, human) {
  const a = Math.round(ai * 100), h = Math.round(human * 100);
  return `<div class="ratio-bar"><div class="ai-seg" style="width:${a}%"></div><div class="human-seg" style="width:${h}%"></div></div>`;
}
function destroyChart(id) { if (charts[id]) { charts[id].destroy(); delete charts[id]; } }
function makeChart(id, cfg) { destroyChart(id); const ctx = document.getElementById(id); if (ctx) charts[id] = new Chart(ctx, cfg); }

const COLORS = {
  ai: 'rgba(99,102,241,0.85)',
  aiB: 'rgba(99,102,241,1)',
  human: 'rgba(34,197,94,0.85)',
  humanB: 'rgba(34,197,94,1)',
};
const PALETTE = [
  'rgba(99,102,241,0.85)','rgba(6,182,212,0.85)','rgba(245,158,11,0.85)',
  'rgba(239,68,68,0.85)','rgba(34,197,94,0.85)','rgba(168,85,247,0.85)',
  'rgba(249,115,22,0.85)','rgba(20,184,166,0.85)','rgba(236,72,153,0.85)','rgba(234,179,8,0.85)'
];

// ---- tabs ----
function switchTab(name) {
  document.querySelectorAll('.tab').forEach((t,i) => {
    const names = ['overview','orgs','depts','projects','developers'];
    t.classList.toggle('active', names[i] === name);
  });
  document.querySelectorAll('.panel').forEach(p => p.classList.toggle('active', p.id === 'panel-'+name));
  if (name === 'orgs') renderOrgsPanel();
  else if (name === 'depts') renderDeptsPanel();
  else if (name === 'projects') renderProjectsPanel();
  else if (name === 'developers') renderDevsPanel();
}

// ---- load all ----
async function loadAll() {
  document.getElementById('last-updated').textContent = '加载中...';
  try {
    const [global, orgs, depts, projects, devs] = await Promise.all([
      apiFetch('/api/v1/aggregate/summary'),
      apiFetch('/api/v1/aggregate/organizations'),
      apiFetch('/api/v1/aggregate/departments'),
      apiFetch('/api/v1/aggregate/projects'),
      apiFetch('/api/v1/aggregate/developers'),
    ]);
    orgsData = orgs; deptsData = depts; projectsData = projects; devsData = devs;
    renderGlobal(global);
    renderOverview(global, orgs, projects, devs);
    populateOrgFilters(orgs);
    document.getElementById('last-updated').textContent = '更新于 ' + new Date().toLocaleTimeString();
  } catch(e) {
    document.getElementById('last-updated').textContent = '加载失败: ' + e.message;
  }
}

// ---- global stats ----
function renderGlobal(g) {
  document.getElementById('g-ai').textContent = pct(g.weighted_ai_ratio);
  document.getElementById('g-human').textContent = pct(g.weighted_human_ratio);
  document.getElementById('g-projects').textContent = g.total_projects;
  document.getElementById('g-reports').textContent = g.total_reports + ' 条上报记录';
  document.getElementById('g-orgs').textContent = g.total_organizations;
  document.getElementById('g-depts').textContent = g.total_departments + ' 个部门';
  document.getElementById('g-devs').textContent = g.total_developers;
}

// ---- overview charts ----
function renderOverview(g, orgs, projects, devs) {
  // Global pie
  makeChart('chart-global-pie', {
    type: 'doughnut',
    data: {
      labels: ['AI 生成', '人工编写'],
      datasets: [{ data: [g.weighted_ai_ratio, g.weighted_human_ratio], backgroundColor: [COLORS.ai, COLORS.human], borderWidth: 0 }]
    },
    options: { responsive: true, maintainAspectRatio: false, cutout: '65%',
      plugins: { legend: { position: 'bottom', labels: { color: '#94a3b8', padding: 16, font: { size: 13 } } } }
    }
  });
  // Org bar
  const orgLabels = orgs.map(o => o.organization || '未知');
  makeChart('chart-org-bar', {
    type: 'bar',
    data: {
      labels: orgLabels,
      datasets: [
        { label: 'AI 编码率', data: orgs.map(o => +(o.weighted_ai_ratio*100).toFixed(1)), backgroundColor: COLORS.ai },
        { label: '人工编码率', data: orgs.map(o => +(o.weighted_human_ratio*100).toFixed(1)), backgroundColor: COLORS.human },
      ]
    },
    options: { responsive: true, maintainAspectRatio: false, plugins: { legend: { labels: { color: '#94a3b8' } } },
      scales: { x: { ticks: { color: '#94a3b8' }, grid: { color: '#1e293b' } }, y: { ticks: { color: '#94a3b8', callback: v => v+'%' }, grid: { color: '#334155' }, max: 100 } },
      onClick: (_, els) => { if (els.length) { document.getElementById('dept-org-filter').value = orgLabels[els[0].index]; switchTab('depts'); } }
    }
  });
  // Project top10
  const topP = [...projects].sort((a,b) => b.weighted_ai_ratio - a.weighted_ai_ratio).slice(0,10);
  makeChart('chart-project-top', {
    type: 'bar',
    data: { labels: topP.map(p => p.project_name), datasets: [{ label: 'AI 编码率', data: topP.map(p => +(p.weighted_ai_ratio*100).toFixed(1)), backgroundColor: PALETTE }] },
    options: { indexAxis: 'y', responsive: true, maintainAspectRatio: false,
      plugins: { legend: { display: false } },
      scales: { x: { ticks: { color: '#94a3b8', callback: v => v+'%' }, grid: { color: '#334155' }, max: 100 }, y: { ticks: { color: '#94a3b8' }, grid: { color: '#1e293b' } } }
    }
  });
  // Dev top10
  const topD = [...devs].sort((a,b) => b.weighted_ai_ratio - a.weighted_ai_ratio).slice(0,10);
  makeChart('chart-dev-top', {
    type: 'bar',
    data: { labels: topD.map(d => d.name || d.email), datasets: [{ label: 'AI 编码率', data: topD.map(d => +(d.weighted_ai_ratio*100).toFixed(1)), backgroundColor: PALETTE }] },
    options: { indexAxis: 'y', responsive: true, maintainAspectRatio: false,
      plugins: { legend: { display: false } },
      scales: { x: { ticks: { color: '#94a3b8', callback: v => v+'%' }, grid: { color: '#334155' }, max: 100 }, y: { ticks: { color: '#94a3b8' }, grid: { color: '#1e293b' } } }
    }
  });
}

// ---- orgs panel ----
function renderOrgsPanel() {
  const data = orgsData;
  // bar chart
  makeChart('chart-orgs-bar', {
    type: 'bar',
    data: { labels: data.map(o=>o.organization||'未知'), datasets: [
      { label: 'AI%', data: data.map(o=>+(o.weighted_ai_ratio*100).toFixed(1)), backgroundColor: COLORS.ai },
      { label: '人工%', data: data.map(o=>+(o.weighted_human_ratio*100).toFixed(1)), backgroundColor: COLORS.human },
    ]},
    options: { responsive: true, maintainAspectRatio: false, plugins: { legend: { labels: { color:'#94a3b8' } } },
      scales: { x:{ticks:{color:'#94a3b8'},grid:{color:'#1e293b'}}, y:{ticks:{color:'#94a3b8',callback:v=>v+'%'},grid:{color:'#334155'},max:100} },
      onClick:(_,els)=>{ if(els.length){ document.getElementById('dept-org-filter').value=data[els[0].index].organization||''; switchTab('depts'); } }
    }
  });
  // table
  if (!data.length) { document.getElementById('orgs-table-wrap').innerHTML = '<div class="empty"><div class="empty-icon">📊</div>暂无组织数据</div>'; return; }
  let html = '<table><thead><tr><th>组织名称</th><th>项目数</th><th>开发者</th><th>总提交数</th><th>AI 编码率</th><th>比例分布</th></tr></thead><tbody>';
  data.forEach(o => {
    html += `<tr class="clickable" onclick="document.getElementById('dept-org-filter').value='${escHtml(o.organization||'')}';switchTab('depts')">
      <td><b>${escHtml(o.organization||'未知')}</b></td>
      <td>${o.project_count}</td><td>${o.developer_count}</td><td>${o.total_commits.toLocaleString()}</td>
      <td>${aiTag(o.weighted_ai_ratio)}</td>
      <td>${ratioBar(o.weighted_ai_ratio, o.weighted_human_ratio)}</td>
    </tr>`;
  });
  html += '</tbody></table>';
  document.getElementById('orgs-table-wrap').innerHTML = html;
}

// ---- depts panel ----
async function loadDepts() {
  const org = document.getElementById('dept-org-filter').value;
  const url = org ? '/api/v1/aggregate/departments?org=' + encodeURIComponent(org) : '/api/v1/aggregate/departments';
  document.getElementById('depts-table-wrap').innerHTML = '<div class="loading"><div class="spinner"></div>加载中...</div>';
  try {
    deptsData = await apiFetch(url);
    renderDeptsPanel();
  } catch(e) {}
}
function renderDeptsPanel() {
  const data = deptsData;
  const org = document.getElementById('dept-org-filter').value;
  // breadcrumb
  let bc = org ? `<span class="crumb" onclick="document.getElementById('dept-org-filter').value='';loadDepts()">所有组织</span><span class="sep">/</span><span class="current">${escHtml(org)}</span>`
    : '<span class="current">所有部门</span>';
  document.getElementById('bc-depts').innerHTML = bc;
  // bar
  makeChart('chart-depts-bar', {
    type: 'bar',
    data: { labels: data.map(d=>(d.department||'未知')), datasets: [
      { label: 'AI%', data: data.map(d=>+(d.weighted_ai_ratio*100).toFixed(1)), backgroundColor: COLORS.ai },
      { label: '人工%', data: data.map(d=>+(d.weighted_human_ratio*100).toFixed(1)), backgroundColor: COLORS.human },
    ]},
    options: { responsive: true, maintainAspectRatio: false, plugins: { legend: { labels: { color:'#94a3b8' } } },
      scales: { x:{ticks:{color:'#94a3b8'},grid:{color:'#1e293b'}}, y:{ticks:{color:'#94a3b8',callback:v=>v+'%'},grid:{color:'#334155'},max:100} }
    }
  });
  // table
  if (!data.length) { document.getElementById('depts-table-wrap').innerHTML = '<div class="empty"><div class="empty-icon">🏢</div>暂无部门数据</div>'; return; }
  let html = '<table><thead><tr><th>组织</th><th>部门</th><th>项目数</th><th>开发者</th><th>总提交数</th><th>AI 编码率</th><th>比例分布</th></tr></thead><tbody>';
  data.forEach(d => {
    html += `<tr>
      <td>${escHtml(d.organization||'未知')}</td><td><b>${escHtml(d.department||'未知')}</b></td>
      <td>${d.project_count}</td><td>${d.developer_count}</td><td>${d.total_commits.toLocaleString()}</td>
      <td>${aiTag(d.weighted_ai_ratio)}</td>
      <td>${ratioBar(d.weighted_ai_ratio, d.weighted_human_ratio)}</td>
    </tr>`;
  });
  html += '</tbody></table>';
  document.getElementById('depts-table-wrap').innerHTML = html;
}

// ---- projects panel ----
function renderProjectsPanel() { loadProjects(); }
async function loadProjects() {
  const org = document.getElementById('proj-org-filter').value;
  const dept = document.getElementById('proj-dept-filter').value;
  let url = '/api/v1/aggregate/projects';
  const params = [];
  if (org) params.push('org='+encodeURIComponent(org));
  if (dept) params.push('dept='+encodeURIComponent(dept));
  if (params.length) url += '?' + params.join('&');
  document.getElementById('projects-table-wrap').innerHTML = '<div class="loading"><div class="spinner"></div>加载中...</div>';
  try {
    projectsData = await apiFetch(url);
    renderProjectsTable(projectsData);
  } catch(e) {}
}
function renderProjectsTable(data) {
  makeChart('chart-projects-bar', {
    type: 'bar',
    data: { labels: data.map(p=>p.project_name), datasets: [
      { label: 'AI%', data: data.map(p=>+(p.weighted_ai_ratio*100).toFixed(1)), backgroundColor: COLORS.ai },
      { label: '人工%', data: data.map(p=>+(p.weighted_human_ratio*100).toFixed(1)), backgroundColor: COLORS.human },
    ]},
    options: { responsive: true, maintainAspectRatio: false, plugins: { legend: { labels: { color:'#94a3b8' } } },
      scales: { x:{ticks:{color:'#94a3b8'},grid:{color:'#1e293b'}}, y:{ticks:{color:'#94a3b8',callback:v=>v+'%'},grid:{color:'#334155'},max:100} }
    }
  });
  if (!data.length) { document.getElementById('projects-table-wrap').innerHTML = '<div class="empty"><div class="empty-icon">📁</div>暂无项目数据</div>'; return; }
  let html = '<table><thead><tr><th>项目名称</th><th>组织</th><th>部门</th><th>分支</th><th>上报次数</th><th>总提交数</th><th>AI 编码率</th><th>比例分布</th></tr></thead><tbody>';
  data.forEach(p => {
    html += `<tr>
      <td><b>${escHtml(p.project_name)}</b></td>
      <td>${escHtml(p.organization||'-')}</td><td>${escHtml(p.department||'-')}</td>
      <td>${escHtml(p.branch||'-')}</td><td>${p.report_count}</td><td>${p.total_commits.toLocaleString()}</td>
      <td>${aiTag(p.weighted_ai_ratio)}</td>
      <td>${ratioBar(p.weighted_ai_ratio, p.weighted_human_ratio)}</td>
    </tr>`;
  });
  html += '</tbody></table>';
  document.getElementById('projects-table-wrap').innerHTML = html;
}

// ---- developers panel ----
function renderDevsPanel() { loadDevelopers(); }
async function loadDevelopers() {
  const org = document.getElementById('dev-org-filter').value;
  const dept = document.getElementById('dev-dept-filter').value;
  let url = '/api/v1/aggregate/developers';
  const params = [];
  if (org) params.push('org='+encodeURIComponent(org));
  if (dept) params.push('dept='+encodeURIComponent(dept));
  if (params.length) url += '?' + params.join('&');
  document.getElementById('devs-table-wrap').innerHTML = '<div class="loading"><div class="spinner"></div>加载中...</div>';
  try {
    devsData = await apiFetch(url);
    renderDevsTable(devsData);
  } catch(e) {}
}
function renderDevsTable(data) {
  makeChart('chart-devs-bar', {
    type: 'bar',
    data: { labels: data.map(d=>d.name||d.email), datasets: [
      { label: 'AI 代码行', data: data.map(d=>d.total_ai_additions), backgroundColor: COLORS.ai },
      { label: '人工代码行', data: data.map(d=>d.total_human_additions), backgroundColor: COLORS.human },
    ]},
    options: { responsive: true, maintainAspectRatio: false, plugins: { legend: { labels: { color:'#94a3b8' } } },
      scales: { x:{ticks:{color:'#94a3b8'},grid:{color:'#1e293b'}}, y:{ticks:{color:'#94a3b8'},grid:{color:'#334155'}} }
    }
  });
  if (!data.length) { document.getElementById('devs-table-wrap').innerHTML = '<div class="empty"><div class="empty-icon">👨‍💻</div>暂无开发者数据</div>'; return; }
  let html = '<table><thead><tr><th>姓名</th><th>邮箱</th><th>组织/部门</th><th>参与项目</th><th>总提交</th><th>AI 代码行</th><th>人工代码行</th><th>AI 编码率</th><th>比例分布</th></tr></thead><tbody>';
  data.forEach(d => {
    const orgDept = [d.organization, d.department].filter(Boolean).join(' / ') || '-';
    html += `<tr>
      <td><b>${escHtml(d.name||'-')}</b></td><td>${escHtml(d.email)}</td>
      <td>${escHtml(orgDept)}</td><td>${d.project_count}</td><td>${d.total_commits.toLocaleString()}</td>
      <td>${d.total_ai_additions.toLocaleString()}</td><td>${d.total_human_additions.toLocaleString()}</td>
      <td>${aiTag(d.weighted_ai_ratio)}</td>
      <td>${ratioBar(d.weighted_ai_ratio, d.weighted_human_ratio)}</td>
    </tr>`;
  });
  html += '</tbody></table>';
  document.getElementById('devs-table-wrap').innerHTML = html;
}

// ---- filter helpers ----
function populateOrgFilters(orgs) {
  const selectors = ['dept-org-filter','proj-org-filter','dev-org-filter'];
  selectors.forEach(id => {
    const sel = document.getElementById(id);
    const cur = sel.value;
    while (sel.options.length > 1) sel.remove(1);
    orgs.forEach(o => {
      const opt = new Option(o.organization||'未知', o.organization||'');
      sel.add(opt);
    });
    sel.value = cur;
  });
}
function populateDeptFilter(selId, org) {
  const relevant = org ? deptsData.filter(d => d.organization === org) : deptsData;
  const sel = document.getElementById(selId);
  while (sel.options.length > 1) sel.remove(1);
  const seen = new Set();
  relevant.forEach(d => {
    if (!seen.has(d.department)) { seen.add(d.department); sel.add(new Option(d.department||'未知', d.department||'')); }
  });
}
function onProjOrgChange() { populateDeptFilter('proj-dept-filter', document.getElementById('proj-org-filter').value); loadProjects(); }
function onDevOrgChange() { populateDeptFilter('dev-dept-filter', document.getElementById('dev-org-filter').value); loadDevelopers(); }
function clearProjFilters() { document.getElementById('proj-org-filter').value=''; document.getElementById('proj-dept-filter').value=''; loadProjects(); }
function clearDevFilters() { document.getElementById('dev-org-filter').value=''; document.getElementById('dev-dept-filter').value=''; loadDevelopers(); }

function escHtml(s) { return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;'); }

// ---- init ----
document.getElementById('server-addr').textContent = location.host;
loadAll();
</script>
</body>
</html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorship::stats::CommitStats;
    use crate::report::model::{
        ReportCommit, ReportRangeInfo, ReportRangeMode, ReportRatios, ReportRepoInfo,
    };
    use std::collections::BTreeMap;

    fn sample_report() -> ReportDocument {
        ReportDocument {
            schema_version: REPORT_SCHEMA_VERSION.to_string(),
            generated_at: "2026-05-18T00:00:00Z".to_string(),
            tool_version: "test".to_string(),
            repo: ReportRepoInfo {
                workdir: None,
                remote_url_hash: Some("sha256:test".to_string()),
                branch: Some("main".to_string()),
                head_commit: Some("abc".to_string()),
            },
            range: ReportRangeInfo {
                mode: ReportRangeMode::Head,
                from: None,
                to: Some("abc".to_string()),
                since: None,
                until: None,
                commit_count: 1,
                commits_with_authorship: 1,
                commits_without_authorship: 0,
            },
            summary: ReportSummary {
                git_diff_added_lines: 3,
                ai_additions: 2,
                human_additions: 1,
                ..Default::default()
            },
            ratios: ReportRatios {
                ai: 0.67,
                human: 0.33,
                mixed: 0.0,
                unknown: 0.0,
            },
            tool_model_breakdown: BTreeMap::new(),
            commits: vec![ReportCommit {
                sha: "abc".to_string(),
                author: "Test <test@example.com>".to_string(),
                author_time: "2026-05-18T00:00:00Z".to_string(),
                subject: "test".to_string(),
                has_authorship_note: true,
                stats: CommitStats {
                    git_diff_added_lines: 3,
                    ai_additions: 2,
                    human_additions: 1,
                    ..Default::default()
                },
            }],
        }
    }

    #[test]
    fn ingest_deduplicates_commit_stats() {
        let mut store = ReportStore::in_memory().unwrap();
        let report = sample_report();

        let first = store.ingest_report(&report).unwrap();
        let second = store.ingest_report(&report).unwrap();

        assert_eq!(first.inserted_commits, 1);
        assert_eq!(first.duplicate_commits, 0);
        assert_eq!(second.inserted_commits, 0);
        assert_eq!(second.duplicate_commits, 1);

        let summary = store.project_summary(first.project_id).unwrap();
        assert_eq!(summary.commit_count, 1);
        assert_eq!(summary.summary.ai_additions, 2);
        assert_eq!(summary.summary.human_additions, 1);
    }

    #[test]
    fn ingest_rejects_wrong_schema_version() {
        let mut store = ReportStore::in_memory().unwrap();
        let mut report = sample_report();
        report.schema_version = "wrong".to_string();

        let error = store.ingest_report(&report).unwrap_err().to_string();
        assert!(error.contains("Unsupported report schema"));
    }

    #[test]
    fn http_api_ingests_and_reads_project_summary() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let db_path = temp.path().join("report-server.sqlite");
        let report_body = serde_json::to_string(&sample_report()).unwrap();
        let post_request = format!(
            "POST /api/v1/reports HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            report_body.len(),
            report_body
        );

        let post_response = handle_http_request(post_request.as_bytes(), &db_path);
        let post_text = String::from_utf8(post_response).unwrap();
        assert!(post_text.starts_with("HTTP/1.1 201 Created"));
        assert!(post_text.contains("\"inserted_commits\": 1"));

        let summary_request = "GET /api/v1/projects/1/summary HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let summary_response = handle_http_request(summary_request.as_bytes(), &db_path);
        let summary_text = String::from_utf8(summary_response).unwrap();
        assert!(summary_text.starts_with("HTTP/1.1 200 OK"));
        assert!(summary_text.contains("\"commit_count\": 1"));
        assert!(summary_text.contains("\"ai_additions\": 2"));

        let commits_request = "GET /api/v1/projects/1/commits HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let commits_response = handle_http_request(commits_request.as_bytes(), &db_path);
        let commits_text = String::from_utf8(commits_response).unwrap();
        assert!(commits_text.starts_with("HTTP/1.1 200 OK"));
        assert!(commits_text.contains("\"sha\": \"abc\""));
    }

    fn sample_project_summary() -> ProjectSummaryReport {
        ProjectSummaryReport {
            project_name: "my-project".to_string(),
            git_url: Some("https://github.com/org/my-project.git".to_string()),
            branch: Some("main".to_string()),
            total_commits: 10,
            developers: vec![
                DeveloperSummary {
                    name: "Alice".to_string(),
                    email: "alice@example.com".to_string(),
                    commits: 6,
                    added_lines: 500,
                    ai_additions: 200,
                    human_additions: 300,
                    ai_ratio: 0.4,
                    human_ratio: 0.6,
                },
                DeveloperSummary {
                    name: "Bob".to_string(),
                    email: "bob@example.com".to_string(),
                    commits: 4,
                    added_lines: 300,
                    ai_additions: 150,
                    human_additions: 150,
                    ai_ratio: 0.5,
                    human_ratio: 0.5,
                },
            ],
            project_ratios: ProjectRatios {
                ai: 0.4375,
                human: 0.5625,
            },
            organization: None,
            department: None,
            reporter_name: None,
            reporter_email: None,
            report_period: None,
        }
    }

    #[test]
    fn ingest_summary_stores_project_and_developers() {
        let mut store = ReportStore::in_memory().unwrap();
        let summary = sample_project_summary();

        let response = store.ingest_summary(&summary).unwrap();
        assert_eq!(response.project_name, "my-project");
        assert_eq!(response.developer_count, 2);
        assert_eq!(response.organization, None);

        // Retrieve detail
        let detail = store.get_project_summary_detail(response.summary_id).unwrap();
        assert_eq!(detail.project_name, "my-project");
        assert_eq!(detail.total_commits, 10);
        assert_eq!(detail.developers.len(), 2);
        assert_eq!(detail.developers[0].name, "Alice");
        assert_eq!(detail.developers[1].email, "bob@example.com");
    }

    #[test]
    fn ingest_summary_upserts_on_same_reporter_same_project() {
        let mut store = ReportStore::in_memory().unwrap();
        let mut summary = sample_project_summary();
        summary.reporter_email = Some("alice@example.com".to_string());

        let first = store.ingest_summary(&summary).unwrap();
        // 同一上报人重复上传 → 更新而非新增
        summary.total_commits = 20;
        summary.project_ratios.ai = 0.6;
        let second = store.ingest_summary(&summary).unwrap();

        assert_eq!(first.summary_id, second.summary_id);

        let detail = store.get_project_summary_detail(second.summary_id).unwrap();
        assert_eq!(detail.total_commits, 20);
    }

    #[test]
    fn ingest_summary_different_reporters_do_not_overwrite_each_other() {
        let mut store = ReportStore::in_memory().unwrap();

        // 上报人 A 上传
        let mut summary_a = sample_project_summary();
        summary_a.reporter_email = Some("alice@example.com".to_string());
        summary_a.total_commits = 10;
        let resp_a = store.ingest_summary(&summary_a).unwrap();

        // 上报人 B 上传同项目
        let mut summary_b = sample_project_summary();
        summary_b.reporter_email = Some("bob@example.com".to_string());
        summary_b.total_commits = 15;
        let resp_b = store.ingest_summary(&summary_b).unwrap();

        // 两者 ID 不同，互不覆盖
        assert_ne!(resp_a.summary_id, resp_b.summary_id);

        let list = store.list_project_summaries().unwrap();
        assert_eq!(list.len(), 2);

        let detail_a = store.get_project_summary_detail(resp_a.summary_id).unwrap();
        let detail_b = store.get_project_summary_detail(resp_b.summary_id).unwrap();
        assert_eq!(detail_a.total_commits, 10);
        assert_eq!(detail_b.total_commits, 15);
    }

    #[test]
    fn ingest_summary_with_org_and_department() {
        let mut store = ReportStore::in_memory().unwrap();
        let mut summary = sample_project_summary();
        summary.organization = Some("ACME Corp".to_string());
        summary.department = Some("研发部".to_string());
        summary.reporter_email = Some("dev@acme.com".to_string());

        let response = store.ingest_summary(&summary).unwrap();
        assert_eq!(response.organization.as_deref(), Some("ACME Corp"));
        assert_eq!(response.department.as_deref(), Some("研发部"));

        let detail = store.get_project_summary_detail(response.summary_id).unwrap();
        assert_eq!(detail.organization.as_deref(), Some("ACME Corp"));
        assert_eq!(detail.department.as_deref(), Some("研发部"));
    }

    #[test]
    fn aggregate_global_returns_weighted_ratios() {
        let mut store = ReportStore::in_memory().unwrap();

        // 项目 A：10 commits，AI 比率 0.4
        let mut s1 = sample_project_summary();
        s1.reporter_email = Some("a@test.com".to_string());
        s1.total_commits = 10;
        s1.project_ratios.ai = 0.4;
        s1.project_ratios.human = 0.6;
        store.ingest_summary(&s1).unwrap();

        // 项目 B：40 commits，AI 比率 0.6
        let mut s2 = sample_project_summary();
        s2.project_name = "project-b".to_string();
        s2.reporter_email = Some("b@test.com".to_string());
        s2.total_commits = 40;
        s2.project_ratios.ai = 0.6;
        s2.project_ratios.human = 0.4;
        store.ingest_summary(&s2).unwrap();

        let global = store.aggregate_global().unwrap();
        assert_eq!(global.total_reports, 2);
        assert_eq!(global.total_projects, 2);
        // 加权 AI 比率 = (10*0.4 + 40*0.6) / 50 = (4+24)/50 = 0.56
        let expected_ai = (10.0 * 0.4 + 40.0 * 0.6) / 50.0;
        assert!((global.weighted_ai_ratio - expected_ai).abs() < 1e-9);
    }

    #[test]
    fn aggregate_by_org_groups_correctly() {
        let mut store = ReportStore::in_memory().unwrap();

        let mut s1 = sample_project_summary();
        s1.organization = Some("OrgA".to_string());
        s1.department = Some("Dept1".to_string());
        s1.reporter_email = Some("u1@org.com".to_string());
        s1.total_commits = 20;
        s1.project_ratios.ai = 0.5;
        s1.project_ratios.human = 0.5;
        store.ingest_summary(&s1).unwrap();

        let mut s2 = sample_project_summary();
        s2.organization = Some("OrgB".to_string());
        s2.department = Some("Dept1".to_string());
        s2.reporter_email = Some("u2@org.com".to_string());
        s2.project_name = "other-project".to_string();
        s2.total_commits = 10;
        s2.project_ratios.ai = 0.8;
        s2.project_ratios.human = 0.2;
        store.ingest_summary(&s2).unwrap();

        let orgs = store.aggregate_by_org().unwrap();
        assert_eq!(orgs.len(), 2);
        let org_a = orgs.iter().find(|o| o.organization == "OrgA").unwrap();
        let org_b = orgs.iter().find(|o| o.organization == "OrgB").unwrap();
        assert_eq!(org_a.project_count, 1);
        assert!((org_a.weighted_ai_ratio - 0.5).abs() < 1e-9);
        assert!((org_b.weighted_ai_ratio - 0.8).abs() < 1e-9);
    }

    #[test]
    fn aggregate_by_developer_crosses_projects() {
        let mut store = ReportStore::in_memory().unwrap();

        // 同一开发者 alice 在两个项目上都有贡献
        let make_summary = |proj: &str, reporter: &str, ai: u32, human: u32| ProjectSummaryReport {
            project_name: proj.to_string(),
            git_url: None,
            branch: None,
            total_commits: 10,
            developers: vec![DeveloperSummary {
                name: "Alice".to_string(),
                email: "alice@example.com".to_string(),
                commits: 10,
                added_lines: (ai + human),
                ai_additions: ai,
                human_additions: human,
                ai_ratio: ai as f64 / (ai + human) as f64,
                human_ratio: human as f64 / (ai + human) as f64,
            }],
            project_ratios: ProjectRatios {
                ai: ai as f64 / (ai + human) as f64,
                human: human as f64 / (ai + human) as f64,
            },
            organization: None,
            department: None,
            reporter_name: None,
            reporter_email: Some(reporter.to_string()),
            report_period: None,
        };

        store.ingest_summary(&make_summary("proj-1", "alice@example.com", 100, 100)).unwrap();
        store.ingest_summary(&make_summary("proj-2", "alice@example.com", 300, 100)).unwrap();

        let devs = store.aggregate_by_developer(None, None).unwrap();
        let alice = devs.iter().find(|d| d.email == "alice@example.com").unwrap();
        assert_eq!(alice.project_count, 2);
        assert_eq!(alice.total_ai_additions, 400);
        assert_eq!(alice.total_human_additions, 200);
        // 加权比率 = 400 / 600
        assert!((alice.weighted_ai_ratio - 400.0 / 600.0).abs() < 1e-9);
    }

    #[test]
    fn list_project_summaries_returns_all() {
        let mut store = ReportStore::in_memory().unwrap();
        let summary1 = sample_project_summary();
        let mut summary2 = sample_project_summary();
        summary2.project_name = "other-project".to_string();
        summary2.reporter_email = Some("other@example.com".to_string());

        store.ingest_summary(&summary1).unwrap();
        store.ingest_summary(&summary2).unwrap();

        let list = store.list_project_summaries().unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn http_api_ingests_and_reads_summary() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let db_path = temp.path().join("summary-server.sqlite");
        let summary_body = serde_json::to_string(&sample_project_summary()).unwrap();

        // POST summary
        let post_request = format!(
            "POST /api/v1/summaries HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            summary_body.len(),
            summary_body
        );
        let post_response = handle_http_request(post_request.as_bytes(), &db_path);
        let post_text = String::from_utf8(post_response).unwrap();
        assert!(post_text.starts_with("HTTP/1.1 201 Created"));
        assert!(post_text.contains("\"project_name\": \"my-project\""));
        assert!(post_text.contains("\"developer_count\": 2"));

        // GET list
        let list_request = "GET /api/v1/summaries HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let list_response = handle_http_request(list_request.as_bytes(), &db_path);
        let list_text = String::from_utf8(list_response).unwrap();
        assert!(list_text.starts_with("HTTP/1.1 200 OK"));
        assert!(list_text.contains("\"project_name\": \"my-project\""));

        // GET detail
        let detail_request = "GET /api/v1/summaries/1 HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let detail_response = handle_http_request(detail_request.as_bytes(), &db_path);
        let detail_text = String::from_utf8(detail_response).unwrap();
        assert!(detail_text.starts_with("HTTP/1.1 200 OK"));
        assert!(detail_text.contains("\"Alice\""));
        assert!(detail_text.contains("\"bob@example.com\""));
    }

    #[test]
    fn http_api_aggregate_summary_returns_global_stats() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let db_path = temp.path().join("agg-server.sqlite");

        // 先上传两条摘要
        for (reporter, proj, ai, human) in [
            ("a@t.com", "proj-a", 0.4f64, 0.6f64),
            ("b@t.com", "proj-b", 0.8f64, 0.2f64),
        ] {
            let mut s = sample_project_summary();
            s.reporter_email = Some(reporter.to_string());
            s.project_name = proj.to_string();
            s.total_commits = 10;
            s.project_ratios.ai = ai;
            s.project_ratios.human = human;
            let body = serde_json::to_string(&s).unwrap();
            let req = format!(
                "POST /api/v1/summaries HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(), body
            );
            handle_http_request(req.as_bytes(), &db_path);
        }

        let req = "GET /api/v1/aggregate/summary HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let resp = String::from_utf8(handle_http_request(req.as_bytes(), &db_path)).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("\"total_reports\": 2"));
        assert!(resp.contains("\"total_projects\": 2"));
    }

    #[test]
    fn parse_query_param_extracts_value() {
        assert_eq!(parse_query_param("/api/v1/aggregate/departments?org=ACME", "org").as_deref(), Some("ACME"));
        assert_eq!(parse_query_param("/api/v1/aggregate/departments?org=ACME+Corp&x=1", "org").as_deref(), Some("ACME Corp"));
        assert_eq!(parse_query_param("/api/v1/aggregate/departments", "org"), None);
        assert_eq!(parse_query_param("/api/v1/aggregate/departments?dept=rd", "org"), None);
    }
}
