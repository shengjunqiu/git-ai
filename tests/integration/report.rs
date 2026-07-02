use crate::repos::test_file::ExpectedLineExt;
use crate::repos::test_repo::TestRepo;
use serde_json::Value;
use std::fs;
use std::time::Instant;

fn extract_json_object(output: &str) -> String {
    let start = output
        .find('{')
        .unwrap_or_else(|| panic!("output does not contain JSON object: {}", output));
    let end = output
        .rfind('}')
        .unwrap_or_else(|| panic!("output does not contain complete JSON object: {}", output));
    output[start..=end].to_string()
}

fn report_json(repo: &TestRepo, args: &[&str]) -> Value {
    let raw = repo
        .git_ai(args)
        .unwrap_or_else(|error| panic!("git-ai {:?} should succeed: {}", args, error));
    serde_json::from_str(&extract_json_object(&raw)).expect("report output should be valid JSON")
}

#[test]
fn report_scan_json_for_head_commit() {
    let repo = TestRepo::new();

    let mut file = repo.filename("report-head.txt");
    file.set_contents(crate::lines!["human line".human(), "ai line".ai()]);
    let commit = repo.stage_all_and_commit("report head").unwrap();

    let report = report_json(&repo, &["report", "scan", "--json"]);

    assert_eq!(report["schema_version"], "git-ai-report/1.0.0");
    assert_eq!(report["range"]["mode"], "head");
    assert_eq!(report["range"]["commit_count"], 1);
    assert_eq!(report["range"]["commits_with_authorship"], 1);
    assert_eq!(report["commits"][0]["sha"], commit.commit_sha);
    assert_eq!(report["commits"][0]["has_authorship_note"], true);
    assert_eq!(report["summary"]["git_diff_added_lines"], 2);
    assert_eq!(report["summary"]["ai_additions"], 1);
    assert_eq!(report["summary"]["human_additions"], 1);
    assert_eq!(report["ratios"]["ai"], 0.5);
    assert_eq!(report["ratios"]["human"], 0.5);
    file.assert_lines_and_blame(crate::lines!["human line".human(), "ai line".ai()]);
}

#[test]
fn report_export_json_and_csv_files() {
    let repo = TestRepo::new();

    let mut file = repo.filename("report-export.txt");
    file.set_contents(crate::lines!["human".human(), "ai".ai()]);
    repo.stage_all_and_commit("report export").unwrap();

    let json_path = repo.path().join("report-output.json");
    let csv_path = repo.path().join("report-output.csv");
    let json_path_arg = json_path.to_str().expect("valid json export path");
    let csv_path_arg = csv_path.to_str().expect("valid csv export path");

    repo.git_ai(&[
        "report",
        "export",
        "--format",
        "json",
        "--output",
        json_path_arg,
    ])
    .expect("json export should succeed");
    repo.git_ai(&[
        "report",
        "export",
        "--format",
        "csv",
        "--output",
        csv_path_arg,
    ])
    .expect("csv export should succeed");

    let json: Value =
        serde_json::from_str(&fs::read_to_string(json_path).expect("json export should exist"))
            .expect("json export should parse");
    assert_eq!(json["summary"]["git_diff_added_lines"], 2);

    let csv = fs::read_to_string(csv_path).expect("csv export should exist");
    assert!(
        csv.starts_with(
            "repo_hash,branch,commit_sha,author,author_time,subject,has_authorship_note"
        )
    );
    assert!(csv.contains(",true,"));
    assert!(csv.contains("report export"));
}

#[test]
fn report_range_limits_commits() {
    let repo = TestRepo::new();

    let mut file = repo.filename("report-range.txt");
    file.set_contents(crate::lines!["base".human()]);
    let first = repo.stage_all_and_commit("report base").unwrap();

    file.set_contents(crate::lines!["base".human(), "ai added".ai()]);
    let second = repo.stage_all_and_commit("report ai").unwrap();

    let range = format!("{}..{}", first.commit_sha, second.commit_sha);
    let report = report_json(&repo, &["report", "scan", "--range", &range, "--json"]);

    assert_eq!(report["range"]["mode"], "range");
    assert_eq!(report["range"]["commit_count"], 1);
    assert_eq!(report["range"]["from"], first.commit_sha);
    assert_eq!(report["range"]["to"], second.commit_sha);
    assert_eq!(report["commits"][0]["sha"], second.commit_sha);
    assert_eq!(report["summary"]["ai_additions"], 1);
}

#[test]
fn report_since_filter_limits_commit_list() {
    let repo = TestRepo::new();

    let mut file = repo.filename("report-date.txt");
    file.set_contents(crate::lines!["old".human()]);
    repo.stage_all_and_commit_with_env(
        "old report commit",
        &[
            ("GIT_AUTHOR_DATE", "2025-01-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2025-01-01T00:00:00Z"),
        ],
    )
    .unwrap();

    file.set_contents(crate::lines!["old".human(), "new ai".ai()]);
    let new_commit = repo
        .stage_all_and_commit_with_env(
            "new report commit",
            &[
                ("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z"),
                ("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z"),
            ],
        )
        .unwrap();

    let report = report_json(
        &repo,
        &["report", "scan", "--since", "2025-12-31", "--json"],
    );

    assert_eq!(report["range"]["mode"], "date");
    assert_eq!(report["range"]["commit_count"], 1);
    assert_eq!(report["commits"][0]["sha"], new_commit.commit_sha);
    assert_eq!(report["summary"]["ai_additions"], 1);
}

