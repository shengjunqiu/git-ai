use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientStatus {
    LoggedIn,
    LoggedOut,
}

impl ClientStatus {
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "logged_in" => Ok(Self::LoggedIn),
            "logged_out" => Ok(Self::LoggedOut),
            _ => Err(AppError::BadRequest(
                "status must be logged_in or logged_out".into(),
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LoggedIn => "logged_in",
            Self::LoggedOut => "logged_out",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DeveloperClientStatus {
    pub status: String,
    pub last_status_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub cli_version: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub hostname: Option<String>,
}

pub struct ClientStatusMetadata {
    pub distinct_id: Option<String>,
    pub cli_version: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub hostname: Option<String>,
}

pub async fn record_status(
    pool: &PgPool,
    user_id: Uuid,
    org_id: Option<Uuid>,
    status: ClientStatus,
    metadata: ClientStatusMetadata,
) -> Result<(), AppError> {
    sqlx::query(
        r#"INSERT INTO developer_client_status (
            user_id, org_id, distinct_id, status, last_status_at, last_seen_at,
            cli_version, os, arch, hostname, updated_at
        ) VALUES (
            $1, $2, $3, $4, now(),
            CASE WHEN $4 = 'logged_in' THEN now() ELSE NULL END,
            $5, $6, $7, $8, now()
        )
        ON CONFLICT (user_id) DO UPDATE SET
            org_id = COALESCE(EXCLUDED.org_id, developer_client_status.org_id),
            distinct_id = COALESCE(EXCLUDED.distinct_id, developer_client_status.distinct_id),
            status = EXCLUDED.status,
            last_status_at = now(),
            last_seen_at = CASE
                WHEN EXCLUDED.status = 'logged_in' THEN now()
                ELSE developer_client_status.last_seen_at
            END,
            cli_version = COALESCE(EXCLUDED.cli_version, developer_client_status.cli_version),
            os = COALESCE(EXCLUDED.os, developer_client_status.os),
            arch = COALESCE(EXCLUDED.arch, developer_client_status.arch),
            hostname = COALESCE(EXCLUDED.hostname, developer_client_status.hostname),
            updated_at = now()"#,
    )
    .bind(user_id)
    .bind(org_id)
    .bind(metadata.distinct_id)
    .bind(status.as_str())
    .bind(metadata.cli_version)
    .bind(metadata.os)
    .bind(metadata.arch)
    .bind(metadata.hostname)
    .execute(pool)
    .await
    .map_err(AppError::Database)?;

    Ok(())
}

pub async fn touch_last_seen(
    pool: &PgPool,
    user_id: Uuid,
    org_id: Option<Uuid>,
    distinct_id: Option<String>,
) -> Result<(), AppError> {
    sqlx::query(
        r#"INSERT INTO developer_client_status (
            user_id, org_id, distinct_id, status, last_status_at, last_seen_at, updated_at
        ) VALUES ($1, $2, $3, 'logged_in', now(), now(), now())
        ON CONFLICT (user_id) DO UPDATE SET
            org_id = COALESCE(EXCLUDED.org_id, developer_client_status.org_id),
            distinct_id = COALESCE(EXCLUDED.distinct_id, developer_client_status.distinct_id),
            status = 'logged_in',
            last_status_at = CASE
                WHEN developer_client_status.status <> 'logged_in' THEN now()
                ELSE developer_client_status.last_status_at
            END,
            last_seen_at = now(),
            updated_at = now()"#,
    )
    .bind(user_id)
    .bind(org_id)
    .bind(distinct_id)
    .execute(pool)
    .await
    .map_err(AppError::Database)?;

    Ok(())
}

pub async fn get_status(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Option<DeveloperClientStatus>, AppError> {
    let row: Option<(
        String,
        DateTime<Utc>,
        Option<DateTime<Utc>>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        r#"SELECT status, last_status_at, last_seen_at, cli_version, os, arch, hostname
           FROM developer_client_status
           WHERE user_id = $1"#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(AppError::Database)?;

    Ok(row.map(
        |(status, last_status_at, last_seen_at, cli_version, os, arch, hostname)| {
            DeveloperClientStatus {
                status,
                last_status_at,
                last_seen_at,
                cli_version,
                os,
                arch,
                hostname,
            }
        },
    ))
}
