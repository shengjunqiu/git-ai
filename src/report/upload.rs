use crate::api::ApiContext;
use crate::error::GitAiError;
use crate::report::model::{ProjectSummaryReport, ReportDocument, UploadResult};

pub trait ReportUploader {
    fn upload(&self, payload: &ReportDocument) -> Result<UploadResult, GitAiError>;
}

pub trait SummaryUploader {
    fn upload_summary(&self, payload: &ProjectSummaryReport) -> Result<UploadResult, GitAiError>;
}

pub struct DryRunUploader {
    pub server_url: String,
}

impl ReportUploader for DryRunUploader {
    fn upload(&self, payload: &ReportDocument) -> Result<UploadResult, GitAiError> {
        validate_server_url(&self.server_url)?;
        Ok(UploadResult {
            uploaded: false,
            message: format!(
                "Upload transport is not enabled yet. Prepared sanitized payload for {} with {} commits.",
                self.server_url,
                payload.commits.len()
            ),
            commit_count: payload.commits.len(),
        })
    }
}

impl SummaryUploader for DryRunUploader {
    fn upload_summary(&self, payload: &ProjectSummaryReport) -> Result<UploadResult, GitAiError> {
        validate_server_url(&self.server_url)?;
        Ok(UploadResult {
            uploaded: false,
            message: format!(
                "Dry run: would upload summary for '{}' ({} developers) to {}.",
                payload.project_name,
                payload.developers.len(),
                self.server_url
            ),
            commit_count: payload.total_commits,
        })
    }
}

pub struct HttpUploader {
    pub server_url: String,
}

impl ReportUploader for HttpUploader {
    fn upload(&self, payload: &ReportDocument) -> Result<UploadResult, GitAiError> {
        let context = ApiContext::new(Some(self.server_url.clone()));
        let endpoint = report_ingest_endpoint(&context.base_url)?;
        let body = serde_json::to_string(payload)?;
        let (_agent, request) = ApiContext::http_post(&endpoint, context.timeout_secs);
        let request = apply_auth_headers(request.set("Content-Type", "application/json"), &context);
        let response = crate::http::send_with_body(request, &body).map_err(GitAiError::Generic)?;
        let response_body = response.as_str().map_err(GitAiError::from)?;

        if !(200..300).contains(&response.status_code) {
            return Err(GitAiError::Generic(format!(
                "Report upload failed with HTTP {}: {}",
                response.status_code, response_body
            )));
        }

        let server_response: serde_json::Value = serde_json::from_str(response_body)?;
        let inserted_commits = server_response
            .get("inserted_commits")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let duplicate_commits = server_response
            .get("duplicate_commits")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        Ok(UploadResult {
            uploaded: true,
            message: format!(
                "Uploaded sanitized report to {} (inserted {}, duplicates {}).",
                endpoint, inserted_commits, duplicate_commits
            ),
            commit_count: payload.commits.len(),
        })
    }
}

impl SummaryUploader for HttpUploader {
    fn upload_summary(&self, payload: &ProjectSummaryReport) -> Result<UploadResult, GitAiError> {
        let context = ApiContext::new(Some(self.server_url.clone()));
        let endpoint = summary_ingest_endpoint(&context.base_url)?;
        let body = serde_json::to_string(payload)?;
        let (_agent, request) = ApiContext::http_post(&endpoint, context.timeout_secs);
        let request = apply_auth_headers(request.set("Content-Type", "application/json"), &context);
        let response = crate::http::send_with_body(request, &body).map_err(GitAiError::Generic)?;
        let response_body = response.as_str().map_err(GitAiError::from)?;

        if !(200..300).contains(&response.status_code) {
            return Err(GitAiError::Generic(format!(
                "Summary upload failed with HTTP {}: {}",
                response.status_code, response_body
            )));
        }

        Ok(UploadResult {
            uploaded: true,
            message: format!(
                "Uploaded summary for '{}' to {}.",
                payload.project_name, endpoint
            ),
            commit_count: payload.total_commits,
        })
    }
}

fn apply_auth_headers(mut request: ureq::Request, context: &ApiContext) -> ureq::Request {
    if let Some(api_key) = &context.api_key {
        request = request.set("X-API-Key", api_key);
        if let Some(identity) = &context.author_identity {
            request = request.set("X-Author-Identity", identity);
        }
    }
    if let Some(token) = &context.auth_token {
        request = request.set("Authorization", &format!("Bearer {}", token));
    }
    request
}

pub fn to_upload_payload(report: &ReportDocument) -> ReportDocument {
    let mut payload = report.clone();
    payload.repo.workdir = None;
    payload
}