#[test]
fn report_ignore_patterns_remove_matching_paths_from_totals() {
    let repo = TestRepo::new();

    let mut included = repo.filename("included.txt");
    included.set_contents(crate::lines!["included ai".ai()]);
    let mut ignored = repo.filename("ignored.txt");
    ignored.set_contents(crate::lines!["ignored ai".ai()]);
    repo.stage_all_and_commit("report ignore").unwrap();

    let full_report = report_json(&repo, &["report", "scan", "--json"]);
    assert_eq!(full_report["summary"]["ai_additions"], 2);

    let ignored_report = report_json(
        &repo,
        &["report", "scan", "--ignore", "ignored.txt", "--json"],
    );
    assert_eq!(ignored_report["summary"]["git_diff_added_lines"], 1);
    assert_eq!(ignored_report["summary"]["ai_additions"], 1);
}

#[test]
fn report_scan_marks_commits_without_authorship_notes() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("raw-commit.txt");
    fs::write(&file_path, "raw line\n").expect("raw test file should be written");

    repo.git_og(&["add", "-A"]).expect("raw add should succeed");
    let commit_output = repo
        .git_og(&["commit", "-m", "raw commit without note"])
        .expect("raw commit should succeed");
    assert!(
        commit_output.contains("raw commit without note"),
        "sanity check raw commit output: {}",
        commit_output
    );

    let report = report_json(&repo, &["report", "scan", "--json"]);

    assert_eq!(report["range"]["commit_count"], 1);
    assert_eq!(report["range"]["commits_with_authorship"], 0);
    assert_eq!(report["range"]["commits_without_authorship"], 1);
    assert_eq!(report["commits"][0]["has_authorship_note"], false);
}

#[test]
fn report_scan_terminal_summary_warns_about_missing_notes() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("summary-raw.txt");
    fs::write(&file_path, "raw line\n").expect("raw test file should be written");
    repo.git_og(&["add", "-A"]).expect("raw add should succeed");
    repo.git_og(&["commit", "-m", "summary raw commit"])
        .expect("raw commit should succeed");

    let output = repo
        .git_ai(&["report", "scan"])
        .expect("terminal report scan should succeed");

    assert!(output.contains("Authorship notes: 0/1"));
    assert!(output.contains("commits have no Git AI authorship note"));
}

#[test]
fn report_export_csv_escapes_subjects_with_commas_and_quotes() {
    let repo = TestRepo::new();

    let mut file = repo.filename("report-csv-escaping.txt");
    file.set_contents(crate::lines!["ai csv line".ai()]);
    repo.stage_all_and_commit("report, \"csv\" export")
        .expect("commit with CSV-sensitive subject should succeed");

    let csv_path = repo.path().join("report-escaped.csv");
    let csv_path_arg = csv_path.to_str().expect("valid csv export path");
    repo.git_ai(&[
        "report",
        "export",
        "--format",
        "csv",
        "--output",
        csv_path_arg,
    ])
    .expect("csv export should succeed");

    let csv = fs::read_to_string(csv_path).expect("csv export should exist");
    assert!(csv.contains("\"report, \"\"csv\"\" export\""));
}

#[test]
fn report_upload_uses_sanitized_payload_from_exported_json() {
    let repo = TestRepo::new();

    let mut file = repo.filename("report-upload.txt");
    file.set_contents(crate::lines!["ai upload line".ai()]);
    repo.stage_all_and_commit("report upload").unwrap();

    let json_path = repo.path().join("report-upload.json");
    let json_path_arg = json_path.to_str().expect("valid upload report path");
    repo.git_ai(&[
        "report",
        "export",
        "--format",
        "json",
        "--output",
        json_path_arg,
    ])
    .expect("json export should succeed");

    let upload = report_json(
        &repo,
        &[
            "report",
            "upload",
            json_path_arg,
            "--server",
            "https://example.invalid/reports",
            "--dry-run",
        ],
    );

    assert_eq!(upload["uploaded"], false);
    assert_eq!(upload["commit_count"], 1);
    assert!(
        upload["message"]
            .as_str()
            .expect("upload message should be a string")
            .contains("Prepared sanitized payload")
    );
}

// =============================================================================
// Phase 1 Extended Tests: CLI Scan MVP — comprehensive scan scenarios
// =============================================================================

#[test]
fn report_scan_all_human_commit_shows_zero_ai_ratio() {
    let repo = TestRepo::new();

    let mut file = repo.filename("all-human.txt");
    file.set_contents(crate::lines![
        "line 1".human(),
        "line 2".human(),
        "line 3".human()
    ]);
    repo.stage_all_and_commit("all human commit").unwrap();

    let report = report_json(&repo, &["report", "scan", "--json"]);

    assert_eq!(report["summary"]["ai_additions"], 0);
    assert_eq!(report["summary"]["human_additions"], 3);
    assert_eq!(report["ratios"]["ai"], 0.0);
    assert_eq!(report["ratios"]["human"], 1.0);
    file.assert_lines_and_blame(crate::lines![
        "line 1".human(),
        "line 2".human(),
        "line 3".human()
    ]);
}

#[test]
fn report_scan_all_ai_commit_shows_100_percent_ai_ratio() {
    let repo = TestRepo::new();

    let mut file = repo.filename("all-ai.txt");
    file.set_contents(crate::lines!["ai 1".ai(), "ai 2".ai()]);
    repo.stage_all_and_commit("all ai commit").unwrap();

    let report = report_json(&repo, &["report", "scan", "--json"]);

    assert_eq!(report["summary"]["ai_additions"], 2);
    assert_eq!(report["summary"]["human_additions"], 0);
    assert_eq!(report["ratios"]["ai"], 1.0);
    assert_eq!(report["ratios"]["human"], 0.0);
    file.assert_lines_and_blame(crate::lines!["ai 1".ai(), "ai 2".ai()]);
}

