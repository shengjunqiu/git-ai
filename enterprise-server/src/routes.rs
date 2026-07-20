use axum::extract::DefaultBodyLimit;
use axum::http::{header, Extensions, HeaderMap, StatusCode, Version};
use axum::middleware;
use axum::routing::{delete, get, post, put};
use axum::Router;
use tower_http::compression::predicate::{Predicate, SizeAbove};
use tower_http::compression::{CompressionLayer, CompressionLevel};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::auth::middleware::request_id_middleware;
use crate::config::AppConfig;
use crate::services::cas::CasStore;
use crate::services::rate_limit::RateLimiter;

/// Shared application state
#[derive(Clone, Debug)]
pub struct AppState {
    pub db: sqlx::PgPool,
    pub redis: redis::Client,
    pub config: AppConfig,
    pub cas_store: CasStore,
    pub rate_limiter: RateLimiter,
    pub auth_password_limiter: std::sync::Arc<tokio::sync::Semaphore>,
}

pub fn auth_password_limiter(config: &AppConfig) -> std::sync::Arc<tokio::sync::Semaphore> {
    std::sync::Arc::new(tokio::sync::Semaphore::new(
        config.auth_password_concurrency,
    ))
}

pub(crate) fn response_has_compressible_content_type(
    _status: StatusCode,
    _version: Version,
    headers: &HeaderMap,
    _extensions: &Extensions,
) -> bool {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    content_type.starts_with("text/")
        || matches!(
            content_type.as_str(),
            "application/javascript"
                | "application/json"
                | "application/wasm"
                | "application/xhtml+xml"
                | "application/xml"
                | "image/svg+xml"
        )
        || content_type.ends_with("+json")
        || content_type.ends_with("+xml")
}

