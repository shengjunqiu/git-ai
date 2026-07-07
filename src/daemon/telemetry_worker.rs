//! Daemon-side telemetry worker that batches and dispatches events.
//!
//! Runs inside the daemon process using tokio. Accumulates telemetry envelopes
//! and CAS payloads, then flushes them to their destinations every 3 seconds.

use crate::api::{ApiClient, ApiContext, CasObject, CasUploadRequest, upload_metrics_with_retry};
use crate::config::{Config, get_or_create_distinct_id};
use crate::daemon::control_api::{CasSyncPayload, TelemetryEnvelope};
use crate::metrics::db::MetricsDatabase;
use crate::metrics::{MetricEvent, MetricsBatch};
use crate::observability::MAX_METRICS_PER_ENVELOPE;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{Duration, interval};

const FLUSH_INTERVAL: Duration = Duration::from_secs(3);

/// Accumulated telemetry events waiting to be flushed.
struct TelemetryBuffer {
    errors: Vec<ErrorEvent>,
    performances: Vec<PerformanceEvent>,
    messages: Vec<MessageEvent>,
    metrics: Vec<MetricEvent>,
    cas_records: Vec<CasSyncPayload>,
}

struct ErrorEvent {
    timestamp: String,
    message: String,
    context: Option<Value>,
}

struct PerformanceEvent {
    timestamp: String,
    operation: String,
    duration_ms: u128,
    context: Option<Value>,
    tags: Option<std::collections::HashMap<String, String>>,
}

struct MessageEvent {
    timestamp: String,
    message: String,
    level: String,
    context: Option<Value>,
}

impl TelemetryBuffer {
    fn new() -> Self {
        Self {
            errors: Vec::new(),
            performances: Vec::new(),
            messages: Vec::new(),
            metrics: Vec::new(),
            cas_records: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.errors.is_empty()
            && self.performances.is_empty()
            && self.messages.is_empty()
            && self.metrics.is_empty()
            && self.cas_records.is_empty()
    }

    fn ingest_envelopes(&mut self, envelopes: Vec<TelemetryEnvelope>) {
        for envelope in envelopes {
            match envelope {
                TelemetryEnvelope::Error {
                    timestamp,
                    message,
                    context,
                } => {
                    self.errors.push(ErrorEvent {
                        timestamp,
                        message,
                        context,
                    });
                }
                TelemetryEnvelope::Performance {
                    timestamp,
                    operation,
                    duration_ms,
                    context,
                    tags,
                } => {
                    self.performances.push(PerformanceEvent {
                        timestamp,
                        operation,
                        duration_ms,
                        context,
                        tags,
                    });
                }
                TelemetryEnvelope::Message {
                    timestamp,
                    message,
                    level,
                    context,
                } => {
                    self.messages.push(MessageEvent {
                        timestamp,
                        message,
                        level,
                        context,
                    });
                }
                TelemetryEnvelope::Metrics { events } => {
                    self.metrics.extend(events);
                }
            }
        }
    }

    fn ingest_cas(&mut self, records: Vec<CasSyncPayload>) {
        self.cas_records.extend(records);
    }

    fn take(&mut self) -> TelemetryBuffer {
        TelemetryBuffer {
            errors: std::mem::take(&mut self.errors),
            performances: std::mem::take(&mut self.performances),
            messages: std::mem::take(&mut self.messages),
            metrics: std::mem::take(&mut self.metrics),
            cas_records: std::mem::take(&mut self.cas_records),
        }
    }
}

/// Handle for submitting telemetry directly within the daemon process.
#[derive(Clone)]
pub struct DaemonTelemetryWorkerHandle {
    buffer: Arc<Mutex<TelemetryBuffer>>,
}

impl DaemonTelemetryWorkerHandle {
    /// Submit telemetry envelopes for batched processing.
    pub async fn submit_telemetry(&self, envelopes: Vec<TelemetryEnvelope>) {
        self.buffer.lock().await.ingest_envelopes(envelopes);
    }

    /// Submit CAS records for batched upload.
    pub async fn submit_cas(&self, records: Vec<CasSyncPayload>) {
        self.buffer.lock().await.ingest_cas(records);
    }

    /// Submit telemetry envelopes synchronously (best-effort, non-blocking).
    ///
    /// Used by the daemon process's own `observability::log_*()` calls which
    /// cannot go through the control socket (the daemon can't connect to itself).
    /// Uses `try_lock()` to avoid blocking the caller if the buffer is contested.
    pub fn submit_telemetry_sync(&self, envelopes: Vec<TelemetryEnvelope>) {
        if let Ok(mut buf) = self.buffer.try_lock() {
            buf.ingest_envelopes(envelopes);
        }
    }