pub fn report_ingest_endpoint(server_url: &str) -> Result<String, GitAiError> {
    validate_server_url(server_url)?;
    let mut parsed = url::Url::parse(server_url)
        .map_err(|e| GitAiError::Generic(format!("Invalid server URL: {}", e)))?;
    let path = parsed.path().trim_end_matches('/');
    if path.is_empty() {
        parsed.set_path("/api/v1/reports");
    } else if !path.ends_with("/api/v1/reports") {
        parsed.set_path(&format!("{}/api/v1/reports", path));
    }
    Ok(parsed.to_string())
}

pub fn summary_ingest_endpoint(server_url: &str) -> Result<String, GitAiError> {
    validate_server_url(server_url)?;
    let mut parsed = url::Url::parse(server_url)
        .map_err(|e| GitAiError::Generic(format!("Invalid server URL: {}", e)))?;
    let path = parsed.path().trim_end_matches('/');
    if path.is_empty() {
        parsed.set_path("/api/v1/summaries");
    } else if !path.ends_with("/api/v1/summaries") {
        parsed.set_path(&format!("{}/api/v1/summaries", path));
    }
    Ok(parsed.to_string())
}

pub fn validate_server_url(server_url: &str) -> Result<(), GitAiError> {
    let parsed = url::Url::parse(server_url)
        .map_err(|e| GitAiError::Generic(format!("Invalid server URL: {}", e)))?;
    match parsed.scheme() {
        "https" | "http" => Ok(()),
        other => Err(GitAiError::Generic(format!(
            "Invalid server URL scheme '{}'. Expected http or https",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::types::StoredCredentials;
    use crate::auth::CredentialStore;
    use crate::authorship::stats::CommitStats;
    use crate::report::model::{
        DeveloperSummary, ProjectRatios, ProjectSummaryReport, ReportCommit, ReportRangeInfo,
        ReportRangeMode, ReportRatios, ReportRepoInfo, ReportSummary, REPORT_SCHEMA_VERSION,
    };
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    fn sample_report() -> ReportDocument {
        ReportDocument {
            schema_version: REPORT_SCHEMA_VERSION.to_string(),
            generated_at: "now".to_string(),
            tool_version: "test".to_string(),
            repo: ReportRepoInfo {
                workdir: Some("/secret/path".to_string()),
                remote_url_hash: Some("sha256:abc".to_string()),
                branch: Some("main".to_string()),
                head_commit: None,
            },
            range: ReportRangeInfo {
                mode: ReportRangeMode::Head,
                from: None,
                to: None,
                since: None,
                until: None,
                commit_count: 0,
                commits_with_authorship: 0,
                commits_without_authorship: 0,
            },
            summary: ReportSummary::default(),
            ratios: ReportRatios::default(),
            tool_model_breakdown: BTreeMap::new(),
            commits: vec![ReportCommit {
                sha: "abc".to_string(),
                author: "Test <test@example.com>".to_string(),
                author_time: "2026-05-18T00:00:00Z".to_string(),
                subject: "test".to_string(),
                has_authorship_note: true,
                stats: CommitStats::default(),
            }],
        }
    }

    #[test]
    fn upload_payload_removes_workdir() {
        let report = sample_report();

        let payload = to_upload_payload(&report);
        assert_eq!(payload.repo.workdir, None);
        assert_eq!(payload.repo.remote_url_hash.as_deref(), Some("sha256:abc"));
    }

    #[test]
    fn report_ingest_endpoint_appends_api_path() {
        assert_eq!(
            report_ingest_endpoint("http://127.0.0.1:8787").unwrap(),
            "http://127.0.0.1:8787/api/v1/reports"
        );
        assert_eq!(
            report_ingest_endpoint("http://127.0.0.1:8787/prefix").unwrap(),
            "http://127.0.0.1:8787/prefix/api/v1/reports"
        );
        assert_eq!(
            report_ingest_endpoint("http://127.0.0.1:8787/api/v1/reports").unwrap(),
            "http://127.0.0.1:8787/api/v1/reports"
        );
    }

    #[test]
    fn http_uploader_posts_sanitized_report() {
        let _ = CredentialStore::new().clear();
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock server should bind");
        let addr = listener.local_addr().expect("mock server addr");
        let (request_tx, request_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("mock server should accept");
            let request = read_mock_http_request(&mut stream);
            request_tx
                .send(request)
                .expect("mock server should send captured request");

            let body =
                r#"{"project_id":1,"upload_id":2,"inserted_commits":1,"duplicate_commits":0}"#;
            let response = format!(
                "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("mock server should respond");
        });

        let payload = to_upload_payload(&sample_report());
        let result = HttpUploader {
            server_url: format!("http://{}", addr),
        }
        .upload(&payload)
        .expect("http upload should succeed");

        assert!(result.uploaded);
        assert_eq!(result.commit_count, 1);
        assert!(result.message.contains("inserted 1"));
        let request = request_rx
            .recv()
            .expect("mock server should capture request");
        let request_text = String::from_utf8_lossy(&request);
        assert!(request_text.starts_with("POST /api/v1/reports "));
        assert!(
            !request_text.contains("/secret/path"),
            "upload body should not include local workdir: {}",
            request_text
        );
        handle.join().expect("mock server should finish");
    }

    #[test]
    fn http_uploader_sends_stored_bearer_token() {
        let store = CredentialStore::new();
        let _ = store.clear();
        store
            .store(&StoredCredentials {
                access_token: "test_report_token".to_string(),
                refresh_token: "test_refresh_token".to_string(),
                access_token_expires_at: chrono::Utc::now().timestamp() + 3600,
                refresh_token_expires_at: chrono::Utc::now().timestamp() + 86400,
            })
            .expect("test credentials should be stored");

        let listener = TcpListener::bind("127.0.0.1:0").expect("mock server should bind");
        let addr = listener.local_addr().expect("mock server addr");
        let (request_tx, request_rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("mock server should accept");
            let request = read_mock_http_request(&mut stream);
            request_tx
                .send(request)
                .expect("mock server should send captured request");

            let body =
                r#"{"project_id":1,"upload_id":2,"inserted_commits":1,"duplicate_commits":0}"#;
            let response = format!(
                "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("mock server should respond");
        });

        HttpUploader {
            server_url: format!("http://{}", addr),
        }
        .upload(&to_upload_payload(&sample_report()))
        .expect("http upload should succeed");

        let request = request_rx
            .recv()
            .expect("mock server should capture request");
        let request_text = String::from_utf8_lossy(&request);
        assert!(request_text.contains("Authorization: Bearer test_report_token"));
        handle.join().expect("mock server should finish");
        let _ = store.clear();
    }

    fn read_mock_http_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_marker = b"\r\n\r\n";
        loop {
            let read = stream.read(&mut chunk).expect("mock server should read");
            assert!(read > 0, "client closed before headers were complete");
            request.extend_from_slice(&chunk[..read]);
            if let Some(header_end) = request
                .windows(header_marker.len())
                .position(|window| window == header_marker)
            {
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| line.split_once(':'))
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let expected_len = header_end + header_marker.len() + content_length;
                while request.len() < expected_len {
                    let read = stream
                        .read(&mut chunk)
                        .expect("mock server should read body");
                    assert!(read > 0, "client closed before body was complete");
                    request.extend_from_slice(&chunk[..read]);
                }
                request.truncate(expected_len);
                return request;
            }
        }
    }

    #[test]
    fn summary_ingest_endpoint_appends_api_path() {
        assert_eq!(
            summary_ingest_endpoint("http://127.0.0.1:8787").unwrap(),
            "http://127.0.0.1:8787/api/v1/summaries"
        );
        assert_eq!(
            summary_ingest_endpoint("http://127.0.0.1:8787/prefix").unwrap(),
            "http://127.0.0.1:8787/prefix/api/v1/summaries"
        );
        assert_eq!(
            summary_ingest_endpoint("http://127.0.0.1:8787/api/v1/summaries").unwrap(),
            "http://127.0.0.1:8787/api/v1/summaries"
        );
    }

    #[test]
    fn dry_run_summary_uploader_returns_message() {
        let summary = ProjectSummaryReport {
            project_name: "test-project".to_string(),
            git_url: None,
            branch: Some("main".to_string()),
            total_commits: 5,
            developers: vec![DeveloperSummary {
                name: "Dev".to_string(),
                email: "dev@test.com".to_string(),
                commits: 5,
                added_lines: 100,
                ai_additions: 50,
                human_additions: 50,
                ai_ratio: 0.5,
                human_ratio: 0.5,
            }],
            project_ratios: ProjectRatios {
                ai: 0.5,
                human: 0.5,
            },
            organization: None,
            department: None,
            reporter_name: None,
            reporter_email: None,
            report_period: None,
        };
        let uploader = DryRunUploader {
            server_url: "http://localhost:8787".to_string(),
        };
        let result = uploader.upload_summary(&summary).unwrap();
        assert!(!result.uploaded);
        assert!(result.message.contains("test-project"));
        assert!(result.message.contains("1 developers"));
    }
}