/// Build the complete router with all routes
pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    let compression = CompressionLayer::new()
        .gzip(true)
        .br(true)
        .quality(CompressionLevel::Precise(5))
        .compress_when(SizeAbove::new(256).and(response_has_compressible_content_type));

    Router::new()
        .route("/", get(crate::handlers::dashboard::dashboard_root))
        // Health checks
        .route("/health", get(crate::handlers::health::health_check))
        .route("/ready", get(crate::handlers::health::readiness_check))
        // OAuth
        .route(
            "/worker/oauth/device/code",
            post(crate::handlers::oauth::device_code),
        )
        .route("/worker/oauth/token", post(crate::handlers::oauth::token))
        // OAuth verification page
        .route(
            "/verify",
            get(crate::handlers::verify::verify_page).post(crate::handlers::verify::verify_submit),
        )
        // Dashboard login / logout
        .route(
            "/login",
            get(crate::handlers::login::login_page).post(crate::handlers::login::login_submit),
        )
        .route("/logout", get(crate::handlers::login::logout))
        // Developer account auth / CLI browser authorization
        .route(
            "/auth/register",
            get(crate::handlers::auth_pages::register_page)
                .post(crate::handlers::auth_api::register),
        )
        .route(
            "/auth/login",
            get(crate::handlers::auth_pages::login_page).post(crate::handlers::auth_api::login),
        )
        .route("/auth/logout", post(crate::handlers::auth_api::logout))
        .route(
            "/auth/organizations",
            get(crate::handlers::auth_api::organizations),
        )
        .route(
            "/auth/organizations/{org_id}/departments",
            get(crate::handlers::auth_api::departments),
        )
        .route(
            "/auth/cli/authorize",
            get(crate::handlers::cli_authorize::authorize_page)
                .post(crate::handlers::cli_authorize::authorize_submit),
        )
        // Metrics
        .route(
            "/worker/metrics/upload",
            post(crate::handlers::metrics::upload_metrics),
        )
        .route(
            "/worker/client/status",
            post(crate::handlers::client_status::update_client_status),
        )
        // CAS
        .route("/worker/cas/upload", post(crate::handlers::cas::upload_cas))
        .route("/worker/cas", get(crate::handlers::cas::read_cas))
        // Releases
        .route(
            "/worker/releases",
            get(crate::handlers::release::get_releases),
        )
        .route(
            "/worker/releases/{channel}/download/{filename}",
            get(crate::handlers::release::download_release),
        )
        .route(
            "/files/{slug}/{version}/download",
            get(crate::handlers::managed_files::download_managed_file),
        )
        // Feature Flags
        .route(
            "/worker/config/feature-flags",
            get(crate::handlers::release::get_feature_flags),
        )
        // JetBrains plugin proxy
        .route(
            "/worker/plugins/jetbrains/download",
            get(crate::handlers::jetbrains::download_jetbrains_plugin),
        )
        // Telemetry proxies
        .route(
            "/worker/telemetry/sentry/{project_id}/store",
            post(crate::handlers::telemetry::sentry_proxy),
        )
        .route(
            "/worker/telemetry/posthog/capture",
            post(crate::handlers::telemetry::posthog_proxy),
        )
        // Reports
        .route(
            "/api/v1/reports",
            post(crate::handlers::report::upload_report),
        )
        .route(
            "/api/v1/summaries",
            post(crate::handlers::report::upload_summary),
        )
        // Bundles
        .route("/api/bundles", post(crate::handlers::bundle::create_bundle))
        // Bundle public share page
        .route(
            "/bundle/{id}",
            get(crate::handlers::bundle_view::view_bundle),
        )
        // Dashboard
        .route(
            "/static/{*path}",
            get(crate::handlers::dashboard::dashboard_static_asset),
        )
        .route("/me", get(crate::handlers::dashboard::dashboard_me))
        .route(
            "/api/v1/aggregate/summary",
            get(crate::handlers::dashboard::aggregate_summary),
        )
        .route(
            "/api/v1/aggregate/organizations",
            get(crate::handlers::dashboard::aggregate_organizations),
        )
        .route(
            "/api/v1/departments",
            get(crate::handlers::dashboard::list_departments),
        )
        .route(
            "/api/v1/aggregate/departments",
            get(crate::handlers::dashboard::aggregate_departments),
        )
        .route(
            "/api/v1/aggregate/projects",
            get(crate::handlers::dashboard::aggregate_projects),
        )
        .route(
            "/api/v1/aggregate/developers",
            get(crate::handlers::dashboard::aggregate_developers),
        )
        .route(
            "/api/v1/aggregate/tools",
            get(crate::handlers::dashboard::aggregate_tools),
        )
        .route(
            "/api/v1/client/status",
            get(crate::handlers::client_status::current_client_status),
        )
        // Audit log query
        .route(
            "/api/v1/audit-log",
            get(crate::handlers::admin::list_audit_log),
        )
        // Admin: Users
        .route(
            "/api/admin/users",
            post(crate::handlers::admin::create_user),
        )
        .route(
            "/api/admin/users/{id}",
            get(crate::handlers::admin::get_user)
                .put(crate::handlers::admin::update_user)
                .delete(crate::handlers::admin::delete_user),
        )
        .route(
            "/api/admin/users/list",
            get(crate::handlers::admin::list_users),
        )
        .route(
            "/api/admin/users/{id}/api-keys",
            get(crate::handlers::admin::list_user_api_keys),
        )
        .route(
            "/api/admin/users/{id}/git-tracking-upload",
            put(crate::handlers::admin::update_git_tracking_upload_authorization),
        )
        .route(
            "/api/admin/users/git-tracking-upload/authorize",
            post(crate::handlers::admin::bulk_authorize_git_tracking_upload),
        )
        // Admin: Organizations
        .route(
            "/api/admin/organizations",
            post(crate::handlers::admin::create_organization),
        )
        .route(
            "/api/admin/organizations/{id}",
            get(crate::handlers::admin::get_organization)
                .delete(crate::handlers::admin::delete_organization),
        )
        .route(
            "/api/admin/organizations/list",
            get(crate::handlers::admin::list_organizations),
        )
        // Admin: Departments
        .route(
            "/api/admin/departments",
            post(crate::handlers::admin::create_department)
                .get(crate::handlers::admin::list_departments),
        )
        .route(
            "/api/admin/departments/{id}",
            delete(crate::handlers::admin::delete_department),
        )
        // Admin: API Keys
        .route(
            "/api/admin/api-keys",
            post(crate::handlers::admin::create_api_key).get(crate::handlers::admin::list_api_keys),
        )
        .route(
            "/api/admin/api-keys/{id}",
            delete(crate::handlers::admin::revoke_api_key),
        )
        // Admin: Install nonces
        .route(
            "/api/admin/install-nonces",
            post(crate::handlers::admin::generate_install_nonce),
        )
        // Admin: Release management
        .route(
            "/api/admin/releases/channel",
            post(crate::handlers::release::update_release_channel),
        )
        .route(
            "/api/admin/releases/upload",
            post(crate::handlers::release::upload_release_asset)
                .layer(DefaultBodyLimit::max(100 * 1024 * 1024)),
        )
        .route(
            "/api/admin/releases/publish",
            post(crate::handlers::release::publish_release_bundle)
                .layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .route(
            "/api/admin/releases/assets",
            get(crate::handlers::release::list_release_assets),
        )
        .route(
            "/api/admin/releases/assets/{channel}/{filename}",
            delete(crate::handlers::release::delete_release_asset),
        )
        // Admin: General file publishing
        .route(
            "/api/admin/files",
            get(crate::handlers::managed_files::list_managed_files),
        )
        .route(
            "/api/admin/files/upload",
            post(crate::handlers::managed_files::upload_managed_file)
                .layer(DefaultBodyLimit::max(512 * 1024 * 1024)),
        )
        .route(
            "/api/admin/files/{slug}",
            put(crate::handlers::managed_files::update_managed_file),
        )
        .route(
            "/api/admin/files/{slug}/publish",
            post(crate::handlers::managed_files::publish_managed_file),
        )
        .route(
            "/api/admin/files/{slug}/versions/{version}",
            delete(crate::handlers::managed_files::delete_managed_file_version),
        )
        // Admin: Repository access rules (whitelist/blacklist)
        .route(
            "/api/admin/repo-access-rules",
            post(crate::handlers::admin::create_repo_access_rule),
        )
        .route(
            "/api/admin/repo-access-rules/list",
            get(crate::handlers::admin::list_repo_access_rules),
        )
        .route(
            "/api/admin/repo-access-rules/{id}",
            delete(crate::handlers::admin::delete_repo_access_rule),
        )
        // Admin: Feature Flags management
        .route(
            "/api/admin/feature-flags",
            post(crate::handlers::admin::upsert_feature_flag)
                .get(crate::handlers::admin::list_feature_flags),
        )
        .route(
            "/api/admin/feature-flags/{key}",
            delete(crate::handlers::admin::delete_feature_flag),
        )
        // Admin: Data export
        .route(
            "/api/admin/export",
            post(crate::handlers::admin::create_export),
        )
        .route(
            "/api/admin/export/{id}",
            get(crate::handlers::admin::get_export),
        )
        // Admin: Data retention policies (Phase 6)
        .route(
            "/api/admin/retention-policies",
            put(crate::handlers::admin::upsert_retention_policy)
                .get(crate::handlers::admin::get_retention_policy),
        )
        .route(
            "/api/admin/purge-expired-data",
            post(crate::handlers::admin::purge_expired_data),
        )
        // Admin: CAS access log (Phase 6)
        .route(
            "/api/admin/cas-access-log",
            get(crate::handlers::admin::list_cas_access_log),
        )
        // Phase 6: PR-level aggregation
        .route(
            "/api/v1/aggregate/pull-requests",
            get(crate::handlers::lifecycle::aggregate_pull_requests),
        )
        .route(
            "/api/v1/pull-requests",
            post(crate::handlers::lifecycle::create_pull_request),
        )
        // Phase 6: AI code persistence
        .route(
            "/api/v1/ai-code-persistence",
            get(crate::handlers::lifecycle::get_ai_code_persistence),
        )
        // Phase 6: Agent readiness
        .route(
            "/api/v1/agent-readiness",
            get(crate::handlers::lifecycle::get_agent_readiness),
        )
        // Phase 6: AI code lifecycle
        .route(
            "/api/v1/ai-code-lifecycle",
            get(crate::handlers::lifecycle::get_ai_code_lifecycle),
        )
        // Phase 6: CI/CD events
        .route(
            "/api/v1/ci-events",
            post(crate::handlers::ci_events::create_ci_event),
        )
        // Phase 6: Alert events
        .route(
            "/api/v1/alert-events",
            post(crate::handlers::ci_events::create_alert_event),
        )
        // Phase 6: Advanced dashboard enhancements
        .route(
            "/api/v1/aggregate/trends",
            get(crate::handlers::dashboard::aggregate_trends),
        )
        .route(
            "/api/v1/aggregate/agent-comparison",
            get(crate::handlers::dashboard::aggregate_agent_comparison),
        )
        .route(
            "/api/v1/aggregate/team-comparison",
            get(crate::handlers::dashboard::aggregate_team_comparison),
        )
        // Middleware
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::services::rate_limit::rate_limit_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(compression)
        // Keep the request span outside TraceLayer so all downstream logs share its request ID.
        .layer(middleware::from_fn(request_id_middleware))
        .layer(cors)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::auth::middleware::browser_security_middleware,
        ))
        .with_state(state)
}