#[test]
fn report_scan_multiple_commits_aggregates_stats_correctly() {
    let repo = TestRepo::new();

    let mut file = repo.filename("multi-commit.txt");
    // Commit 1: 2 human, 1 ai
    file.set_contents(crate::lines!["h1".human(), "h2".human(), "a1".ai()]);
    let first = repo.stage_all_and_commit("first commit").unwrap();

    // Commit 2: 1 more human, 1 more ai
    file.set_contents(crate::lines![
        "h1".human(),
        "h2".human(),
        "a1".ai(),
        "h3".human(),
        "a2".ai()
    ]);
    let _second = repo.stage_all_and_commit("second commit").unwrap();

    let range = format!("{}..{}", first.commit_sha, _second.commit_sha);
    let report = report_json(&repo, &["report", "scan", "--range", &range, "--json"]);

    assert_eq!(report["range"]["commit_count"], 1);
    // Second commit adds new lines; exact counts depend on diff attribution
    assert!(report["summary"]["git_diff_added_lines"].as_u64().unwrap() >= 2);
    assert!(report["summary"]["ai_additions"].as_u64().unwrap() >= 1);
}

#[test]
fn report_scan_mixed_attribution_line_counts_correctly() {
    let repo = TestRepo::new();

    let mut file = repo.filename("mixed-file.txt");
    // Create with 1 human + 1 ai + 1 mixed
    file.set_contents(crate::lines!["human line".human(), "ai line".ai()]);
    repo.stage_all_and_commit("mixed commit").unwrap();

    let report = report_json(&repo, &["report", "scan", "--json"]);

    assert_eq!(report["summary"]["git_diff_added_lines"], 2);
    // Ratios should sum to ~1.0 (ai + human)
    let ai = report["ratios"]["ai"].as_f64().unwrap();
    let human = report["ratios"]["human"].as_f64().unwrap();
    let mixed = report["ratios"]["mixed"].as_f64().unwrap();
    let unknown = report["ratios"]["unknown"].as_f64().unwrap();
    let total_ratio = ai + human + mixed + unknown;
    assert!(
        (total_ratio - 1.0).abs() < 0.01,
        "ratios should sum to 1.0, got {}",
        total_ratio
    );
}

#[test]
fn report_scan_shows_repo_info_in_json() {
    let repo = TestRepo::new();

    let mut file = repo.filename("repo-info.txt");
    file.set_contents(crate::lines!["content".human()]);
    repo.stage_all_and_commit("repo info test").unwrap();

    let report = report_json(&repo, &["report", "scan", "--json"]);

    // Repo section should have workdir
    assert!(report["repo"]["workdir"].is_string());
    // Branch should be present
    assert!(report["repo"]["branch"].is_string());
    // head_commit should be present
    assert!(report["repo"]["head_commit"].is_string());
}

#[test]
fn report_scan_tool_model_breakdown_present_in_json() {
    let repo = TestRepo::new();

    let mut file = repo.filename("tool-model.txt");
    file.set_contents(crate::lines!["ai line".ai()]);
    repo.stage_all_and_commit("tool model test").unwrap();

    let report = report_json(&repo, &["report", "scan", "--json"]);

    // tool_model_breakdown should exist (even if empty for some agents)
    assert!(report["tool_model_breakdown"].is_object());
}

#[test]
fn report_scan_commit_details_include_author_and_time() {
    let repo = TestRepo::new();

    let mut file = repo.filename("commit-detail.txt");
    file.set_contents(crate::lines!["detail".human()]);
    repo.stage_all_and_commit("commit detail test").unwrap();

    let report = report_json(&repo, &["report", "scan", "--json"]);

    let commit = &report["commits"][0];
    assert!(commit["author"].is_string());
    assert!(commit["author"].as_str().unwrap().contains('@'));
    assert!(commit["author_time"].is_string());
    assert!(commit["subject"].as_str().unwrap() == "commit detail test");
}

// =============================================================================
// Phase 2 Extended Tests: Export MVP — JSON/CSV edge cases
// =============================================================================

#[test]
fn report_export_json_file_is_valid_and_parseable() {
    let repo = TestRepo::new();

    let mut file = repo.filename("json-valid.txt");
    file.set_contents(crate::lines!["h".human(), "a".ai()]);
    repo.stage_all_and_commit("json valid test").unwrap();

    let json_path = repo.path().join("report-valid.json");
    let json_path_arg = json_path.to_str().expect("valid path");
    repo.git_ai(&[
        "report",
        "export",
        "--format",
        "json",
        "--output",
        json_path_arg,
    ])
    .expect("json export should succeed");

    let content = fs::read_to_string(&json_path).expect("json file should exist");
    let parsed: Value = serde_json::from_str(&content).expect("exported JSON should be valid");
    assert_eq!(parsed["schema_version"], "git-ai-report/1.0.0");
    assert_eq!(parsed["summary"]["git_diff_added_lines"], 2);
}