    /// Submit CAS records synchronously (best-effort, non-blocking).
    ///
    /// Used by daemon-owned post-commit paths that cannot route through the
    /// control socket because the daemon cannot connect to itself.
    pub fn submit_cas_sync(&self, records: Vec<CasSyncPayload>) {
        if let Ok(mut buf) = self.buffer.try_lock() {
            buf.ingest_cas(records);
        }
    }
}

/// Global handle for the daemon's in-process telemetry worker.
///
/// Set once when the daemon spawns its telemetry worker, allowing
/// `observability::log_*()` functions to route events directly into
/// the worker buffer when running inside the daemon process.
static DAEMON_INTERNAL_TELEMETRY: std::sync::OnceLock<DaemonTelemetryWorkerHandle> =
    std::sync::OnceLock::new();

/// Register the daemon's in-process telemetry worker handle.
/// Called once during daemon startup after `spawn_telemetry_worker()`.
pub fn set_daemon_internal_telemetry(handle: DaemonTelemetryWorkerHandle) {
    let _ = DAEMON_INTERNAL_TELEMETRY.set(handle);
}

/// Submit telemetry from within the daemon process (sync, best-effort).
/// Returns true if the handle was available and envelopes were submitted.
pub fn submit_daemon_internal_telemetry(envelopes: Vec<TelemetryEnvelope>) -> bool {
    if let Some(handle) = DAEMON_INTERNAL_TELEMETRY.get() {
        handle.submit_telemetry_sync(envelopes);
        true
    } else {
        false
    }
}

/// Submit CAS records from within the daemon process (sync, best-effort).
/// Returns true if the handle was available and records were submitted.
pub fn submit_daemon_internal_cas(records: Vec<CasSyncPayload>) -> bool {
    if let Some(handle) = DAEMON_INTERNAL_TELEMETRY.get() {
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let handle = handle.clone();
            runtime.spawn(async move {
                handle.submit_cas(records).await;
            });
        } else {
            handle.submit_cas_sync(records);
        }
        true
    } else {
        false
    }
}

/// Spawn the telemetry worker task. Returns a handle for submitting events.
///
/// The worker runs a flush loop every 3 seconds, sending accumulated events
/// to their respective destinations (Sentry, PostHog, metrics API, CAS API).
pub fn spawn_telemetry_worker() -> DaemonTelemetryWorkerHandle {
    let buffer = Arc::new(Mutex::new(TelemetryBuffer::new()));
    let handle = DaemonTelemetryWorkerHandle {
        buffer: buffer.clone(),
    };

    tokio::spawn(async move {
        telemetry_flush_loop(buffer).await;
    });

    handle
}

async fn telemetry_flush_loop(buffer: Arc<Mutex<TelemetryBuffer>>) {
    let mut ticker = interval(FLUSH_INTERVAL);
    // The first tick completes immediately; skip it.
    ticker.tick().await;

    loop {
        ticker.tick().await;

        let snapshot = {
            let mut buf = buffer.lock().await;
            if buf.is_empty() {
                continue;
            }
            buf.take()
        };

        // Flush in a blocking task since the underlying HTTP clients are synchronous.
        tokio::task::spawn_blocking(move || {
            flush_telemetry_batch(snapshot);
        })
        .await
        .unwrap_or_else(|e| {
            tracing::error!(%e, "telemetry flush task panicked");
        });
    }
}

fn flush_telemetry_batch(batch: TelemetryBuffer) {
    let config = Config::get();
    let distinct_id = get_or_create_distinct_id();

    // Flush metrics (always processed — uploaded or stored in SQLite)
    if !batch.metrics.is_empty() {
        flush_metrics(&batch.metrics);
    }

    // Flush Sentry events (errors, performance, messages)
    let has_sentry_or_posthog =
        !batch.errors.is_empty() || !batch.performances.is_empty() || !batch.messages.is_empty();

    if has_sentry_or_posthog {
        flush_sentry_and_posthog(
            config,
            &distinct_id,
            &batch.errors,
            &batch.performances,
            &batch.messages,
        );
    }

    // Flush CAS records
    if !batch.cas_records.is_empty() {
        flush_cas(batch.cas_records);
    }
}

