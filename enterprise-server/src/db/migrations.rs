//! Database migration runner
//!
//! Executes SQL migrations programmatically instead of using the sqlx::migrate! macro,
//! which requires a live database connection at compile time.

use sqlx::PgPool;

/// Run all database migrations
pub async fn run_migrations(pool: &PgPool) -> anyhow::Result<()> {
    // Create migration tracking table
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _migrations (
            id SERIAL PRIMARY KEY,
            name TEXT UNIQUE NOT NULL,
            applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )",
    )
    .execute(pool)
    .await?;

    // Apply migrations in order
    let migrations: &[(&str, &str)] = &[
        ("001_initial_schema", include_str!("../../migrations/001_initial_schema.sql")),
        ("002_repo_access_list", include_str!("../../migrations/002_repo_access_list.sql")),
        ("003_phase6_enterprise", include_str!("../../migrations/003_phase6_enterprise.sql")),
        ("004_data_isolation", include_str!("../../migrations/004_data_isolation.sql")),
        ("006_developer_registration_cli_auth", include_str!("../../migrations/006_developer_registration_cli_auth.sql")),
    ];

    for (name, sql) in migrations {
        // Check if already applied
        let applied: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM _migrations WHERE name = $1)",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap_or(false);

        if !applied {
            tracing::info!("Applying migration: {}", name);
            sqlx::raw_sql(sql).execute(pool).await?;
            sqlx::query("INSERT INTO _migrations (name) VALUES ($1)")
                .bind(name)
                .execute(pool)
                .await?;
            tracing::info!("Migration {} applied successfully", name);
        } else {
            tracing::debug!("Migration {} already applied, skipping", name);
        }
    }

    Ok(())
}