#[test]
fn report_export_csv_contains_all_required_columns() {
    let repo = TestRepo::new();

    let mut file = repo.filename("csv-cols.txt");
    file.set_contents(crate::lines!["csv h".human(), "csv a".ai()]);
    repo.stage_all_and_commit("csv columns test").unwrap();

    let csv_path = repo.path().join("report-cols.csv");
    let csv_path_arg = csv_path.to_str().expect("valid path");
    repo.git_ai(&[
        "report",
        "export",
        "--format",
        "csv",
        "--output",
        csv_path_arg,
    ])
    .expect("csv export should succeed");

    let csv = fs::read_to_string(&csv_path).expect("csv file should exist");
    let header_line = csv.lines().next().expect("csv should have a header");
    // Verify all expected columns
    assert!(header_line.contains("repo_hash"));
    assert!(header_line.contains("branch"));
    assert!(header_line.contains("commit_sha"));
    assert!(header_line.contains("author"));
    assert!(header_line.contains("author_time"));
    assert!(header_line.contains("subject"));
    assert!(header_line.contains("has_authorship_note"));
    assert!(header_line.contains("ai_additions"));
    assert!(header_line.contains("human_additions"));
    assert!(header_line.contains("mixed_additions"));
    assert!(header_line.contains("unknown_additions"));
}

#[test]
fn report_export_csv_escapes_newlines_in_subject() {
    let repo = TestRepo::new();

    let mut file = repo.filename("csv-newline.txt");
    file.set_contents(crate::lines!["data".ai()]);
    // Use a subject with commas and quotes to test CSV escaping
    repo.stage_all_and_commit("subject, with \"quotes\"")
        .expect("commit should succeed");

    let csv_path = repo.path().join("report-newline.csv");
    let csv_path_arg = csv_path.to_str().expect("valid path");
    repo.git_ai(&[
        "report",
        "export",
        "--format",
        "csv",
        "--output",
        csv_path_arg,
    ])
    .expect("csv export should succeed");

    let csv = fs::read_to_string(&csv_path).expect("csv file should exist");
    // The subject with commas/quotes should be CSV-escaped
    assert!(
        csv.contains("\"subject, with \"\"quotes\"\"\""),
        "CSV should properly escape commas and quotes in subject: {}",
        csv
    );
}

// =============================================================================
// Phase 3 Extended Tests: Upload — payload sanitization and validation
// =============================================================================

#[test]
fn report_upload_dry_run_does_not_actually_upload() {
    let repo = TestRepo::new();

    let mut file = repo.filename("dry-run.txt");
    file.set_contents(crate::lines!["dry ai".ai(), "dry human".human()]);
    repo.stage_all_and_commit("dry run test").unwrap();

    let result = report_json(
        &repo,
        &[
            "report",
            "upload",
            "--server",
            "http://localhost:9999",
            "--dry-run",
        ],
    );

    assert_eq!(result["uploaded"], false);
    assert_eq!(result["commit_count"], 1);
}

#[test]
fn report_upload_from_json_file_removes_workdir() {
    let repo = TestRepo::new();

    let mut file = repo.filename("upload-sanitize.txt");
    file.set_contents(crate::lines!["sanitized".ai()]);
    repo.stage_all_and_commit("sanitize test").unwrap();

    let json_path = repo.path().join("report-sanitize.json");
    let json_path_arg = json_path.to_str().expect("valid path");
    repo.git_ai(&[
        "report",
        "export",
        "--format",
        "json",
        "--output",
        json_path_arg,
    ])
    .expect("json export should succeed");

    // Verify the exported JSON contains workdir
    let exported: Value =
        serde_json::from_str(&fs::read_to_string(&json_path).expect("json should exist"))
            .expect("exported json should parse");
    assert!(exported["repo"]["workdir"].is_string());

    // Upload dry-run from the JSON file — payload should be sanitized
    let upload = report_json(
        &repo,
        &[
            "report",
            "upload",
            json_path_arg,
            "--server",
            "https://example.com/reports",
            "--dry-run",
        ],
    );
    assert_eq!(upload["uploaded"], false);
}

#[test]
fn report_upload_without_server_flag_fails() {
    let repo = TestRepo::new();

    let mut file = repo.filename("no-server.txt");
    file.set_contents(crate::lines!["no server".ai()]);
    repo.stage_all_and_commit("no server test").unwrap();

    let result = repo.git_ai(&["report", "upload"]);
    assert!(result.is_err(), "upload without --server should fail");
}

// =============================================================================
// Phase 4 Extended Tests: CLI hardening — date, ignore, diagnostics
// =============================================================================

#[test]
fn report_until_filter_limits_commit_list() {
    let repo = TestRepo::new();

    let mut file = repo.filename("until-test.txt");
    file.set_contents(crate::lines!["old".human()]);
    repo.stage_all_and_commit_with_env(
        "old until commit",
        &[
            ("GIT_AUTHOR_DATE", "2025-01-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2025-01-01T00:00:00Z"),
        ],
    )
    .unwrap();

    file.set_contents(crate::lines!["old".human(), "new line".ai()]);
    repo.stage_all_and_commit_with_env(
        "new until commit",
        &[
            ("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z"),
        ],
    )
    .unwrap();

    // --until should only include commits before the date
    let report = report_json(
        &repo,
        &["report", "scan", "--until", "2025-06-01", "--json"],
    );

    assert_eq!(report["range"]["mode"], "date");
    assert_eq!(report["range"]["commit_count"], 1);
}

