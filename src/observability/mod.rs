use std::collections::HashMap;
use std::time::Duration;

use crate::metrics::MetricEvent;

pub mod wrapper_performance_targets;

/// Maximum events per metrics envelope
pub const MAX_METRICS_PER_ENVELOPE: usize = 250;

/// Submit telemetry envelopes via the best available path:
/// 1. External daemon control socket (wrapper processes)
/// 2. In-process daemon telemetry worker (daemon process itself)
/// 3. Persist metrics locally and attempt a direct upload when neither daemon
///    route is available
fn submit_telemetry_envelope(
    envelopes: Vec<crate::daemon::TelemetryEnvelope>,
) -> Option<crate::daemon::telemetry_worker::MetricsUploadResult> {
    if crate::daemon::telemetry_handle::daemon_telemetry_available() {
        match crate::daemon::telemetry_handle::submit_telemetry(envelopes) {
            Ok(()) => None,
            Err((error, envelopes)) => {
                tracing::warn!(
                    %error,
                    "telemetry: daemon submission failed; using durable local fallback"
                );
                crate::daemon::telemetry_worker::submit_local_telemetry(envelopes)
            }
        }
    } else if crate::daemon::daemon_process_active() {
        let fallback = envelopes.clone();
        if crate::daemon::telemetry_worker::submit_daemon_internal_telemetry(envelopes) {
            None
        } else {
            crate::daemon::telemetry_worker::submit_local_telemetry(fallback)
        }
    } else {
        crate::daemon::telemetry_worker::submit_local_telemetry(envelopes)
    }
}

/// Log an error to Sentry (via daemon telemetry worker)
pub fn log_error(error: &dyn std::error::Error, context: Option<serde_json::Value>) {
    let envelope = crate::daemon::TelemetryEnvelope::Error {
        timestamp: chrono::Utc::now().to_rfc3339(),
        message: error.to_string(),
        context,
    };
    let _ = submit_telemetry_envelope(vec![envelope]);
}

/// Log a performance metric to Sentry (via daemon telemetry worker)
pub fn log_performance(
    operation: &str,
    duration: Duration,
    context: Option<serde_json::Value>,
    tags: Option<HashMap<String, String>>,
) {
    let envelope = crate::daemon::TelemetryEnvelope::Performance {
        timestamp: chrono::Utc::now().to_rfc3339(),
        operation: operation.to_string(),
        duration_ms: duration.as_millis(),
        context,
        tags,
    };
    let _ = submit_telemetry_envelope(vec![envelope]);
}

/// Log a message to Sentry (info, warning, etc.) (via daemon telemetry worker)
#[allow(dead_code)]
pub fn log_message(message: &str, level: &str, context: Option<serde_json::Value>) {
    let envelope = crate::daemon::TelemetryEnvelope::Message {
        timestamp: chrono::Utc::now().to_rfc3339(),
        message: message.to_string(),
        level: level.to_string(),
        context,
    };
    let _ = submit_telemetry_envelope(vec![envelope]);
}

/// Log a batch of metric events (via daemon telemetry worker).
///
/// Events are batched into envelopes of up to 250 events each.
pub fn log_metrics(
    events: Vec<MetricEvent>,
) -> Option<crate::daemon::telemetry_worker::MetricsUploadResult> {
    #[cfg(any(test, feature = "test-support"))]
    std::env::var_os("GIT_AI_TEST_ENABLE_TELEMETRY")?;

    if events.is_empty() {
        return None;
    }

    let mut result: Option<crate::daemon::telemetry_worker::MetricsUploadResult> = None;

    // Split into chunks of MAX_METRICS_PER_ENVELOPE
    for chunk in events.chunks(MAX_METRICS_PER_ENVELOPE) {
        let envelope = crate::daemon::TelemetryEnvelope::Metrics {
            events: chunk.to_vec(),
        };
        if let Some(chunk_result) = submit_telemetry_envelope(vec![envelope]) {
            if let Some(result) = result.as_mut() {
                result.merge(chunk_result);
            } else {
                result = Some(chunk_result);
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Duration;

    // Test error logging
    #[test]
    fn test_log_error_no_panic() {
        use std::io;
        let error = io::Error::new(io::ErrorKind::NotFound, "test error");
        log_error(&error, None);
    }

    #[test]
    fn test_log_error_with_context() {
        use serde_json::json;
        use std::io;
        let error = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
        let context = json!({"file": "test.txt", "operation": "read"});
        log_error(&error, Some(context));
    }

    // Test performance logging
    #[test]
    fn test_log_performance_basic() {
        log_performance("test_operation", Duration::from_millis(100), None, None);
    }

    #[test]
    fn test_log_performance_with_context() {
        use serde_json::json;
        let context = json!({"files": 5, "lines": 100});
        log_performance("test_op", Duration::from_secs(1), Some(context), None);
    }

    #[test]
    fn test_log_performance_with_tags() {
        let mut tags = HashMap::new();
        tags.insert("command".to_string(), "commit".to_string());
        tags.insert("repo".to_string(), "test".to_string());
        log_performance("commit_op", Duration::from_millis(500), None, Some(tags));
    }

    // Test message logging
    #[test]
    fn test_log_message_basic() {
        log_message("test message", "info", None);
    }

    #[test]
    fn test_log_message_with_context() {
        use serde_json::json;
        let context = json!({"user": "test", "action": "login"});
        log_message("user logged in", "info", Some(context));
    }

    #[test]
    fn test_log_message_warning() {
        log_message("warning message", "warning", None);
    }

    // Test metrics logging
    #[test]
    fn test_log_metrics_empty() {
        log_metrics(vec![]);
    }

    // Test constants
    #[test]
    fn test_max_metrics_per_envelope() {
        assert_eq!(MAX_METRICS_PER_ENVELOPE, 250);
    }
}
