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
    pub device_key: String,
    pub status: String,
    pub last_status_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub cli_version: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub hostname: Option<String>,
    pub device_count: usize,
    pub devices: Vec<DeveloperClientDeviceStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeveloperClientDeviceStatus {
    pub device_key: String,
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
    let device_key = device_key_from_metadata(&metadata);

    sqlx::query(
        r#"INSERT INTO developer_client_status (
            user_id, device_key, org_id, distinct_id, status, last_status_at, last_seen_at,
            cli_version, os, arch, hostname, updated_at
        ) VALUES (
            $1, $2, $3, $4, $5, now(),
            CASE WHEN $5 = 'logged_in' THEN now() ELSE NULL END,
            $6, $7, $8, $9, now()
        )
        ON CONFLICT (user_id, device_key) DO UPDATE SET
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
    .bind(device_key)
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
    let device_key = device_key_from_parts(distinct_id.as_deref(), None);

    sqlx::query(
        r#"INSERT INTO developer_client_status (
            user_id, device_key, org_id, distinct_id, status, last_status_at, last_seen_at, updated_at
        ) VALUES ($1, $2, $3, $4, 'logged_in', now(), now(), now())
        ON CONFLICT (user_id, device_key) DO UPDATE SET
            org_id = COALESCE(EXCLUDED.org_id, developer_client_status.org_id),
            distinct_id = COALESCE(EXCLUDED.distinct_id, developer_client_status.distinct_id),
            status = 'logged_in',
            last_status_at = CASE
                WHEN developer_client_status.status <> 'logged_in' THEN now()
                ELSE developer_client_status.last_status_at
            END,
            last_seen_at = CASE
                WHEN developer_client_status.status <> 'logged_in'
                    OR developer_client_status.last_seen_at IS NULL
                    OR developer_client_status.last_seen_at < now() - interval '60 seconds'
                THEN now()
                ELSE developer_client_status.last_seen_at
            END,
            updated_at = now()
        WHERE developer_client_status.status <> 'logged_in'
            OR developer_client_status.last_seen_at IS NULL
            OR developer_client_status.last_seen_at < now() - interval '60 seconds'
            OR (
                EXCLUDED.org_id IS NOT NULL
                AND developer_client_status.org_id IS DISTINCT FROM EXCLUDED.org_id
            )
            OR (
                EXCLUDED.distinct_id IS NOT NULL
                AND developer_client_status.distinct_id IS DISTINCT FROM EXCLUDED.distinct_id
            )"#,
    )
    .bind(user_id)
    .bind(device_key)
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
    let rows: Vec<(
        String,
        String,
        DateTime<Utc>,
        Option<DateTime<Utc>>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        r#"SELECT device_key, status, last_status_at, last_seen_at, cli_version, os, arch, hostname
           FROM developer_client_status
           WHERE user_id = $1
           ORDER BY (status = 'logged_in') DESC, COALESCE(last_seen_at, last_status_at) DESC"#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(AppError::Database)?;

    if rows.is_empty() {
        return Ok(None);
    }

    let devices: Vec<DeveloperClientDeviceStatus> = rows
        .into_iter()
        .map(
            |(
                device_key,
                status,
                last_status_at,
                last_seen_at,
                cli_version,
                os,
                arch,
                hostname,
            )| {
                DeveloperClientDeviceStatus {
                    device_key,
                    status,
                    last_status_at,
                    last_seen_at,
                    cli_version,
                    os,
                    arch,
                    hostname,
                }
            },
        )
        .collect();
    let summary = devices[0].clone();
    let aggregate_status = if devices.iter().any(|device| device.status == "logged_in") {
        "logged_in"
    } else {
        "logged_out"
    };
    let last_status_at = devices
        .iter()
        .map(|device| device.last_status_at)
        .max()
        .unwrap_or(summary.last_status_at);
    let last_seen_at = devices
        .iter()
        .filter_map(|device| device.last_seen_at)
        .max();

    Ok(Some(DeveloperClientStatus {
        device_key: summary.device_key,
        status: aggregate_status.to_string(),
        last_status_at,
        last_seen_at,
        cli_version: summary.cli_version,
        os: summary.os,
        arch: summary.arch,
        hostname: summary.hostname,
        device_count: devices.len(),
        devices,
    }))
}

