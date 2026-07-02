use crate::error::GitAiError;
use crate::report::export::{export_csv, export_json, export_summary_csv, export_summary_json};
use crate::report::model::{ReportFormat, ReportOptions};
use crate::report::scan::{build_project_summary, resolve_report_repository, scan_report};
use crate::report::server::serve;
use crate::report::upload::{
    DryRunUploader, HttpUploader, ReportUploader, SummaryUploader, to_upload_payload,
};
use std::path::PathBuf;

pub fn handle_report(args: &[String]) {
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        print_help();
        return;
    }

    let result = match args[0].as_str() {
        "scan" => handle_scan(&args[1..]),
        "export" => handle_export(&args[1..]),
        "summary" => handle_summary(&args[1..]),
        "upload" => handle_upload(&args[1..]),
        "server" => handle_server(&args[1..]),
        other => Err(GitAiError::Generic(format!(
            "Unknown report command: {}",
            other
        ))),
    };

    if let Err(e) = result {
        eprintln!("Report failed: {}", e);
        std::process::exit(1);
    }
}

fn handle_scan(args: &[String]) -> Result<(), GitAiError> {
    let (options, json_output, _) = parse_report_options(args)?;
    let repo = resolve_report_repository(options.repo_path.as_deref())?;
    let report = scan_report(&repo, &options)?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_summary(&report);
    }

    Ok(())
}

fn handle_summary(args: &[String]) -> Result<(), GitAiError> {
    let (options, _json_output, export_opts) = parse_report_options(args)?;
    let repo = resolve_report_repository(options.repo_path.as_deref())?;
    let mut summary = build_project_summary(&repo, &options)?;

    // 将 CLI 传入的元数据覆盖到摘要（CLI 优先于 git config 自动填充）
    if export_opts.organization.is_some() {
        summary.organization = export_opts.organization.clone();
    }
    if export_opts.department.is_some() {
        summary.department = export_opts.department.clone();
    }
    if let Some(ref reporter) = export_opts.reporter {
        summary.reporter_name = Some(reporter.clone());
    }
    if let Some(ref reporter_email) = export_opts.reporter_email {
        summary.reporter_email = Some(reporter_email.clone());
    }
    if export_opts.period.is_some() {
        summary.report_period = export_opts.period.clone();
    }

    // If --server is provided, upload the summary instead of exporting locally
    if let Some(server_url) = export_opts.server {
        let result = if export_opts.dry_run {
            let uploader = DryRunUploader { server_url };
            uploader.upload_summary(&summary)?
        } else {
            let uploader = HttpUploader { server_url };
            uploader.upload_summary(&summary)?
        };
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    let format = export_opts.format.unwrap_or(ReportFormat::Json);
    let output = export_opts.output.as_deref().map(PathBuf::from);

    match format {
        ReportFormat::Json => {
            export_summary_json(&summary, output.as_deref())?;
        }
        ReportFormat::Csv => {
            export_summary_csv(&summary, output.as_deref())?;
        }
    }

    Ok(())
}

fn handle_export(args: &[String]) -> Result<(), GitAiError> {
    let (options, _json_output, export_opts) = parse_report_options(args)?;
    let format = export_opts.format.unwrap_or(ReportFormat::Json);
    let repo = resolve_report_repository(options.repo_path.as_deref())?;
    let report = scan_report(&repo, &options)?;
    let output = export_opts.output.as_deref().map(PathBuf::from);

    match format {
        ReportFormat::Json => {
            export_json(&report, output.as_deref())?;
        }
        ReportFormat::Csv => {
            export_csv(&report, output.as_deref())?;
        }
    }

    Ok(())
}

fn handle_upload(args: &[String]) -> Result<(), GitAiError> {
    let (options, _json_output, export_opts) = parse_report_options(args)?;
    let server_url = export_opts
        .server
        .ok_or_else(|| GitAiError::Generic("report upload requires --server <url>".to_string()))?;

    let report = if let Some(path) = options.repo_path.as_deref()
        && path.ends_with(".json")
        && std::path::Path::new(path).is_file()
    {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content)?
    } else {
        let repo = resolve_report_repository(options.repo_path.as_deref())?;
        scan_report(&repo, &options)?
    };

    let payload = to_upload_payload(&report);
    let result = if export_opts.dry_run {
        let uploader = DryRunUploader { server_url };
        uploader.upload(&payload)?
    } else {
        let uploader = HttpUploader { server_url };
        uploader.upload(&payload)?
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn handle_server(args: &[String]) -> Result<(), GitAiError> {
    let mut addr =
        std::env::var("GIT_AI_REPORT_ADDR").unwrap_or_else(|_| "127.0.0.1:8787".to_string());
    let mut db = std::env::var("GIT_AI_REPORT_DB")
        .unwrap_or_else(|_| "git-ai-report-server.sqlite".to_string());

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--addr" => {
                addr = required_value(args, i, "--addr")?;
                i += 2;
            }
            "--db" => {
                db = required_value(args, i, "--db")?;
                i += 2;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            value => {
                return Err(GitAiError::Generic(format!(
                    "Unknown report server argument: {}",
                    value
                )));
            }
        }
    }

    serve(&addr, &PathBuf::from(db))
}

