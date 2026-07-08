use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub database_max_connections: u32,
    pub database_min_connections: u32,
    pub database_acquire_timeout_seconds: u64,
    pub redis_url: String,
    pub jwt_secret: String,
    pub s3_endpoint: String,
    pub s3_bucket: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub s3_region: String,
    pub cas_upload_concurrency: usize,
    pub base_url: String,
    // Telemetry
    pub sentry_dsn: String,
    pub posthog_host: String,
    pub posthog_api_key: String,
}

#[derive(Debug, Deserialize)]
pub struct EnvConfig {
    pub database_url: String,
    pub database_max_connections: Option<u32>,
    pub database_min_connections: Option<u32>,
    pub database_acquire_timeout_seconds: Option<u64>,
    pub redis_url: String,
    pub jwt_secret: String,
    pub s3_endpoint: Option<String>,
    pub s3_bucket: Option<String>,
    pub s3_access_key: Option<String>,
    pub s3_secret_key: Option<String>,
    pub s3_region: Option<String>,
    pub cas_upload_concurrency: Option<usize>,
    pub base_url: Option<String>,
    // Telemetry
    pub sentry_dsn: Option<String>,
    pub posthog_host: Option<String>,
    pub posthog_api_key: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();
        let env: EnvConfig = envy::from_env()?;

        Ok(Self {
            database_url: env.database_url,
            database_max_connections: env.database_max_connections.unwrap_or(20),
            database_min_connections: env.database_min_connections.unwrap_or(1),
            database_acquire_timeout_seconds: env.database_acquire_timeout_seconds.unwrap_or(5),
            redis_url: env.redis_url,
            jwt_secret: env.jwt_secret,
            s3_endpoint: env
                .s3_endpoint
                .unwrap_or_else(|| "http://localhost:9000".into()),
            s3_bucket: env.s3_bucket.unwrap_or_else(|| "git-ai-cas".into()),
            s3_access_key: env.s3_access_key.unwrap_or_else(|| "minioadmin".into()),
            s3_secret_key: env.s3_secret_key.unwrap_or_else(|| "minioadmin".into()),
            s3_region: env.s3_region.unwrap_or_else(|| "us-east-1".into()),
            cas_upload_concurrency: env.cas_upload_concurrency.unwrap_or(8).max(1),
            base_url: env
                .base_url
                .unwrap_or_else(|| "http://localhost:8080".into()),
            sentry_dsn: env.sentry_dsn.unwrap_or_default(),
            posthog_host: env.posthog_host.unwrap_or_default(),
            posthog_api_key: env.posthog_api_key.unwrap_or_default(),
        })
    }
}