fn flush_metrics(events: &[MetricEvent]) {
    let context = ApiContext::new(None);
    let api_base_url = context.base_url.clone();
    let client = ApiClient::new(context);

    let using_default_api = api_base_url == crate::config::DEFAULT_API_BASE_URL;
    let should_upload = !using_default_api || client.is_logged_in() || client.has_api_key();

    if !should_upload && !events.is_empty() {
        tracing::warn!(
            event_count = events.len(),
            api_base_url = %api_base_url,
            "telemetry: metrics upload skipped — not logged in and no API key configured. \
             Set GIT_AI_API_KEY or run `git-ai login` to enable automatic upload, \
             or set GIT_AI_API_BASE_URL to your enterprise server."
        );
    }

    for chunk in events.chunks(MAX_METRICS_PER_ENVELOPE) {
        let batch = MetricsBatch::new(chunk.to_vec());
        if should_upload {
            match upload_metrics_with_retry(&client, &batch, "daemon_telemetry") {
                Ok(()) => continue,
                Err(e) => {
                    tracing::warn!(%e, "telemetry: metrics upload failed, storing in local DB");
                    store_metrics_in_db(chunk);
                    continue;
                }
            }
        }
        store_metrics_in_db(chunk);
    }
}

fn store_metrics_in_db(events: &[MetricEvent]) {
    if events.is_empty() {
        return;
    }

    let event_jsons: Vec<String> = events
        .iter()
        .filter_map(|e| serde_json::to_string(e).ok())
        .collect();

    if event_jsons.is_empty() {
        return;
    }

    if let Ok(db) = MetricsDatabase::global()
        && let Ok(mut db_lock) = db.lock()
    {
        let _ = db_lock.insert_events(&event_jsons);
    }
}