#[test]
fn report_since_and_until_combined_filters() {
    let repo = TestRepo::new();

    let mut file = repo.filename("range-dates.txt");
    file.set_contents(crate::lines!["base".human()]);
    repo.stage_all_and_commit_with_env(
        "commit early",
        &[
            ("GIT_AUTHOR_DATE", "2024-06-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2024-06-01T00:00:00Z"),
        ],
    )
    .unwrap();

    file.set_contents(crate::lines!["base".human(), "mid ai".ai()]);
    repo.stage_all_and_commit_with_env(
        "commit middle",
        &[
            ("GIT_AUTHOR_DATE", "2025-06-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2025-06-01T00:00:00Z"),
        ],
    )
    .unwrap();

    file.set_contents(crate::lines!["base".human(), "mid ai".ai(), "late ai".ai()]);
    repo.stage_all_and_commit_with_env(
        "commit late",
        &[
            ("GIT_AUTHOR_DATE", "2026-06-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2026-06-01T00:00:00Z"),
        ],
    )
    .unwrap();

    let report = report_json(
        &repo,
        &[
            "report",
            "scan",
            "--since",
            "2025-01-01",
            "--until",
            "2026-01-01",
            "--json",
        ],
    );

    assert_eq!(report["range"]["mode"], "date");
    // Only the "middle" commit should be within the date range
    assert_eq!(report["range"]["commit_count"], 1);
    assert_eq!(report["summary"]["ai_additions"], 1);
}

#[test]
fn report_ignore_multiple_patterns() {
    let repo = TestRepo::new();

    let mut included = repo.filename("keep.txt");
    included.set_contents(crate::lines!["keep ai".ai()]);
    let mut ignored1 = repo.filename("skip1.txt");
    ignored1.set_contents(crate::lines!["skip1 ai".ai()]);
    let mut ignored2 = repo.filename("skip2.txt");
    ignored2.set_contents(crate::lines!["skip2 ai".ai()]);
    repo.stage_all_and_commit("multi ignore test").unwrap();

    let report = report_json(
        &repo,
        &[
            "report",
            "scan",
            "--ignore",
            "skip1.txt",
            "skip2.txt",
            "--json",
        ],
    );

    // Only keep.txt should be counted
    assert_eq!(report["summary"]["ai_additions"], 1);
}

#[test]
fn report_ignore_glob_pattern() {
    let repo = TestRepo::new();

    let mut included = repo.filename("keep.txt");
    included.set_contents(crate::lines!["keep ai".ai()]);
    let mut ignored = repo.filename("skip_gen.txt");
    ignored.set_contents(crate::lines!["skip gen ai".ai()]);
    repo.stage_all_and_commit("glob ignore test").unwrap();

    let report = report_json(
        &repo,
        &["report", "scan", "--ignore", "skip_*.txt", "--json"],
    );

    assert_eq!(report["summary"]["ai_additions"], 1);
}

#[test]
fn report_scan_terminal_output_shows_percentages() {
    let repo = TestRepo::new();

    let mut file = repo.filename("terminal-out.txt");
    file.set_contents(crate::lines!["h line".human(), "a line".ai()]);
    repo.stage_all_and_commit("terminal output test").unwrap();

    let output = repo
        .git_ai(&["report", "scan"])
        .expect("terminal scan should succeed");

    assert!(output.contains("Repository:"));
    assert!(output.contains("Commits: 1"));
    assert!(output.contains("Authorship notes: 1/1"));
    assert!(output.contains("AI: 50.0%"));
    assert!(output.contains("Human: 50.0%"));
}

#[test]
fn report_scan_terminal_output_shows_zero_percent_when_all_human() {
    let repo = TestRepo::new();

    let mut file = repo.filename("terminal-human.txt");
    file.set_contents(crate::lines!["only human".human()]);
    repo.stage_all_and_commit("all human terminal").unwrap();

    let output = repo
        .git_ai(&["report", "scan"])
        .expect("terminal scan should succeed");

    assert!(output.contains("AI: 0.0%"));
    assert!(output.contains("Human: 100.0%"));
}

// =============================================================================
// Phase 5 Extended: end-to-end scan → export → upload pipeline
// =============================================================================

#[test]
fn report_full_pipeline_scan_export_upload_roundtrip() {
    let repo = TestRepo::new();

    let mut file = repo.filename("pipeline.txt");
    file.set_contents(crate::lines![
        "human 1".human(),
        "ai 1".ai(),
        "human 2".human(),
        "ai 2".ai()
    ]);
    repo.stage_all_and_commit("pipeline commit").unwrap();

    // Step 1: Scan
    let scan = report_json(&repo, &["report", "scan", "--json"]);
    assert_eq!(scan["summary"]["ai_additions"], 2);
    assert_eq!(scan["summary"]["human_additions"], 2);

    // Step 2: Export JSON
    let json_path = repo.path().join("pipeline.json");
    let json_path_arg = json_path.to_str().expect("valid path");
    repo.git_ai(&[
        "report",
        "export",
        "--format",
        "json",
        "--output",
        json_path_arg,
    ])
    .expect("json export should succeed");

    let exported: Value =
        serde_json::from_str(&fs::read_to_string(&json_path).expect("json should exist"))
            .expect("exported json should parse");
    assert_eq!(exported["summary"]["ai_additions"], 2);
    assert_eq!(exported["summary"]["human_additions"], 2);

    // Step 3: Export CSV
    let csv_path = repo.path().join("pipeline.csv");
    let csv_path_arg = csv_path.to_str().expect("valid path");
    repo.git_ai(&[
        "report",
        "export",
        "--format",
        "csv",
        "--output",
        csv_path_arg,
    ])
    .expect("csv export should succeed");

    let csv = fs::read_to_string(&csv_path).expect("csv should exist");
    assert!(csv.contains("2,2")); // ai_additions=2, human_additions=2

    // Step 4: Upload from exported JSON
    let upload = report_json(
        &repo,
        &[
            "report",
            "upload",
            json_path_arg,
            "--server",
            "https://example.com/api/v1/reports",
            "--dry-run",
        ],
    );
    assert_eq!(upload["uploaded"], false);
    assert_eq!(upload["commit_count"], 1);

    // Verify attribution integrity through the pipeline
    file.assert_lines_and_blame(crate::lines![
        "human 1".human(),
        "ai 1".ai(),
        "human 2".human(),
        "ai 2".ai(),
    ]);
}