#[derive(Default)]
struct ExportOptions {
    format: Option<ReportFormat>,
    output: Option<String>,
    server: Option<String>,
    dry_run: bool,
    /// 上报元数据
    organization: Option<String>,
    department: Option<String>,
    reporter: Option<String>,
    reporter_email: Option<String>,
    period: Option<String>,
}

fn parse_report_options(
    args: &[String],
) -> Result<(ReportOptions, bool, ExportOptions), GitAiError> {
    let mut repo_path = None;
    let mut json_output = false;
    let mut export = ExportOptions::default();
    let mut options = ReportOptions::new(None);

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json_output = true;
                export.format = Some(ReportFormat::Json);
                i += 1;
            }
            "--range" => {
                options.range = Some(required_value(args, i, "--range")?);
                i += 2;
            }
            "--branch" => {
                options.branch = Some(required_value(args, i, "--branch")?);
                i += 2;
            }
            "--since" => {
                options.since = Some(required_value(args, i, "--since")?);
                i += 2;
            }
            "--until" => {
                options.until = Some(required_value(args, i, "--until")?);
                i += 2;
            }
            "--ignore" => {
                i += 1;
                let mut found = false;
                while i < args.len() && !args[i].starts_with("--") {
                    options.ignore_patterns.push(args[i].clone());
                    found = true;
                    i += 1;
                }
                if !found {
                    return Err(GitAiError::Generic(
                        "--ignore requires at least one pattern".to_string(),
                    ));
                }
            }
            "--format" => {
                let value = required_value(args, i, "--format")?;
                export.format = Some(ReportFormat::parse(&value).ok_or_else(|| {
                    GitAiError::Generic(format!("Invalid report format '{}'", value))
                })?);
                i += 2;
            }
            "--output" | "-o" => {
                export.output = Some(required_value(args, i, "--output")?);
                i += 2;
            }
            "--server" => {
                export.server = Some(required_value(args, i, "--server")?);
                i += 2;
            }
            "--dry-run" => {
                export.dry_run = true;
                i += 1;
            }
            "--org" | "--organization" => {
                export.organization = Some(required_value(args, i, "--organization")?);
                i += 2;
            }
            "--dept" | "--department" => {
                export.department = Some(required_value(args, i, "--department")?);
                i += 2;
            }
            "--reporter" | "--reporter-name" => {
                export.reporter = Some(required_value(args, i, "--reporter-name")?);
                i += 2;
            }
            "--reporter-email" => {
                export.reporter_email = Some(required_value(args, i, "--reporter-email")?);
                i += 2;
            }
            "--period" | "--report-period" => {
                export.period = Some(required_value(args, i, "--report-period")?);
                i += 2;
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            value if value.starts_with('-') => {
                return Err(GitAiError::Generic(format!(
                    "Unknown report argument: {}",
                    value
                )));
            }
            value => {
                if repo_path.is_some() {
                    return Err(GitAiError::Generic(format!(
                        "Unexpected extra report argument: {}",
                        value
                    )));
                }
                repo_path = Some(value.to_string());
                i += 1;
            }
        }
    }

    options.repo_path = repo_path;
    Ok((options, json_output, export))
}