fn flush_sentry_and_posthog(
    config: &Config,
    distinct_id: &str,
    errors: &[ErrorEvent],
    performances: &[PerformanceEvent],
    messages: &[MessageEvent],
) {
    // Check for Enterprise DSN
    let enterprise_dsn = config
        .telemetry_enterprise_dsn()
        .map(|s| s.to_string())
        .or_else(|| {
            std::env::var("SENTRY_ENTERPRISE")
                .ok()
                .or_else(|| option_env!("SENTRY_ENTERPRISE").map(|s| s.to_string()))
                .filter(|s| !s.is_empty())
        });

    // Check for OSS DSN
    let oss_dsn = if config.is_telemetry_oss_disabled() {
        None
    } else {
        std::env::var("SENTRY_OSS")
            .ok()
            .or_else(|| option_env!("SENTRY_OSS").map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
    };

    // Check for PostHog configuration
    let posthog_api_key = if config.is_telemetry_oss_disabled() {
        None
    } else {
        std::env::var("POSTHOG_API_KEY")
            .ok()
            .or_else(|| option_env!("POSTHOG_API_KEY").map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
    };

    let posthog_host = std::env::var("POSTHOG_HOST")
        .ok()
        .or_else(|| option_env!("POSTHOG_HOST").map(|s| s.to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://us.i.posthog.com".to_string());

    // Build Sentry clients
    let oss_client = oss_dsn.and_then(|dsn| SentryClient::from_dsn(&dsn));
    let enterprise_client = enterprise_dsn.and_then(|dsn| SentryClient::from_dsn(&dsn));

    // Build base tags
    let mut base_tags = BTreeMap::new();
    base_tags.insert("os".to_string(), json!(std::env::consts::OS));
    base_tags.insert("arch".to_string(), json!(std::env::consts::ARCH));
    base_tags.insert("distinct_id".to_string(), json!(distinct_id));

    // Send errors
    for error in errors {
        let mut extra = BTreeMap::new();
        if let Some(ctx) = &error.context
            && let Some(obj) = ctx.as_object()
        {
            for (key, value) in obj {
                extra.insert(key.clone(), value.clone());
            }
        }

        let event = json!({
            "message": error.message,
            "level": "error",
            "timestamp": error.timestamp,
            "platform": "other",
            "tags": base_tags,
            "extra": extra,
            "release": format!("git-ai@{}", env!("CARGO_PKG_VERSION")),
        });

        if let Some(client) = &oss_client {
            let _ = client.send_event(event.clone());
        }
        if let Some(client) = &enterprise_client {
            let _ = client.send_event(event);
        }
    }

    // Send performance events
    for perf in performances {
        let mut extra = BTreeMap::new();
        extra.insert("operation".to_string(), json!(perf.operation));
        extra.insert("duration_ms".to_string(), json!(perf.duration_ms));
        if let Some(ctx) = &perf.context
            && let Some(obj) = ctx.as_object()
        {
            for (key, value) in obj {
                extra.insert(key.clone(), value.clone());
            }
        }

        let mut perf_tags = base_tags.clone();
        if let Some(tags) = &perf.tags {
            for (key, value) in tags {
                perf_tags.insert(key.clone(), json!(value));
            }
        }

        let event = json!({
            "message": format!("Performance: {} ({}ms)", perf.operation, perf.duration_ms),
            "level": "info",
            "timestamp": perf.timestamp,
            "platform": "other",
            "tags": perf_tags,
            "extra": extra,
            "release": format!("git-ai@{}", env!("CARGO_PKG_VERSION")),
        });

        if let Some(client) = &oss_client {
            let _ = client.send_event(event.clone());
        }
        if let Some(client) = &enterprise_client {
            let _ = client.send_event(event);
        }
    }

    // Send messages (to Sentry + PostHog)
    for msg in messages {
        let mut extra = BTreeMap::new();
        if let Some(ctx) = &msg.context
            && let Some(obj) = ctx.as_object()
        {
            for (key, value) in obj {
                extra.insert(key.clone(), value.clone());
            }
        }

        let sentry_event = json!({
            "message": msg.message,
            "level": msg.level,
            "timestamp": msg.timestamp,
            "platform": "other",
            "tags": base_tags,
            "extra": extra,
            "release": format!("git-ai@{}", env!("CARGO_PKG_VERSION")),
        });

        if let Some(client) = &oss_client {
            let _ = client.send_event(sentry_event.clone());
        }
        if let Some(client) = &enterprise_client {
            let _ = client.send_event(sentry_event);
        }

        // PostHog only gets messages
        if let Some(api_key) = &posthog_api_key {
            let mut properties = BTreeMap::new();
            properties.insert("os".to_string(), json!(std::env::consts::OS));
            properties.insert("arch".to_string(), json!(std::env::consts::ARCH));
            properties.insert("version".to_string(), json!(env!("CARGO_PKG_VERSION")));
            properties.insert("message".to_string(), json!(msg.message));
            properties.insert("level".to_string(), json!(msg.level));
            if let Some(ctx) = &msg.context
                && let Some(obj) = ctx.as_object()
            {
                for (key, value) in obj {
                    properties.insert(key.clone(), value.clone());
                }
            }

            let endpoint = format!("{}/capture/", posthog_host.trim_end_matches('/'));
            let mut ph_event = json!({
                "api_key": api_key,
                "event": msg.message,
                "properties": properties,
                "distinct_id": distinct_id,
            });
            ph_event["timestamp"] = json!(msg.timestamp);

            let agent = crate::http::build_agent(Some(30));
            let request = agent
                .post(&endpoint)
                .set("Content-Type", "application/json");
            let _ = crate::http::send_with_body(
                request,
                &serde_json::to_string(&ph_event).unwrap_or_default(),
            );
        }
    }
}

fn flush_cas(records: Vec<CasSyncPayload>) {
    let context = ApiContext::new(None);
    let api_base_url = context.base_url.clone();
    let client = ApiClient::new(context);

    let using_default_api = api_base_url == crate::config::DEFAULT_API_BASE_URL;
    if using_default_api && !client.is_logged_in() && !client.has_api_key() {
        if !records.is_empty() {
            tracing::warn!(
                record_count = records.len(),
                api_base_url = %api_base_url,
                "telemetry: CAS upload skipped — not logged in and no API key configured. \
                 Set GIT_AI_API_KEY or run `git-ai login` to enable automatic upload, \
                 or set GIT_AI_API_BASE_URL to your enterprise server."
            );
        }
        return;
    }

    // Build upload request
    let mut cas_objects = Vec::new();
    for record in &records {
        let content: Value = match serde_json::from_str(&record.data) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(%e, "telemetry: CAS parse error");
                continue;
            }
        };
        // Convert serialized JSON metadata string to HashMap
        let metadata = record
            .metadata
            .as_ref()
            .and_then(|m| serde_json::from_str::<std::collections::HashMap<String, String>>(m).ok())
            .unwrap_or_default();
        cas_objects.push(CasObject {
            content,
            hash: record.hash.clone(),
            metadata,
        });
    }

    if cas_objects.is_empty() {
        return;
    }

    for chunk in cas_objects.chunks(50) {
        let hashes: Vec<String> = chunk.iter().map(|o| o.hash.clone()).collect();
        let request = CasUploadRequest {
            objects: chunk.to_vec(),
        };
        match client.upload_cas(request) {
            Ok(_response) => {
                // Delete successfully uploaded records from the internal DB queue
                // so they don't accumulate as stale entries.
                if let Ok(db) = crate::authorship::internal_db::InternalDatabase::global()
                    && let Ok(mut db_lock) = db.lock()
                {
                    let _ = db_lock.delete_cas_by_hashes(&hashes);
                }
                tracing::info!(count = chunk.len(), "telemetry: uploaded CAS objects");
            }
            Err(e) => {
                tracing::warn!(%e, count = chunk.len(), "telemetry: CAS upload error — records will be retried on next flush");
            }
        }
    }
}

/// Minimal Sentry client (mirrors flush.rs SentryClient)
struct SentryClient {
    endpoint: String,
    public_key: String,
}

impl SentryClient {
    fn from_dsn(dsn: &str) -> Option<Self> {
        let url = url::Url::parse(dsn).ok()?;
        let public_key = url.username().to_string();
        let host = url.host_str()?;
        let project_id = url.path().trim_start_matches('/');
        let scheme = url.scheme();
        let endpoint = format!("{}://{}/api/{}/store/", scheme, host, project_id);
        Some(SentryClient {
            endpoint,
            public_key,
        })
    }

    fn send_event(&self, event: Value) -> Result<(), Box<dyn std::error::Error>> {
        let auth_header = format!(
            "Sentry sentry_version=7, sentry_key={}, sentry_client=git-ai/{}",
            self.public_key,
            env!("CARGO_PKG_VERSION")
        );

        let body = serde_json::to_string(&event)?;
        let agent = crate::http::build_agent(Some(30));
        let request = agent
            .post(&self.endpoint)
            .set("X-Sentry-Auth", &auth_header)
            .set("Content-Type", "application/json");
        let response = crate::http::send_with_body(request, &body)?;

        let status = response.status_code;
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(format!("Sentry returned status {}", status).into())
        }
    }
}

/// Determine whether telemetry (metrics + CAS) should be uploaded based on
/// the current API configuration and authentication state.
///
/// Uploads are skipped when ALL of the following are true:
/// 1. The API base URL is the default (`https://usegitai.com`)
/// 2. The user is not logged in (no OAuth token)
/// 3. No API key is configured
///
/// In this state, metrics are stored in a local SQLite database instead of
/// being uploaded, and CAS records remain in the internal DB for later retry.
pub fn should_upload_telemetry() -> bool {
    let context = ApiContext::new(None);
    let api_base_url = context.base_url.clone();
    let client = ApiClient::new(context);

    let using_default_api = api_base_url == crate::config::DEFAULT_API_BASE_URL;
    !using_default_api || client.is_logged_in() || client.has_api_key()
}

/// Diagnostic information about why telemetry upload may be skipped.
#[derive(Debug, Clone)]
pub struct TelemetryUploadDiagnostics {
    pub api_base_url: String,
    pub using_default_api: bool,
    pub is_logged_in: bool,
    pub has_api_key: bool,
    pub should_upload: bool,
}

impl std::fmt::Display for TelemetryUploadDiagnostics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TelemetryUploadDiagnostics(api_base_url={}, using_default_api={}, is_logged_in={}, has_api_key={}, should_upload={})",
            self.api_base_url,
            self.using_default_api,
            self.is_logged_in,
            self.has_api_key,
            self.should_upload
        )
    }
}