#[test]
fn report_help_flag_shows_usage() {
    let repo = TestRepo::new();

    let output = repo
        .git_ai(&["report", "--help"])
        .expect("help should succeed");

    assert!(output.contains("git-ai report"));
    assert!(output.contains("scan"));
    assert!(output.contains("export"));
    assert!(output.contains("upload"));
    assert!(output.contains("server"));
}

#[test]
fn report_unknown_subcommand_fails() {
    let repo = TestRepo::new();

    let result = repo.git_ai(&["report", "nonexistent"]);
    assert!(result.is_err(), "unknown subcommand should fail");
}

// =============================================================================
// Phase 6 Extended: Server MVP integration — via HTTP API
// =============================================================================

#[test]
fn report_server_api_upload_and_read_back() {
    let repo = TestRepo::new();

    let mut file = repo.filename("server-e2e.txt");
    file.set_contents(crate::lines!["server human".human(), "server ai".ai()]);
    repo.stage_all_and_commit("server e2e commit").unwrap();

    // Step 1: Export JSON report
    let json_path = repo.path().join("server-e2e.json");
    let json_path_arg = json_path.to_str().expect("valid path");
    repo.git_ai(&[
        "report",
        "export",
        "--format",
        "json",
        "--output",
        json_path_arg,
    ])
    .expect("json export should succeed");

    // Step 2: Upload via dry-run — verifies the full pipeline without needing a real server
    let upload = report_json(
        &repo,
        &[
            "report",
            "upload",
            json_path_arg,
            "--server",
            "https://example.com/api/v1/reports",
            "--dry-run",
        ],
    );

    assert_eq!(upload["uploaded"], false);
    assert_eq!(upload["commit_count"], 1);

    // Verify the exported JSON has the right structure for server ingestion
    let exported: Value =
        serde_json::from_str(&fs::read_to_string(&json_path).expect("json should exist"))
            .expect("exported json should parse");

    assert_eq!(exported["schema_version"], "git-ai-report/1.0.0");
    assert!(exported["commits"].is_array());
    assert!(exported["summary"]["ai_additions"].as_u64().unwrap() >= 1);

    file.assert_lines_and_blame(crate::lines!["server human".human(), "server ai".ai()]);
}

// =============================================================================
// Custom checkpoint flow tests — validate scan with explicit checkpointing
// =============================================================================

#[test]
fn report_scan_with_custom_checkpoints_tracks_attribution() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("checkpoint-report.md");

    // Initial content — untracked
    let initial = "Untracked line\n";
    fs::write(&file_path, initial).unwrap();
    repo.stage_all_and_commit("initial commit").unwrap();

    let mut file = repo.filename("checkpoint-report.md");
    file.assert_committed_lines(crate::lines!["Untracked line".unattributed_human()]);

    // Add known human content
    let second = "Untracked line\nHuman line\n";
    fs::write(&file_path, second).unwrap();
    repo.git_ai(&["checkpoint", "mock_known_human", "checkpoint-report.md"])
        .unwrap();
    repo.stage_all_and_commit("add human line").unwrap();
    file.assert_committed_lines(crate::lines![
        "Untracked line".unattributed_human(),
        "Human line".human(),
    ]);

    // Add AI content
    let third = "Untracked line\nHuman line\nAI line\n";
    fs::write(&file_path, third).unwrap();
    repo.git_ai(&["checkpoint", "mock_ai", "checkpoint-report.md"])
        .unwrap();
    repo.stage_all_and_commit("add ai line").unwrap();
    file.assert_committed_lines(crate::lines![
        "Untracked line".unattributed_human(),
        "Human line".human(),
        "AI line".ai(),
    ]);

    // Scan report for the full history using a range
    let _first_sha = repo
        .git_og(&["rev-parse", "--short", "HEAD~2"])
        .expect("should get first sha")
        .trim()
        .to_string();
    let _head_sha = repo
        .git_og(&["rev-parse", "HEAD"])
        .expect("should get head sha")
        .trim()
        .to_string();

    // Note: use short sha for first commit since rev-list works with partial SHAs
    // Actually just scan all commits from initial
    let report = report_json(&repo, &["report", "scan", "--json"]);

    // HEAD only shows the latest commit
    assert_eq!(report["range"]["commit_count"], 1);
    // The last commit added the AI line, so we should have at least 1 AI addition
    assert!(
        report["summary"]["ai_additions"].as_u64().unwrap() >= 1,
        "should have AI additions"
    );
}

// =============================================================================
// Range boundary tests
// =============================================================================

#[test]
fn report_range_three_commits_with_different_attributions() {
    let repo = TestRepo::new();

    let mut file = repo.filename("three-commits.txt");
    file.set_contents(crate::lines!["base".human()]);
    let first = repo.stage_all_and_commit("base commit").unwrap();

    file.set_contents(crate::lines!["base".human(), "ai addition".ai()]);
    let _second = repo.stage_all_and_commit("ai commit").unwrap();

    file.set_contents(crate::lines![
        "base".human(),
        "ai addition".ai(),
        "human addition".human()
    ]);
    let third = repo.stage_all_and_commit("human commit").unwrap();

    // Full range
    let range = format!("{}..{}", first.commit_sha, third.commit_sha);
    let report = report_json(&repo, &["report", "scan", "--range", &range, "--json"]);

    assert_eq!(report["range"]["commit_count"], 2); // second + third
    // Verify both AI and human contributions exist
    assert!(
        report["summary"]["ai_additions"].as_u64().unwrap() >= 1,
        "should have AI additions"
    );
    assert!(
        report["summary"]["human_additions"].as_u64().unwrap() >= 1,
        "should have human additions"
    );
}

