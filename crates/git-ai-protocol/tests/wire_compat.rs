use git_ai_protocol::bundle::{ApiFileRecord, BundleData, CreateBundleRequest};
use git_ai_protocol::cas::{CasObject, CasUploadRequest};
use git_ai_protocol::client_status::ClientStatusRequest;
use git_ai_protocol::metrics::{MetricEvent, MetricsBatch};
use git_ai_protocol::oauth::{DeviceCodeResponse, TokenResponse};
use git_ai_protocol::report::ReportDocument;
use serde_json::json;
use std::collections::HashMap;

#[test]
fn metrics_wire_format_keeps_positional_field_names() {
    let batch = MetricsBatch {
        version: 1,
        events: vec![MetricEvent {
            timestamp: 1_700_000_000,
            event_id: 1,
            values: HashMap::from([("0".to_string(), json!(12))]),
            attrs: HashMap::from([("0".to_string(), json!("1.3.2"))]),
        }],
    };

    assert_eq!(
        serde_json::to_value(batch).unwrap(),
        json!({
            "v": 1,
            "events": [{
                "t": 1_700_000_000,
                "e": 1,
                "v": {"0": 12},
                "a": {"0": "1.3.2"}
            }]
        })
    );
}

#[test]
fn cas_and_bundle_keep_existing_omission_rules() {
    let cas = CasUploadRequest {
        objects: vec![CasObject {
            content: json!({"messages": []}),
            hash: "abc123".to_string(),
            metadata: HashMap::new(),
        }],
    };
    assert_eq!(
        serde_json::to_value(cas).unwrap(),
        json!({"objects": [{"content": {"messages": []}, "hash": "abc123"}]})
    );

    let bundle = CreateBundleRequest {
        title: "Example".to_string(),
        data: BundleData {
            prompts: HashMap::from([("prompt-1".to_string(), json!({"messages": []}))]),
            files: HashMap::from([(
                "src/main.rs".to_string(),
                ApiFileRecord {
                    annotations: HashMap::from([(
                        "prompt-1".to_string(),
                        vec![json!(1), json!([3, 5])],
                    )]),
                    diff: Some("+line".to_string()),
                    base_content: Some(String::new()),
                },
            )]),
        },
    };
    let value = serde_json::to_value(bundle).unwrap();
    assert_eq!(value["data"]["files"]["src/main.rs"]["base_content"], "");
    assert_eq!(
        value["data"]["files"]["src/main.rs"]["annotations"]["prompt-1"],
        json!([1, [3, 5]])
    );
}

#[test]
fn oauth_uses_one_unsigned_numeric_contract() {
    let token: TokenResponse = serde_json::from_value(json!({
        "access_token": "access",
        "token_type": "Bearer",
        "expires_in": 3600,
        "refresh_token": "refresh",
        "refresh_expires_in": 7_776_000
    }))
    .unwrap();
    assert_eq!(token.expires_in, 3600);

    let device: DeviceCodeResponse = serde_json::from_value(json!({
        "device_code": "device",
        "user_code": "ABCD-EFGH",
        "verification_uri": "https://example.com/verify",
        "expires_in": 900,
        "interval": 5
    }))
    .unwrap();
    assert_eq!(device.interval, 5);
    assert!(device.verification_uri_complete.is_none());
}

#[test]
fn client_status_accepts_legacy_missing_metadata() {
    let status: ClientStatusRequest =
        serde_json::from_value(json!({"status": "logged_in"})).unwrap();
    assert_eq!(status.status, "logged_in");
    assert!(status.cli_version.is_none());
    assert!(status.hostname.is_none());
}

#[test]
fn report_request_remains_lenient_for_older_clients() {
    let report: ReportDocument = serde_json::from_value(json!({
        "schema_version": "git-ai-report/1.0.0",
        "generated_at": "2026-01-01T00:00:00Z",
        "tool_version": "1.3.2"
    }))
    .unwrap();

    assert!(report.repo.is_none());
    assert!(report.summary.is_none());
    assert!(report.commits.is_empty());
}
