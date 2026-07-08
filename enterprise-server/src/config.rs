use serde::Deserialize;

const DEFAULT_RATE_LIMIT_METRICS_MAX_REQUESTS: u32 = 60;
const DEFAULT_RATE_LIMIT_METRICS_WINDOW_SECONDS: u64 = 60;
const DEFAULT_RATE_LIMIT_CAS_UPLOAD_MAX_REQUESTS: u32 = 30;
const DEFAULT_RATE_LIMIT_CAS_UPLOAD_WINDOW_SECONDS: u64 = 60;
const DEFAULT_RATE_LIMIT_CAS_READ_MAX_REQUESTS: u32 = 100;
const DEFAULT_RATE_LIMIT_CAS_READ_WINDOW_SECONDS: u64 = 60;
const DEFAULT_RATE_LIMIT_OAUTH_MAX_REQUESTS: u32 = 600;
const DEFAULT_RATE_LIMIT_OAUTH_WINDOW_SECONDS: u64 = 60;
const DEFAULT_RATE_LIMIT_AUTH_MAX_REQUESTS: u32 = 300;
const DEFAULT_RATE_LIMIT_AUTH_WINDOW_SECONDS: u64 = 60;
const DEFAULT_RATE_LIMIT_ADMIN_MAX_REQUESTS: u32 = 30;
const DEFAULT_RATE_LIMIT_ADMIN_WINDOW_SECONDS: u64 = 60;
const DEFAULT_RATE_LIMIT_DEFAULT_MAX_REQUESTS: u32 = 300;
const DEFAULT_RATE_LIMIT_DEFAULT_WINDOW_SECONDS: u64 = 60;

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
    pub metrics_write_rollups: bool,
    pub dashboard_use_rollups: bool,
    // Rate limiting
    pub rate_limit_metrics_max_requests: u32,
    pub rate_limit_metrics_window_seconds: u64,
    pub rate_limit_cas_upload_max_requests: u32,
    pub rate_limit_cas_upload_window_seconds: u64,
    pub rate_limit_cas_read_max_requests: u32,
    pub rate_limit_cas_read_window_seconds: u64,
    pub rate_limit_oauth_max_requests: u32,
    pub rate_limit_oauth_window_seconds: u64,
    pub rate_limit_auth_max_requests: u32,
    pub rate_limit_auth_window_seconds: u64,
    pub rate_limit_admin_max_requests: u32,
    pub rate_limit_admin_window_seconds: u64,
    pub rate_limit_default_max_requests: u32,
    pub rate_limit_default_window_seconds: u64,
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
    pub metrics_write_rollups: Option<bool>,
    pub dashboard_use_rollups: Option<bool>,
    pub rate_limit_metrics_max_requests: Option<u32>,
    pub rate_limit_metrics_window_seconds: Option<u64>,
    pub rate_limit_cas_upload_max_requests: Option<u32>,
    pub rate_limit_cas_upload_window_seconds: Option<u64>,
    pub rate_limit_cas_read_max_requests: Option<u32>,
    pub rate_limit_cas_read_window_seconds: Option<u64>,
    pub rate_limit_oauth_max_requests: Option<u32>,
    pub rate_limit_oauth_window_seconds: Option<u64>,
    pub rate_limit_auth_max_requests: Option<u32>,
    pub rate_limit_auth_window_seconds: Option<u64>,
    pub rate_limit_admin_max_requests: Option<u32>,
    pub rate_limit_admin_window_seconds: Option<u64>,
    pub rate_limit_default_max_requests: Option<u32>,
    pub rate_limit_default_window_seconds: Option<u64>,
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

        Ok(Self::from_env_config(env))
    }

    fn from_env_config(env: EnvConfig) -> Self {
        Self {
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
            metrics_write_rollups: env.metrics_write_rollups.unwrap_or(true),
            dashboard_use_rollups: env.dashboard_use_rollups.unwrap_or(false),
            rate_limit_metrics_max_requests: max_requests(
                env.rate_limit_metrics_max_requests,
                DEFAULT_RATE_LIMIT_METRICS_MAX_REQUESTS,
            ),
            rate_limit_metrics_window_seconds: window_seconds(
                env.rate_limit_metrics_window_seconds,
                DEFAULT_RATE_LIMIT_METRICS_WINDOW_SECONDS,
            ),
            rate_limit_cas_upload_max_requests: max_requests(
                env.rate_limit_cas_upload_max_requests,
                DEFAULT_RATE_LIMIT_CAS_UPLOAD_MAX_REQUESTS,
            ),
            rate_limit_cas_upload_window_seconds: window_seconds(
                env.rate_limit_cas_upload_window_seconds,
                DEFAULT_RATE_LIMIT_CAS_UPLOAD_WINDOW_SECONDS,
            ),
            rate_limit_cas_read_max_requests: max_requests(
                env.rate_limit_cas_read_max_requests,
                DEFAULT_RATE_LIMIT_CAS_READ_MAX_REQUESTS,
            ),
            rate_limit_cas_read_window_seconds: window_seconds(
                env.rate_limit_cas_read_window_seconds,
                DEFAULT_RATE_LIMIT_CAS_READ_WINDOW_SECONDS,
            ),
            rate_limit_oauth_max_requests: max_requests(
                env.rate_limit_oauth_max_requests,
                DEFAULT_RATE_LIMIT_OAUTH_MAX_REQUESTS,
            ),
            rate_limit_oauth_window_seconds: window_seconds(
                env.rate_limit_oauth_window_seconds,
                DEFAULT_RATE_LIMIT_OAUTH_WINDOW_SECONDS,
            ),
            rate_limit_auth_max_requests: max_requests(
                env.rate_limit_auth_max_requests,
                DEFAULT_RATE_LIMIT_AUTH_MAX_REQUESTS,
            ),
            rate_limit_auth_window_seconds: window_seconds(
                env.rate_limit_auth_window_seconds,
                DEFAULT_RATE_LIMIT_AUTH_WINDOW_SECONDS,
            ),
            rate_limit_admin_max_requests: max_requests(
                env.rate_limit_admin_max_requests,
                DEFAULT_RATE_LIMIT_ADMIN_MAX_REQUESTS,
            ),
            rate_limit_admin_window_seconds: window_seconds(
                env.rate_limit_admin_window_seconds,
                DEFAULT_RATE_LIMIT_ADMIN_WINDOW_SECONDS,
            ),
            rate_limit_default_max_requests: max_requests(
                env.rate_limit_default_max_requests,
                DEFAULT_RATE_LIMIT_DEFAULT_MAX_REQUESTS,
            ),
            rate_limit_default_window_seconds: window_seconds(
                env.rate_limit_default_window_seconds,
                DEFAULT_RATE_LIMIT_DEFAULT_WINDOW_SECONDS,
            ),
            base_url: env
                .base_url
                .unwrap_or_else(|| "http://localhost:8080".into()),
            sentry_dsn: env.sentry_dsn.unwrap_or_default(),
            posthog_host: env.posthog_host.unwrap_or_default(),
            posthog_api_key: env.posthog_api_key.unwrap_or_default(),
        }
    }
}

