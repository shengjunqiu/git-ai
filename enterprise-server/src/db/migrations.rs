//! Database migration runner
//!
//! Executes SQL migrations programmatically instead of using the sqlx::migrate! macro,
//! which requires a live database connection at compile time.

use sqlx::{PgConnection, PgPool};

const MIGRATION_LOCK_KEY: &str = "git-ai-enterprise-server:migrations";

const MIGRATIONS: &[(&str, &str)] = &[
    (
        "001_initial_schema",
        include_str!("../../migrations/001_initial_schema.sql"),
    ),
    (
        "002_repo_access_list",
        include_str!("../../migrations/002_repo_access_list.sql"),
    ),
    (
        "003_phase6_enterprise",
        include_str!("../../migrations/003_phase6_enterprise.sql"),
    ),
    (
        "004_data_isolation",
        include_str!("../../migrations/004_data_isolation.sql"),
    ),
    (
        "006_developer_registration_cli_auth",
        include_str!("../../migrations/006_developer_registration_cli_auth.sql"),
    ),
    (
        "007_project_scope_by_user_org",
        include_str!("../../migrations/007_project_scope_by_user_org.sql"),
    ),
    (
        "008_default_linewell_registration_options",
        include_str!("../../migrations/008_default_linewell_registration_options.sql"),
    ),
    (
        "009_fix_metrics_ai_additions_all_rollup",
        include_str!("../../migrations/009_fix_metrics_ai_additions_all_rollup.sql"),
    ),
    (
        "010_backfill_metrics_rollup_fields",
        include_str!("../../migrations/010_backfill_metrics_rollup_fields.sql"),
    ),
    (
        "011_developer_client_status",
        include_str!("../../migrations/011_developer_client_status.sql"),
    ),
    (
        "012_developer_client_status_device_key",
        include_str!("../../migrations/012_developer_client_status_device_key.sql"),
    ),
    (
        "013_version_release_assets",
        include_str!("../../migrations/013_version_release_assets.sql"),
    ),
    (
        "014_metrics_query_indexes",
        include_str!("../../migrations/014_metrics_query_indexes.sql"),
    ),
    (
        "015_commit_stats_author_time_at",
        include_str!("../../migrations/015_commit_stats_author_time_at.sql"),
    ),
    (
        "016_metrics_daily_rollups",
        include_str!("../../migrations/016_metrics_daily_rollups.sql"),
    ),
    (
        "017_metrics_tool_model_events",
        include_str!("../../migrations/017_metrics_tool_model_events.sql"),
    ),
    (
        "018_users_lower_email_index",
        include_str!("../../migrations/018_users_lower_email_index.sql"),
    ),
    (
        "019_metrics_rollup_dirty_scopes",
        include_str!("../../migrations/019_metrics_rollup_dirty_scopes.sql"),
    ),
    (
        "020_log_pagination_indexes",
        include_str!("../../migrations/020_log_pagination_indexes.sql"),
    ),
    (
        "021_admin_list_pagination_indexes",
        include_str!("../../migrations/021_admin_list_pagination_indexes.sql"),
    ),
    (
        "022_pull_request_pagination_indexes",
        include_str!("../../migrations/022_pull_request_pagination_indexes.sql"),
    ),
    (
        "023_department_rollup_indexes",
        include_str!("../../migrations/023_department_rollup_indexes.sql"),
    ),
    (
        "024_dashboard_aggregate_pagination_indexes",
        include_str!("../../migrations/024_dashboard_aggregate_pagination_indexes.sql"),
    ),
    (
        "025_dashboard_rollup_project_indexes",
        include_str!("../../migrations/025_dashboard_rollup_project_indexes.sql"),
    ),
    (
        "026_dashboard_git_identity_index",
        include_str!("../../migrations/026_dashboard_git_identity_index.sql"),
    ),
    (
        "027_department_aggregate_fallback_index",
        include_str!("../../migrations/027_department_aggregate_fallback_index.sql"),
    ),
];

/// Run all database migrations
pub async fn run_migrations(pool: &PgPool) -> anyhow::Result<()> {
    let mut conn = pool.acquire().await?;

    acquire_migration_lock(&mut conn).await?;
    let migration_result = run_migrations_locked(&mut conn).await;
    let unlock_result = release_migration_lock(&mut conn).await;

    if let Err(unlock_error) = unlock_result {
        if migration_result.is_ok() {
            return Err(unlock_error);
        }
        tracing::warn!("failed to release migration advisory lock: {unlock_error}");
    }

    migration_result
}