#[test]
fn report_invalid_range_format_fails() {
    let repo = TestRepo::new();

    let mut file = repo.filename("invalid-range.txt");
    file.set_contents(crate::lines!["content".human()]);
    repo.stage_all_and_commit("range test").unwrap();

    let result = repo.git_ai(&["report", "scan", "--range", "invalid-no-dots", "--json"]);
    assert!(result.is_err(), "invalid range format should fail");
}

#[test]
fn report_empty_since_range_fails() {
    let repo = TestRepo::new();

    let mut file = repo.filename("empty-range.txt");
    file.set_contents(crate::lines!["content".human()]);
    repo.stage_all_and_commit_with_env(
        "old commit",
        &[
            ("GIT_AUTHOR_DATE", "2020-01-01T00:00:00Z"),
            ("GIT_COMMITTER_DATE", "2020-01-01T00:00:00Z"),
        ],
    )
    .unwrap();

    // Since a very late date should yield no commits and fail
    let result = repo.git_ai(&["report", "scan", "--since", "2099-01-01", "--json"]);
    assert!(result.is_err(), "no commits in range should fail");
}

#[test]
#[ignore = "100+ commit smoke test for report scan performance; run manually when tuning report scan"]
fn report_scan_100_commit_range_smoke() {
    let repo = TestRepo::new();
    let file_path = repo.path().join("report-many-commits.txt");

    fs::write(&file_path, "line 0\n").expect("seed file should be written");
    repo.git_og(&["add", "-A"])
        .expect("seed add should succeed");
    repo.git_og(&["commit", "-q", "-m", "report seed"])
        .expect("seed commit should succeed");
    let first = repo
        .git_og(&["rev-parse", "HEAD"])
        .expect("first rev-parse should succeed")
        .trim()
        .to_string();

    for i in 1..=100 {
        let content = (0..=i)
            .map(|line| format!("line {}\n", line))
            .collect::<String>();
        fs::write(&file_path, content).expect("commit file should be written");
        repo.git_og(&["add", "-A"])
            .expect("loop add should succeed");
        repo.git_og(&["commit", "-q", "-m", &format!("report commit {}", i)])
            .expect("loop commit should succeed");
    }

    let last = repo
        .git_og(&["rev-parse", "HEAD"])
        .expect("last rev-parse should succeed")
        .trim()
        .to_string();
    let range = format!("{}..{}", first, last);

    let started = Instant::now();
    let report = report_json(&repo, &["report", "scan", "--range", &range, "--json"]);
    let elapsed = started.elapsed();

    assert_eq!(report["range"]["commit_count"], 100);
    eprintln!("report scan over 100 commits completed in {:.2?}", elapsed);
}

// =============================================================================
// Summary command tests — simplified project-level AI/human ratio report
// =============================================================================

fn summary_json(repo: &TestRepo, args: &[&str]) -> Value {
    let full_args = {
        let mut a = vec!["report", "summary"];
        a.extend_from_slice(args);
        a
    };
    let raw = repo
        .git_ai(&full_args)
        .unwrap_or_else(|error| panic!("git-ai summary {:?} should succeed: {}", args, error));
    serde_json::from_str(&extract_json_object(&raw)).expect("summary output should be valid JSON")
}

#[test]
fn summary_single_commit_has_project_name_and_ratios() {
    let repo = TestRepo::new();

    let mut file = repo.filename("summary.txt");
    file.set_contents(crate::lines!["human line".human(), "ai line".ai()]);
    repo.stage_all_and_commit("summary test").unwrap();

    let summary = summary_json(&repo, &[]);

    // project_name should exist (derived from directory name since no remote)
    assert!(
        summary["project_name"].is_string(),
        "project_name should be a string"
    );
    assert!(
        !summary["project_name"].as_str().unwrap().is_empty(),
        "project_name should not be empty"
    );
    // git_url may be null (no remote configured)
    assert!(summary["git_url"].is_null() || summary["git_url"].is_string());
    // branch should be present
    assert!(summary["branch"].is_string());
    // total_commits
    assert_eq!(summary["total_commits"], 1);
    // developers array
    assert!(summary["developers"].is_array());
    assert!(!summary["developers"].as_array().unwrap().is_empty());
    // project_ratios
    assert!(summary["project_ratios"]["ai"].is_f64());
    assert!(summary["project_ratios"]["human"].is_f64());
}

#[test]
fn summary_all_human_commit_shows_zero_ai_ratio() {
    let repo = TestRepo::new();

    let mut file = repo.filename("summary-human.txt");
    file.set_contents(crate::lines!["only human".human()]);
    repo.stage_all_and_commit("all human summary").unwrap();

    let summary = summary_json(&repo, &[]);

    assert_eq!(summary["project_ratios"]["ai"], 0.0);
    assert_eq!(summary["project_ratios"]["human"], 1.0);
}