fn device_key_from_metadata(metadata: &ClientStatusMetadata) -> String {
    device_key_from_parts(
        metadata.distinct_id.as_deref(),
        metadata.hostname.as_deref(),
    )
}

fn device_key_from_parts(distinct_id: Option<&str>, hostname: Option<&str>) -> String {
    first_non_empty(distinct_id)
        .or_else(|| first_non_empty(hostname))
        .unwrap_or("unknown")
        .to_string()
}

fn first_non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use uuid::Uuid;

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
                        "skipping client status test: could not connect to admin database: {error}"
                    );
                    return Ok(None);
                }
            };

            if let Err(error) = create_database(&admin_pool, &db_name).await {
                eprintln!(
                    "skipping client status test: could not create isolated database {db_name}: {error}"
                );
                return Ok(None);
            }

            let pool = PgPoolOptions::new()
                .max_connections(6)
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn records_multiple_devices_for_same_user() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.pool).await?;

        record_status(
            &db.pool,
            user_id,
            Some(org_id),
            ClientStatus::LoggedIn,
            metadata("device-a", "host-a"),
        )
        .await?;
        record_status(
            &db.pool,
            user_id,
            Some(org_id),
            ClientStatus::LoggedIn,
            metadata("device-b", "host-b"),
        )
        .await?;

        assert_eq!(table_count(&db.pool, "developer_client_status").await?, 2);

        let status = get_status(&db.pool, user_id).await?.expect("status row");
        assert_eq!(status.status, "logged_in");
        assert_eq!(status.device_count, 2);
        assert!(
            status
                .devices
                .iter()
                .any(|device| device.device_key == "device-a")
        );
        assert!(
            status
                .devices
                .iter()
                .any(|device| device.device_key == "device-b")
        );

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn logout_one_device_does_not_logout_other_devices() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.pool).await?;

        record_status(
            &db.pool,
            user_id,
            Some(org_id),
            ClientStatus::LoggedIn,
            metadata("device-a", "host-a"),
        )
        .await?;
        record_status(
            &db.pool,
            user_id,
            Some(org_id),
            ClientStatus::LoggedIn,
            metadata("device-b", "host-b"),
        )
        .await?;
        record_status(
            &db.pool,
            user_id,
            Some(org_id),
            ClientStatus::LoggedOut,
            metadata("device-a", "host-a"),
        )
        .await?;

        let status = get_status(&db.pool, user_id).await?.expect("status row");
        assert_eq!(status.status, "logged_in");
        assert_eq!(status.device_count, 2);
        assert_eq!(
            status
                .devices
                .iter()
                .find(|device| device.device_key == "device-a")
                .map(|device| device.status.as_str()),
            Some("logged_out")
        );
        assert_eq!(
            status
                .devices
                .iter()
                .find(|device| device.device_key == "device-b")
                .map(|device| device.status.as_str()),
            Some("logged_in")
        );

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn touch_last_seen_throttles_recent_logged_in_updates() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.pool).await?;

        touch_last_seen(&db.pool, user_id, Some(org_id), Some("device-a".into())).await?;
        let first = client_status_row(&db.pool, user_id, "device-a").await?;

        touch_last_seen(&db.pool, user_id, Some(org_id), Some("device-a".into())).await?;
        let second = client_status_row(&db.pool, user_id, "device-a").await?;

        assert_eq!(second.status, "logged_in");
        assert_eq!(second.last_status_at, first.last_status_at);
        assert_eq!(second.last_seen_at, first.last_seen_at);
        assert_eq!(second.updated_at, first.updated_at);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn touch_last_seen_updates_after_throttle_window() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.pool).await?;

        touch_last_seen(&db.pool, user_id, Some(org_id), Some("device-a".into())).await?;
        sqlx::query(
            "UPDATE developer_client_status
             SET last_seen_at = now() - interval '61 seconds',
                 updated_at = now() - interval '61 seconds'
             WHERE user_id = $1 AND device_key = $2",
        )
        .bind(user_id)
        .bind("device-a")
        .execute(&db.pool)
        .await?;

        let stale = client_status_row(&db.pool, user_id, "device-a").await?;
        touch_last_seen(&db.pool, user_id, Some(org_id), Some("device-a".into())).await?;
        let refreshed = client_status_row(&db.pool, user_id, "device-a").await?;

        assert_eq!(refreshed.status, "logged_in");
        assert_eq!(refreshed.last_status_at, stale.last_status_at);
        assert!(refreshed.last_seen_at > stale.last_seen_at);
        assert!(refreshed.updated_at > stale.updated_at);

        db.cleanup().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn touch_last_seen_restores_logged_in_after_logout() -> anyhow::Result<()> {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (user_id, org_id) = insert_test_identity(&db.pool).await?;

        record_status(
            &db.pool,
            user_id,
            Some(org_id),
            ClientStatus::LoggedOut,
            metadata("device-a", "host-a"),
        )
        .await?;
        let logged_out = client_status_row(&db.pool, user_id, "device-a").await?;

        touch_last_seen(&db.pool, user_id, Some(org_id), Some("device-a".into())).await?;
        let logged_in = client_status_row(&db.pool, user_id, "device-a").await?;
        let status = get_status(&db.pool, user_id).await?.expect("status row");

        assert_eq!(logged_out.status, "logged_out");
        assert_eq!(logged_in.status, "logged_in");
        assert!(logged_in.last_status_at >= logged_out.last_status_at);
        assert!(logged_in.last_seen_at.is_some());
        assert_eq!(status.status, "logged_in");

        db.cleanup().await?;
        Ok(())
    }

    fn metadata(distinct_id: &str, hostname: &str) -> ClientStatusMetadata {
        ClientStatusMetadata {
            distinct_id: Some(distinct_id.to_string()),
            cli_version: Some("1.0.0".to_string()),
            os: Some("macos".to_string()),
            arch: Some("arm64".to_string()),
            hostname: Some(hostname.to_string()),
        }
    }

    async fn insert_test_identity(pool: &PgPool) -> anyhow::Result<(Uuid, Uuid)> {
        let user_id = Uuid::new_v4();
        let org_id = Uuid::new_v4();

        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org_id)
            .bind("Client Status Test Org")
            .bind(format!("client-status-test-{}", org_id.simple()))
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO users (id, email, name, default_org_id) VALUES ($1, $2, $3, $4)")
            .bind(user_id)
            .bind(format!("{user_id}@example.com"))
            .bind("Client Status Test User")
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

    #[derive(Debug)]
    struct ClientStatusRow {
        status: String,
        last_status_at: DateTime<Utc>,
        last_seen_at: Option<DateTime<Utc>>,
        updated_at: DateTime<Utc>,
    }

    async fn client_status_row(
        pool: &PgPool,
        user_id: Uuid,
        device_key: &str,
    ) -> anyhow::Result<ClientStatusRow> {
        let (status, last_status_at, last_seen_at, updated_at) = sqlx::query_as(
            r#"SELECT status, last_status_at, last_seen_at, updated_at
               FROM developer_client_status
               WHERE user_id = $1 AND device_key = $2"#,
        )
        .bind(user_id)
        .bind(device_key)
        .fetch_one(pool)
        .await?;

        Ok(ClientStatusRow {
            status,
            last_status_at,
            last_seen_at,
            updated_at,
        })
    }

    async fn table_count(pool: &PgPool, table: &str) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(pool)
            .await?)
    }

    fn test_database_url() -> String {
        dotenvy::dotenv().ok();
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://gitai:gitai@localhost:5433/gitai_enterprise".into())
    }

    fn unique_test_database_name() -> String {
        format!("git_ai_client_status_test_{}", Uuid::new_v4().simple())
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