fn max_requests(value: Option<u32>, default: u32) -> u32 {
    value.unwrap_or(default).max(1)
}

fn window_seconds(value: Option<u64>, default: u64) -> u64 {
    value.unwrap_or(default).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_rate_limit_tiers() {
        let config = AppConfig::from_env_config(minimal_env_config());

        assert_eq!(config.rate_limit_metrics_max_requests, 60);
        assert_eq!(config.rate_limit_metrics_window_seconds, 60);
        assert_eq!(config.rate_limit_oauth_max_requests, 600);
        assert_eq!(config.rate_limit_oauth_window_seconds, 60);
        assert_eq!(config.rate_limit_auth_max_requests, 300);
        assert_eq!(config.rate_limit_auth_window_seconds, 60);
        assert_eq!(config.rate_limit_default_max_requests, 300);
        assert_eq!(config.rate_limit_default_window_seconds, 60);
    }

    #[test]
    fn config_overrides_rate_limit_tiers() {
        let mut env = minimal_env_config();
        env.rate_limit_auth_max_requests = Some(12);
        env.rate_limit_auth_window_seconds = Some(30);
        env.rate_limit_oauth_max_requests = Some(0);
        env.rate_limit_oauth_window_seconds = Some(0);

        let config = AppConfig::from_env_config(env);

        assert_eq!(config.rate_limit_auth_max_requests, 12);
        assert_eq!(config.rate_limit_auth_window_seconds, 30);
        assert_eq!(config.rate_limit_oauth_max_requests, 1);
        assert_eq!(config.rate_limit_oauth_window_seconds, 1);
    }

    #[test]
    fn config_reads_rate_limit_environment_variables() -> anyhow::Result<()> {
        let env: EnvConfig = envy::from_iter(
            [
                ("DATABASE_URL", "postgresql://localhost/test"),
                ("REDIS_URL", "redis://localhost:6379"),
                ("JWT_SECRET", "test-secret"),
                ("RATE_LIMIT_AUTH_MAX_REQUESTS", "12"),
                ("RATE_LIMIT_AUTH_WINDOW_SECONDS", "30"),
                ("RATE_LIMIT_OAUTH_MAX_REQUESTS", "24"),
                ("RATE_LIMIT_OAUTH_WINDOW_SECONDS", "45"),
                ("RATE_LIMIT_DEFAULT_MAX_REQUESTS", "36"),
                ("RATE_LIMIT_DEFAULT_WINDOW_SECONDS", "60"),
                ("RATE_LIMIT_METRICS_MAX_REQUESTS", "48"),
                ("RATE_LIMIT_METRICS_WINDOW_SECONDS", "75"),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string())),
        )?;

        let config = AppConfig::from_env_config(env);

        assert_eq!(config.rate_limit_auth_max_requests, 12);
        assert_eq!(config.rate_limit_auth_window_seconds, 30);
        assert_eq!(config.rate_limit_oauth_max_requests, 24);
        assert_eq!(config.rate_limit_oauth_window_seconds, 45);
        assert_eq!(config.rate_limit_default_max_requests, 36);
        assert_eq!(config.rate_limit_default_window_seconds, 60);
        assert_eq!(config.rate_limit_metrics_max_requests, 48);
        assert_eq!(config.rate_limit_metrics_window_seconds, 75);
        Ok(())
    }

    fn minimal_env_config() -> EnvConfig {
        EnvConfig {
            database_url: "postgresql://localhost/test".to_string(),
            database_max_connections: None,
            database_min_connections: None,
            database_acquire_timeout_seconds: None,
            redis_url: "redis://localhost:6379".to_string(),
            jwt_secret: "test-secret".to_string(),
            s3_endpoint: None,
            s3_bucket: None,
            s3_access_key: None,
            s3_secret_key: None,
            s3_region: None,
            cas_upload_concurrency: None,
            metrics_write_rollups: None,
            dashboard_use_rollups: None,
            rate_limit_metrics_max_requests: None,
            rate_limit_metrics_window_seconds: None,
            rate_limit_cas_upload_max_requests: None,
            rate_limit_cas_upload_window_seconds: None,
            rate_limit_cas_read_max_requests: None,
            rate_limit_cas_read_window_seconds: None,
            rate_limit_oauth_max_requests: None,
            rate_limit_oauth_window_seconds: None,
            rate_limit_auth_max_requests: None,
            rate_limit_auth_window_seconds: None,
            rate_limit_admin_max_requests: None,
            rate_limit_admin_window_seconds: None,
            rate_limit_default_max_requests: None,
            rate_limit_default_window_seconds: None,
            base_url: None,
            sentry_dsn: None,
            posthog_host: None,
            posthog_api_key: None,
        }
    }
}