#[test]
fn summary_all_ai_commit_shows_full_ai_ratio() {
    let repo = TestRepo::new();

    let mut file = repo.filename("summary-ai.txt");
    file.set_contents(crate::lines!["ai only".ai()]);
    repo.stage_all_and_commit("all ai summary").unwrap();

    let summary = summary_json(&repo, &[]);

    assert_eq!(summary["project_ratios"]["ai"], 1.0);
    assert_eq!(summary["project_ratios"]["human"], 0.0);
}

#[test]
fn summary_multiple_commits_aggregates_all() {
    let repo = TestRepo::new();

    let mut file = repo.filename("summary-multi.txt");
    file.set_contents(crate::lines!["human1".human()]);
    repo.stage_all_and_commit("commit 1").unwrap();

    file.set_contents(crate::lines!["human1".human(), "ai1".ai()]);
    repo.stage_all_and_commit("commit 2").unwrap();

    let summary = summary_json(&repo, &[]);

    // Both commits should be counted (total_commits >= 2)
    assert!(
        summary["total_commits"].as_u64().unwrap() >= 2,
        "should scan all history commits"
    );
    // AI ratio should be > 0
    assert!(
        summary["project_ratios"]["ai"].as_f64().unwrap() > 0.0,
        "project AI ratio should be positive"
    );
}

#[test]
fn summary_developer_has_name_and_ratios() {
    let repo = TestRepo::new();

    let mut file = repo.filename("summary-dev.txt");
    file.set_contents(crate::lines!["human line".human(), "ai line".ai()]);
    repo.stage_all_and_commit("dev ratio test").unwrap();

    let summary = summary_json(&repo, &[]);
    let devs = summary["developers"]
        .as_array()
        .expect("developers should be array");
    let dev = &devs[0];

    // Developer should have name, email, commits, and ratios
    assert!(dev["name"].is_string());
    assert!(dev["email"].is_string());
    // TestRepo sets user.name "Test User" and user.email "test@example.com"
    assert_eq!(dev["name"].as_str().unwrap(), "Test User");
    assert_eq!(dev["email"].as_str().unwrap(), "test@example.com");
    assert!(dev["commits"].as_u64().unwrap() >= 1);
    assert!(dev["added_lines"].as_u64().unwrap() >= 2);
    assert!(dev["ai_additions"].as_u64().unwrap() >= 1);
    assert!(dev["human_additions"].as_u64().unwrap() >= 1);
    assert!(dev["ai_ratio"].is_f64());
    assert!(dev["human_ratio"].is_f64());
    // Ratios should sum to ~1.0
    let ai = dev["ai_ratio"].as_f64().unwrap();
    let human = dev["human_ratio"].as_f64().unwrap();
    assert!(
        (ai + human - 1.0).abs() < 0.01,
        "dev ai_ratio + human_ratio should sum to ~1.0, got {} + {} = {}",
        ai,
        human,
        ai + human
    );
}

#[test]
fn summary_export_to_json_file() {
    let repo = TestRepo::new();

    let mut file = repo.filename("summary-export.txt");
    file.set_contents(crate::lines!["exported human".human(), "exported ai".ai()]);
    repo.stage_all_and_commit("summary export commit").unwrap();

    let json_path = repo.path().join("summary-export.json");
    let json_path_arg = json_path.to_str().expect("valid path");
    repo.git_ai(&[
        "report",
        "summary",
        "--format",
        "json",
        "--output",
        json_path_arg,
    ])
    .expect("summary json export should succeed");

    let content = fs::read_to_string(&json_path).expect("json file should exist");
    let exported: Value = serde_json::from_str(&content).expect("should parse as JSON");
    assert!(exported["project_name"].is_string());
    assert!(exported["developers"].is_array());
}

#[test]
fn summary_export_to_csv_file() {
    let repo = TestRepo::new();

    let mut file = repo.filename("summary-csv.txt");
    file.set_contents(crate::lines!["csv human".human(), "csv ai".ai()]);
    repo.stage_all_and_commit("summary csv commit").unwrap();

    let csv_path = repo.path().join("summary-export.csv");
    let csv_path_arg = csv_path.to_str().expect("valid path");
    repo.git_ai(&[
        "report",
        "summary",
        "--format",
        "csv",
        "--output",
        csv_path_arg,
    ])
    .expect("summary csv export should succeed");

    let csv = fs::read_to_string(&csv_path).expect("csv file should exist");
    // Check CSV header
    assert!(
        csv.contains("project_name,git_url,branch,developer,developer_email,commits,added_lines,ai_additions,human_additions,ai_ratio,human_ratio,project_ai_ratio,project_human_ratio"),
        "CSV should have expected header columns"
    );
    // Check data row exists
    assert!(
        csv.lines().count() >= 2,
        "CSV should have header + at least 1 data row"
    );
}

#[test]
fn summary_help_lists_summary_command() {
    let repo = TestRepo::new();

    let output = repo
        .git_ai(&["report", "--help"])
        .expect("help should succeed");

    assert!(
        output.contains("summary"),
        "help should mention summary command"
    );
}

#[test]
fn summary_with_ignore_patterns() {
    let repo = TestRepo::new();

    let mut keep = repo.filename("keep-summary.txt");
    keep.set_contents(crate::lines!["keep line".ai()]);
    let mut skip = repo.filename("skip-summary.txt");
    skip.set_contents(crate::lines!["skip line".ai()]);
    repo.stage_all_and_commit("ignore test summary").unwrap();

    let summary = summary_json(&repo, &["--ignore", "skip-summary.txt"]);

    // Only keep-summary.txt should be counted
    assert!(
        summary["project_ratios"]["ai"].as_f64().unwrap() > 0.0,
        "AI ratio should be positive with ignore filter"
    );
}