/// Get detailed diagnostics about the telemetry upload gating decision.
/// Useful for debugging why automatic uploads are not working.
pub fn telemetry_upload_diagnostics() -> TelemetryUploadDiagnostics {
    let context = ApiContext::new(None);
    let api_base_url = context.base_url.clone();
    let is_logged_in = context.auth_token.is_some();
    let has_api_key = context.api_key.is_some();

    let using_default_api = api_base_url == crate::config::DEFAULT_API_BASE_URL;
    let should_upload = !using_default_api || is_logged_in || has_api_key;

    TelemetryUploadDiagnostics {
        api_base_url,
        using_default_api,
        is_logged_in,
        has_api_key,
        should_upload,
    }
}

/// Print a concise post-commit upload status for user-facing CLI output.
///
/// This intentionally reports queue/auth/daemon state, not "uploaded successfully":
/// the actual network upload is asynchronous and may still retry in the background.
pub fn print_commit_upload_notice() {
    let diag = telemetry_upload_diagnostics();
    let daemon_available = crate::daemon::daemon_process_active()
        || crate::daemon::telemetry_handle::daemon_telemetry_available();
    eprintln!("{}", commit_upload_notice_message(&diag, daemon_available));
}

fn commit_upload_notice_message(
    diag: &TelemetryUploadDiagnostics,
    daemon_available: bool,
) -> String {
    // The enterprise metrics endpoint is authenticated; a custom server URL alone is
    // not enough to give the user a meaningful "queued for upload" confirmation.
    if !diag.is_logged_in && !diag.has_api_key {
        let login_target = if diag.using_default_api {
            "<enterprise-server-url>"
        } else {
            diag.api_base_url.as_str()
        };
        return format!(
            "[git-ai] AI tracking saved locally. Upload not enabled: run `git-ai login --server {}` or set GIT_AI_API_KEY.",
            login_target
        );
    }

    if !daemon_available {
        return "[git-ai] AI tracking saved locally. Upload not queued: background daemon is not running. Run `git-ai install-hooks` to restart it.".to_string();
    }

    format!(
        "[git-ai] AI tracking upload queued to {}.",
        diag.api_base_url
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::client::{ApiClient, ApiContext};
    use crate::config::DEFAULT_API_BASE_URL;

    fn test_diag(
        api_base_url: &str,
        using_default_api: bool,
        is_logged_in: bool,
        has_api_key: bool,
    ) -> TelemetryUploadDiagnostics {
        TelemetryUploadDiagnostics {
            api_base_url: api_base_url.to_string(),
            using_default_api,
            is_logged_in,
            has_api_key,
            should_upload: !using_default_api || is_logged_in || has_api_key,
        }
    }

    #[test]
    fn test_commit_upload_notice_queued_when_authenticated_and_daemon_available() {
        let diag = test_diag("https://enterprise.example.com", false, true, false);
        let message = commit_upload_notice_message(&diag, true);
        assert_eq!(
            message,
            "[git-ai] AI tracking upload queued to https://enterprise.example.com."
        );
    }

    #[test]
    fn test_commit_upload_notice_requests_login_for_enterprise_url() {
        let diag = test_diag("https://enterprise.example.com", false, false, false);
        let message = commit_upload_notice_message(&diag, true);
        assert_eq!(
            message,
            "[git-ai] AI tracking saved locally. Upload not enabled: run `git-ai login --server https://enterprise.example.com` or set GIT_AI_API_KEY."
        );
    }

    #[test]
    fn test_commit_upload_notice_requests_server_for_default_url() {
        let diag = test_diag(DEFAULT_API_BASE_URL, true, false, false);
        let message = commit_upload_notice_message(&diag, true);
        assert_eq!(
            message,
            "[git-ai] AI tracking saved locally. Upload not enabled: run `git-ai login --server <enterprise-server-url>` or set GIT_AI_API_KEY."
        );
    }

    #[test]
    fn test_commit_upload_notice_reports_missing_daemon_after_auth() {
        let diag = test_diag("https://enterprise.example.com", false, true, false);
        let message = commit_upload_notice_message(&diag, false);
        assert_eq!(
            message,
            "[git-ai] AI tracking saved locally. Upload not queued: background daemon is not running. Run `git-ai install-hooks` to restart it."
        );
    }

    // ==================== Upload Gating Logic Tests ====================

    /// Test that the default API URL is recognized as "default"
    #[test]
    fn test_default_api_url_is_recognized() {
        assert_eq!(DEFAULT_API_BASE_URL, "https://usegitai.com");
    }

    /// Test that ApiContext without auth has no token when using the default base URL.
    /// Note: api_key may come from the config file, so we only assert auth_token.
    #[test]
    fn test_api_context_default_state() {
        let ctx = ApiContext::without_auth(Some(DEFAULT_API_BASE_URL.to_string()));
        assert_eq!(ctx.base_url, DEFAULT_API_BASE_URL);
        assert!(
            ctx.auth_token.is_none(),
            "without_auth should have no auth_token"
        );
    }

    /// Test that ApiContext with a custom base URL still has no auth
    /// but is considered "not using default API".
    #[test]
    fn test_api_context_custom_url_no_auth() {
        let custom_url = "https://enterprise.example.com";
        let ctx = ApiContext::without_auth(Some(custom_url.to_string()));
        assert_eq!(ctx.base_url, custom_url);
        assert!(ctx.auth_token.is_none());
        // API key may come from config — we only check URL here
    }

    /// Test that ApiContext with an explicit auth token is "logged in".
    #[test]
    fn test_api_client_is_logged_in_with_token() {
        let ctx = ApiContext::with_auth(
            Some(DEFAULT_API_BASE_URL.to_string()),
            "test_token".to_string(),
        );
        let client = ApiClient::new(ctx);
        assert!(client.is_logged_in());
    }

    /// Test that ApiContext with an API key has the key available.
    #[test]
    fn test_api_client_has_api_key_when_set() {
        let mut ctx = ApiContext::without_auth(Some(DEFAULT_API_BASE_URL.to_string()));
        ctx.api_key = Some("test-api-key".to_string());
        let client = ApiClient::new(ctx);
        assert!(client.has_api_key());
        // auth_token is still None
        assert!(!client.is_logged_in());
    }

    // ==================== Gating Decision Tests ====================

    /// Test: default API + not logged in + no API key => should NOT upload
    /// This tests the gating logic directly, constructing an ApiClient with
    /// explicitly no auth credentials.
    #[test]
    fn test_should_upload_default_api_no_auth() {
        let mut ctx = ApiContext::without_auth(Some(DEFAULT_API_BASE_URL.to_string()));
        // Explicitly clear any API key from config to test the "no auth" case
        ctx.api_key = None;
        ctx.auth_token = None;
        let using_default_api = ctx.base_url == DEFAULT_API_BASE_URL;
        let client = ApiClient::new(ctx);
        let should_upload = !using_default_api || client.is_logged_in() || client.has_api_key();
        assert!(using_default_api);
        assert!(!client.is_logged_in());
        assert!(!client.has_api_key());
        assert!(
            !should_upload,
            "should NOT upload when using default API with no auth"
        );
    }

    /// Test: default API + logged in => should upload
    #[test]
    fn test_should_upload_default_api_logged_in() {
        let ctx =
            ApiContext::with_auth(Some(DEFAULT_API_BASE_URL.to_string()), "token".to_string());
        let using_default_api = ctx.base_url == DEFAULT_API_BASE_URL;
        let client = ApiClient::new(ctx);
        let should_upload = !using_default_api || client.is_logged_in() || client.has_api_key();
        assert!(using_default_api);
        assert!(client.is_logged_in());
        assert!(
            should_upload,
            "should upload when logged in even with default API"
        );
    }

    /// Test: default API + API key => should upload
    #[test]
    fn test_should_upload_default_api_with_api_key() {
        let mut ctx = ApiContext::without_auth(Some(DEFAULT_API_BASE_URL.to_string()));
        ctx.api_key = Some("my-api-key".to_string());
        ctx.auth_token = None;
        let using_default_api = ctx.base_url == DEFAULT_API_BASE_URL;
        let client = ApiClient::new(ctx);
        let should_upload = !using_default_api || client.is_logged_in() || client.has_api_key();
        assert!(using_default_api);
        assert!(client.has_api_key());
        assert!(!client.is_logged_in());
        assert!(
            should_upload,
            "should upload with API key even with default API"
        );
    }

    /// Test: custom API URL + no auth => should upload (enterprise users)
    #[test]
    fn test_should_upload_custom_api_no_auth() {
        let custom_url = "https://enterprise.example.com";
        let mut ctx = ApiContext::without_auth(Some(custom_url.to_string()));
        ctx.api_key = None;
        ctx.auth_token = None;
        let using_default_api = ctx.base_url == DEFAULT_API_BASE_URL;
        let client = ApiClient::new(ctx);
        let should_upload = !using_default_api || client.is_logged_in() || client.has_api_key();
        assert!(!using_default_api);
        assert!(
            should_upload,
            "should upload with custom API URL even without auth"
        );
    }

    /// Test: custom API URL + logged in => should upload
    #[test]
    fn test_should_upload_custom_api_logged_in() {
        let custom_url = "https://enterprise.example.com";
        let ctx = ApiContext::with_auth(Some(custom_url.to_string()), "token".to_string());
        let using_default_api = ctx.base_url == DEFAULT_API_BASE_URL;
        let client = ApiClient::new(ctx);
        let should_upload = !using_default_api || client.is_logged_in() || client.has_api_key();
        assert!(!using_default_api);
        assert!(should_upload);
    }

    /// Test: all three gating conditions are independent — any one being true allows upload.
    #[test]
    fn test_gating_any_condition_allows_upload() {
        // Verify the truth table: should_upload = !using_default_api OR is_logged_in OR has_api_key
        // This is logically equivalent to: !(using_default_api AND !is_logged_in AND !has_api_key)
        // i.e., upload is blocked ONLY when ALL three blocking conditions are true simultaneously.

        // Blocked: default API AND not logged in AND no API key
        let mut ctx = ApiContext::without_auth(Some(DEFAULT_API_BASE_URL.to_string()));
        ctx.api_key = None;
        ctx.auth_token = None;
        let blocked = !using_default_api_or_authenticated(&ctx);
        assert!(blocked, "should be blocked with default API + no auth");

        // Unblocked: custom API
        let mut ctx2 = ApiContext::without_auth(Some("https://custom.example.com".to_string()));
        ctx2.api_key = None;
        ctx2.auth_token = None;
        let unblocked_custom = !using_default_api_or_authenticated(&ctx2);
        assert!(!unblocked_custom, "should NOT be blocked with custom API");

        // Unblocked: logged in
        let mut ctx3 =
            ApiContext::with_auth(Some(DEFAULT_API_BASE_URL.to_string()), "token".to_string());
        ctx3.api_key = None;
        let unblocked_login = !using_default_api_or_authenticated(&ctx3);
        assert!(!unblocked_login, "should NOT be blocked when logged in");

        // Unblocked: has API key
        let mut ctx4 = ApiContext::without_auth(Some(DEFAULT_API_BASE_URL.to_string()));
        ctx4.api_key = Some("key".to_string());
        ctx4.auth_token = None;
        let unblocked_key = !using_default_api_or_authenticated(&ctx4);
        assert!(!unblocked_key, "should NOT be blocked when has API key");
    }

    /// Helper: compute should_upload from an ApiContext reference
    fn using_default_api_or_authenticated(ctx: &ApiContext) -> bool {
        let using_default_api = ctx.base_url == DEFAULT_API_BASE_URL;
        !using_default_api || ctx.auth_token.is_some() || ctx.api_key.is_some()
    }

    // ==================== Diagnostics Tests ====================

    /// Test that telemetry_upload_diagnostics returns a valid diagnostic struct
    #[test]
    fn test_telemetry_upload_diagnostics_structure() {
        let diag = telemetry_upload_diagnostics();
        // The actual values depend on the test environment, but the structure
        // should always be valid and internally consistent.
        assert_eq!(
            diag.using_default_api,
            diag.api_base_url == DEFAULT_API_BASE_URL
        );
        assert_eq!(
            diag.should_upload,
            !diag.using_default_api || diag.is_logged_in || diag.has_api_key
        );
    }

    /// Test that the diagnostics Display implementation works
    #[test]
    fn test_telemetry_upload_diagnostics_display() {
        let diag = TelemetryUploadDiagnostics {
            api_base_url: "https://example.com".to_string(),
            using_default_api: false,
            is_logged_in: true,
            has_api_key: false,
            should_upload: true,
        };
        let display = format!("{}", diag);
        assert!(display.contains("should_upload=true"));
        assert!(display.contains("is_logged_in=true"));
        assert!(display.contains("using_default_api=false"));
    }

    // ==================== Metrics Storage Tests ====================

    /// Test that store_metrics_in_db handles empty events gracefully
    #[test]
    fn test_store_metrics_in_db_empty() {
        // Should not panic with empty events
        store_metrics_in_db(&[]);
    }

    // ==================== TelemetryBuffer Tests ====================

    #[test]
    fn test_telemetry_buffer_new_is_empty() {
        let buf = TelemetryBuffer::new();
        assert!(buf.is_empty());
    }

    #[test]
    fn test_telemetry_buffer_ingest_metrics() {
        let mut buf = TelemetryBuffer::new();
        let events = vec![TelemetryEnvelope::Metrics {
            events: vec![create_test_metric_event()],
        }];
        buf.ingest_envelopes(events);
        assert!(!buf.is_empty());
        assert_eq!(buf.metrics.len(), 1);
    }

    #[test]
    fn test_telemetry_buffer_ingest_cas() {
        let mut buf = TelemetryBuffer::new();
        let records = vec![CasSyncPayload {
            hash: "abc123".to_string(),
            data: r#"{"test": true}"#.to_string(),
            metadata: None,
        }];
        buf.ingest_cas(records);
        assert!(!buf.is_empty());
        assert_eq!(buf.cas_records.len(), 1);
    }

    #[test]
    fn test_telemetry_buffer_take() {
        let mut buf = TelemetryBuffer::new();
        let events = vec![TelemetryEnvelope::Metrics {
            events: vec![create_test_metric_event()],
        }];
        buf.ingest_envelopes(events);

        let taken = buf.take();
        assert!(!taken.is_empty());
        assert!(buf.is_empty(), "original buffer should be empty after take");
        assert_eq!(taken.metrics.len(), 1);
    }

    #[test]
    fn test_telemetry_buffer_ingest_error() {
        let mut buf = TelemetryBuffer::new();
        let events = vec![TelemetryEnvelope::Error {
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            message: "test error".to_string(),
            context: None,
        }];
        buf.ingest_envelopes(events);
        assert!(!buf.is_empty());
        assert_eq!(buf.errors.len(), 1);
    }

    #[test]
    fn test_telemetry_buffer_ingest_performance() {
        let mut buf = TelemetryBuffer::new();
        let events = vec![TelemetryEnvelope::Performance {
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            operation: "test_op".to_string(),
            duration_ms: 100,
            context: None,
            tags: None,
        }];
        buf.ingest_envelopes(events);
        assert!(!buf.is_empty());
        assert_eq!(buf.performances.len(), 1);
    }

    #[test]
    fn test_telemetry_buffer_ingest_message() {
        let mut buf = TelemetryBuffer::new();
        let events = vec![TelemetryEnvelope::Message {
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            message: "test message".to_string(),
            level: "info".to_string(),
            context: None,
        }];
        buf.ingest_envelopes(events);
        assert!(!buf.is_empty());
        assert_eq!(buf.messages.len(), 1);
    }

    /// Helper to create a test MetricEvent
    fn create_test_metric_event() -> MetricEvent {
        use crate::metrics::types::MetricEventId;

        MetricEvent {
            event_id: MetricEventId::Committed as u16,
            timestamp: 1700000000,
            values: std::collections::HashMap::new(),
            attrs: std::collections::HashMap::new(),
        }
    }
}
