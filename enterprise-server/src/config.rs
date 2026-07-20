use serde::Deserialize;
use url::Url;

const DEFAULT_METRICS_ROLLUP_WORKER_INTERVAL_SECONDS: u64 = 5;
const DEFAULT_METRICS_ROLLUP_WORKER_BATCH_SIZE: i64 = 100;
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
const DEFAULT_RATE_LIMIT_ADMIN_MAX_REQUESTS: u32 = 300;
const DEFAULT_RATE_LIMIT_ADMIN_WINDOW_SECONDS: u64 = 60;
const DEFAULT_RATE_LIMIT_DEFAULT_MAX_REQUESTS: u32 = 300;
const DEFAULT_RATE_LIMIT_DEFAULT_WINDOW_SECONDS: u64 = 60;
const DEFAULT_AUTH_PASSWORD_CONCURRENCY: usize = 8;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetricsRollupWriteMode {
    Sync,
    DirtyAsync,
    Off,
}

impl MetricsRollupWriteMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::DirtyAsync => "dirty_async",
            Self::Off => "off",
        }
    }
}

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
    pub auth_password_concurrency: usize,
    pub metrics_rollup_write_mode: MetricsRollupWriteMode,
    pub metrics_rollup_worker_enabled: bool,
    pub metrics_rollup_worker_interval_seconds: u64,
    pub metrics_rollup_worker_batch_size: i64,
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
    pub auth_password_concurrency: Option<usize>,
    pub metrics_write_rollups: Option<bool>,
    pub metrics_rollup_write_mode: Option<MetricsRollupWriteMode>,
    pub metrics_rollup_worker_enabled: Option<bool>,
    pub metrics_rollup_worker_interval_seconds: Option<u64>,
    pub metrics_rollup_worker_batch_size: Option<i64>,
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
    pub allow_insecure_public_url: Option<bool>,
    // Telemetry
    pub sentry_dsn: Option<String>,
    pub posthog_host: Option<String>,
    pub posthog_api_key: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();
        let env: EnvConfig = envy::from_env()?;
        let allow_insecure_public_url = env.allow_insecure_public_url.unwrap_or(false);
        let mut config = Self::from_env_config(env);
        config.base_url = validate_public_base_url(&config.base_url, allow_insecure_public_url)?;

        Ok(config)
    }

    fn from_env_config(env: EnvConfig) -> Self {
        let legacy_metrics_write_rollups = env.metrics_write_rollups.unwrap_or(true);
        let metrics_rollup_write_mode =
            env.metrics_rollup_write_mode
                .unwrap_or(if legacy_metrics_write_rollups {
                    MetricsRollupWriteMode::Sync
                } else {
                    MetricsRollupWriteMode::Off
                });

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
            auth_password_concurrency: env
                .auth_password_concurrency
                .unwrap_or(DEFAULT_AUTH_PASSWORD_CONCURRENCY)
                .max(1),
            metrics_rollup_write_mode,
            metrics_rollup_worker_enabled: env.metrics_rollup_worker_enabled.unwrap_or(false),
            metrics_rollup_worker_interval_seconds: window_seconds(
                env.metrics_rollup_worker_interval_seconds,
                DEFAULT_METRICS_ROLLUP_WORKER_INTERVAL_SECONDS,
            ),
            metrics_rollup_worker_batch_size: env
                .metrics_rollup_worker_batch_size
                .unwrap_or(DEFAULT_METRICS_ROLLUP_WORKER_BATCH_SIZE)
                .max(1),
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

fn validate_public_base_url(value: &str, allow_insecure: bool) -> anyhow::Result<String> {
    let mut url = Url::parse(value)
        .map_err(|error| anyhow::anyhow!("BASE_URL must be an absolute URL: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!("BASE_URL must use http or https");
    }
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        anyhow::bail!("BASE_URL must be an origin without credentials, path, query, or fragment");
    }
    if url.scheme() == "http" && !is_loopback_host(url.host_str().unwrap_or_default()) {
        if !allow_insecure {
            anyhow::bail!(
                "BASE_URL must use HTTPS outside localhost; set ALLOW_INSECURE_PUBLIC_URL=true only for an explicitly accepted development deployment"
            );
        }
        tracing::warn!(
            base_url = %url,
            "ALLOW_INSECURE_PUBLIC_URL is enabled; credentials and installers can be intercepted"
        );
    }

    url.set_path("");
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
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
        assert_eq!(config.rate_limit_admin_max_requests, 300);
        assert_eq!(config.rate_limit_admin_window_seconds, 60);
        assert_eq!(config.rate_limit_default_max_requests, 300);
        assert_eq!(config.rate_limit_default_window_seconds, 60);
        assert_eq!(config.auth_password_concurrency, 8);
        assert_eq!(
            config.metrics_rollup_write_mode,
            MetricsRollupWriteMode::Sync
        );
        assert!(!config.metrics_rollup_worker_enabled);
        assert_eq!(config.metrics_rollup_worker_interval_seconds, 5);
        assert_eq!(config.metrics_rollup_worker_batch_size, 100);
    }

    #[test]
    fn config_overrides_rate_limit_tiers() {
        let mut env = minimal_env_config();
        env.rate_limit_auth_max_requests = Some(12);
        env.rate_limit_auth_window_seconds = Some(30);
        env.rate_limit_oauth_max_requests = Some(0);
        env.rate_limit_oauth_window_seconds = Some(0);
        env.auth_password_concurrency = Some(0);

        let config = AppConfig::from_env_config(env);

        assert_eq!(config.rate_limit_auth_max_requests, 12);
        assert_eq!(config.rate_limit_auth_window_seconds, 30);
        assert_eq!(config.rate_limit_oauth_max_requests, 1);
        assert_eq!(config.rate_limit_oauth_window_seconds, 1);
        assert_eq!(config.auth_password_concurrency, 1);
    }

    #[test]
    fn legacy_metrics_write_rollups_controls_default_rollup_mode() {
        let mut env = minimal_env_config();
        env.metrics_write_rollups = Some(false);

        let config = AppConfig::from_env_config(env);

        assert_eq!(
            config.metrics_rollup_write_mode,
            MetricsRollupWriteMode::Off
        );
    }

    #[test]
    fn metrics_rollup_write_mode_overrides_legacy_rollup_bool() {
        let mut env = minimal_env_config();
        env.metrics_write_rollups = Some(false);
        env.metrics_rollup_write_mode = Some(MetricsRollupWriteMode::DirtyAsync);

        let config = AppConfig::from_env_config(env);

        assert_eq!(
            config.metrics_rollup_write_mode,
            MetricsRollupWriteMode::DirtyAsync
        );
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
                ("AUTH_PASSWORD_CONCURRENCY", "4"),
                ("METRICS_ROLLUP_WRITE_MODE", "dirty_async"),
                ("METRICS_ROLLUP_WORKER_ENABLED", "true"),
                ("METRICS_ROLLUP_WORKER_INTERVAL_SECONDS", "9"),
                ("METRICS_ROLLUP_WORKER_BATCH_SIZE", "42"),
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
        assert_eq!(config.auth_password_concurrency, 4);
        assert_eq!(
            config.metrics_rollup_write_mode,
            MetricsRollupWriteMode::DirtyAsync
        );
        assert!(config.metrics_rollup_worker_enabled);
        assert_eq!(config.metrics_rollup_worker_interval_seconds, 9);
        assert_eq!(config.metrics_rollup_worker_batch_size, 42);
        Ok(())
    }

    #[test]
    fn public_base_url_requires_https_outside_loopback() {
        assert_eq!(
            validate_public_base_url("https://git-ai.example.com/", false).unwrap(),
            "https://git-ai.example.com"
        );
        assert_eq!(
            validate_public_base_url("http://127.0.0.1:8080", false).unwrap(),
            "http://127.0.0.1:8080"
        );
        assert!(validate_public_base_url("http://git-ai.example.com", false).is_err());
        assert_eq!(
            validate_public_base_url("http://git-ai.example.com", true).unwrap(),
            "http://git-ai.example.com"
        );
    }

    #[test]
    fn public_base_url_rejects_untrusted_url_components() {
        for value in [
            "javascript:alert(1)",
            "https://user:secret@git-ai.example.com",
            "https://git-ai.example.com/prefix",
            "https://git-ai.example.com/?redirect=evil",
            "https://git-ai.example.com/#fragment",
        ] {
            assert!(validate_public_base_url(value, false).is_err(), "{value}");
        }
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
            auth_password_concurrency: None,
            metrics_write_rollups: None,
            metrics_rollup_write_mode: None,
            metrics_rollup_worker_enabled: None,
            metrics_rollup_worker_interval_seconds: None,
            metrics_rollup_worker_batch_size: None,
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
            allow_insecure_public_url: None,
            sentry_dsn: None,
            posthog_host: None,
            posthog_api_key: None,
        }
    }
}