fn required_value(args: &[String], index: usize, flag: &str) -> Result<String, GitAiError> {
    args.get(index + 1)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| GitAiError::Generic(format!("{} requires a value", flag)))
}

fn print_summary(report: &crate::report::ReportDocument) {
    println!(
        "Repository: {}",
        report.repo.workdir.as_deref().unwrap_or("(unknown)")
    );
    println!("Commits: {}", report.range.commit_count);
    println!(
        "Authorship notes: {}/{}",
        report.range.commits_with_authorship, report.range.commit_count
    );
    println!("Added lines: {}", report.summary.git_diff_added_lines);
    println!("Deleted lines: {}", report.summary.git_diff_deleted_lines);
    println!("AI: {:.1}%", report.ratios.ai * 100.0);
    println!("Human: {:.1}%", report.ratios.human * 100.0);
    println!("Mixed: {:.1}%", report.ratios.mixed * 100.0);
    println!("Unknown: {:.1}%", report.ratios.unknown * 100.0);

    if report.range.commits_without_authorship > 0 {
        println!(
            "Note: {} commits have no Git AI authorship note; unknown lines may include code created before git-ai tracking or before notes were fetched.",
            report.range.commits_without_authorship
        );
    }
}

fn print_help() {
    eprintln!("git-ai report - generate AI/human project usage reports");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  git-ai report scan [repo] [--range <from>..<to>] [--json]");
    eprintln!(
        "  git-ai report export [repo] [--range <from>..<to>] --format <json|csv> --output <path>"
    );
    eprintln!(
        "  git-ai report summary [repo] [--format <json|csv>] [--output <path>] [--server <url> [--dry-run]]"
    );
    eprintln!(
        "  git-ai report upload <report.json|repo> [--range <from>..<to>] --server <url> [--dry-run]"
    );
    eprintln!("  git-ai report server [--addr 127.0.0.1:8787] [--db report.sqlite]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  scan      Scan commits and show AI/human attribution stats");
    eprintln!("  export    Export full report to JSON or CSV file");
    eprintln!(
        "  summary   Generate simplified project summary (all history, per-developer AI ratio)"
    );
    eprintln!("  upload    Upload report to a server");
    eprintln!("  server    Start a report ingestion server");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --range <from>..<to>  Commit range to scan");
    eprintln!("  --branch <branch>     Branch to scan");
    eprintln!("  --since <time>        Filter commits after time");
    eprintln!("  --until <time>        Filter commits before time");
    eprintln!("  --ignore <patterns>   Ignore file patterns");
    eprintln!("  --json                Print JSON report");
    eprintln!("  --format <json|csv>   Export format");
    eprintln!("  --output, -o <path>   Export output path");
    eprintln!("  --server <url>        Upload server URL");
    eprintln!("  --dry-run             Validate upload payload without sending it");
    eprintln!("  --org <name>          Organization name (attached to summary upload)");
    eprintln!("  --dept <name>         Department name (attached to summary upload)");
    eprintln!("  --reporter <email>    Override reporter email (default: git config user.email)");
    eprintln!("  --period <label>      Report period label, e.g. 2026-Q2");
    eprintln!("  --addr <host:port>    Report server listen address");
    eprintln!("  --db <path>           Report server SQLite database path");
}