async fn acquire_migration_lock(conn: &mut PgConnection) -> anyhow::Result<()> {
    sqlx::query("SELECT pg_advisory_lock(hashtext($1))")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn release_migration_lock(conn: &mut PgConnection) -> anyhow::Result<()> {
    sqlx::query("SELECT pg_advisory_unlock(hashtext($1))")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn run_migrations_locked(conn: &mut PgConnection) -> anyhow::Result<()> {
    // Create migration tracking table
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _migrations (
            id SERIAL PRIMARY KEY,
            name TEXT UNIQUE NOT NULL,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )",
    )
    .execute(&mut *conn)
    .await?;

    for (name, sql) in MIGRATIONS {
        // Check if already applied
        let applied: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _migrations WHERE name = $1)")
                .bind(name)
                .fetch_one(&mut *conn)
                .await
                .unwrap_or(false);

        if !applied {
            tracing::info!("Applying migration: {}", name);
            sqlx::raw_sql(sql).execute(&mut *conn).await?;
            sqlx::query("INSERT INTO _migrations (name) VALUES ($1)")
                .bind(name)
                .execute(&mut *conn)
                .await?;
            tracing::info!("Migration {} applied successfully", name);
        } else {
            tracing::debug!("Migration {} already applied, skipping", name);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_migrations_serializes_concurrent_callers() -> anyhow::Result<()> {
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
                    "skipping migration concurrency test: could not connect to admin database: {error}"
                );
                return Ok(());
            }
        };

        if let Err(error) = create_database(&admin_pool, &db_name).await {
            eprintln!(
                "skipping migration concurrency test: could not create isolated database {db_name}: {error}"
            );
            return Ok(());
        }

        let test_pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&test_url)
            .await?;

        let pool_a = test_pool.clone();
        let pool_b = test_pool.clone();
        let first = async move { run_migrations(&pool_a).await };
        let second = async move { run_migrations(&pool_b).await };

        let (first_result, second_result) = tokio::join!(first, second);
        first_result?;
        second_result?;

        let duplicate_names: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM (
                SELECT name FROM _migrations GROUP BY name HAVING COUNT(*) > 1
            ) duplicate_migrations",
        )
        .fetch_one(&test_pool)
        .await?;
        assert_eq!(duplicate_names, 0);

        let applied_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _migrations")
            .fetch_one(&test_pool)
            .await?;
        assert_eq!(applied_count as usize, MIGRATIONS.len());

        let lower_email_index_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1
                FROM pg_indexes
                WHERE schemaname = 'public'
                  AND indexname = 'idx_users_email_lower'
            )",
        )
        .fetch_one(&test_pool)
        .await?;
        assert!(lower_email_index_exists);

        let dirty_scopes_table_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1
                FROM information_schema.tables
                WHERE table_schema = 'public'
                  AND table_name = 'metrics_rollup_dirty_scopes'
            )",
        )
        .fetch_one(&test_pool)
        .await?;
        assert!(dirty_scopes_table_exists);

        let dirty_claim_index_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1
                FROM pg_indexes
                WHERE schemaname = 'public'
                  AND indexname = 'idx_metrics_rollup_dirty_claim'
            )",
        )
        .fetch_one(&test_pool)
        .await?;
        assert!(dirty_claim_index_exists);

        let dirty_scope_id_column_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1
                FROM information_schema.columns
                WHERE table_schema = 'public'
                  AND table_name = 'metrics_rollup_dirty_scopes'
                  AND column_name = 'id'
            )",
        )
        .fetch_one(&test_pool)
        .await?;
        assert!(dirty_scope_id_column_exists);

        let dirty_scope_index_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1
                FROM pg_indexes
                WHERE schemaname = 'public'
                  AND indexname = 'idx_metrics_rollup_dirty_scope'
            )",
        )
        .fetch_one(&test_pool)
        .await?;
        assert!(dirty_scope_index_exists);

        test_pool.close().await;
        drop_database(&admin_pool, &db_name).await?;
        admin_pool.close().await;

        Ok(())
    }

    fn test_database_url() -> String {
        dotenvy::dotenv().ok();
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://gitai:gitai@localhost:5433/gitai_enterprise".into())
    }

    fn unique_test_database_name() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        format!("git_ai_migration_test_{}_{}", std::process::id(), nanos)
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
