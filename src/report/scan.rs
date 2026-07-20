use crate::authorship::ignore::effective_ignore_patterns;
use crate::authorship::stats::{CommitStats, stats_for_commit_stats};
use crate::error::GitAiError;
use crate::git::refs::commits_with_authorship_notes;
use crate::git::repository::{Repository, exec_git};
use crate::git::{find_repository, find_repository_in_path};
use crate::report::model::{DeveloperSummary, ProjectRatios, ProjectSummaryReport};
use crate::report::model::{
    REPORT_SCHEMA_VERSION, ReportCommit, ReportDocument, ReportOptions, ReportRangeInfo,
    ReportRangeMode, ReportRepoInfo, ReportSummary, calculate_ratios,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub fn resolve_report_repository(path: Option<&str>) -> Result<Repository, GitAiError> {
    if let Some(path) = path
        && !path.trim().is_empty()
    {
        return find_repository_in_path(path);
    }
    find_repository(&[])
}

pub fn scan_report(
    repo: &Repository,
    options: &ReportOptions,
) -> Result<ReportDocument, GitAiError> {
    let commit_shas = resolve_commits(repo, options)?;
    let notes = commits_with_authorship_notes(repo, &commit_shas)?;
    let ignore_patterns = effective_ignore_patterns(repo, &options.ignore_patterns, &[]);

    let mut commits = Vec::new();
    let mut summary = ReportSummary::default();
    let mut tool_model_breakdown = BTreeMap::new();

    for sha in &commit_shas {
        let stats = stats_for_commit_stats(repo, sha, &ignore_patterns)?;
        summary.add_commit_stats(&stats);
        merge_tool_model_stats(&mut tool_model_breakdown, &stats);
        commits.push(report_commit(repo, sha, notes.contains(sha), stats)?);
    }

    let range_info = report_range_info(options, &commit_shas, notes.len());
    let ratios = calculate_ratios(&summary);

    Ok(ReportDocument {
        schema_version: REPORT_SCHEMA_VERSION.to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        repo: report_repo_info(repo)?,
        range: range_info,
        summary,
        ratios,
        tool_model_breakdown,
        commits,
    })
}

pub fn resolve_commits(
    repo: &Repository,
    options: &ReportOptions,
) -> Result<Vec<String>, GitAiError> {
    if let Some(range) = options.range.as_deref() {
        let (from, to) = range.split_once("..").ok_or_else(|| {
            GitAiError::Generic("Invalid --range format. Expected <from>..<to>".to_string())
        })?;
        if from.trim().is_empty() || to.trim().is_empty() {
            return Err(GitAiError::Generic(
                "Invalid --range format. Both sides are required".to_string(),
            ));
        }
        return rev_list(repo, &[range.to_string()], options);
    }

    if let Some(branch) = options.branch.as_deref() {
        let mut rev_args = vec![branch.to_string()];
        if let Ok(upstream) = upstream_for_branch(repo, branch)
            && let Some(upstream) = upstream
        {
            rev_args = vec![format!("{}..{}", upstream, branch)];
        }
        return rev_list(repo, &rev_args, options);
    }

    if options.since.is_some() || options.until.is_some() {
        return rev_list(repo, &["HEAD".to_string()], options);
    }

    let head = repo.head()?.target()?;
    Ok(vec![head])
}

fn rev_list(
    repo: &Repository,
    rev_args: &[String],
    options: &ReportOptions,
) -> Result<Vec<String>, GitAiError> {
    let mut args = repo.global_args_for_exec();
    args.push("rev-list".to_string());
    args.push("--reverse".to_string());
    if let Some(since) = options.since.as_deref() {
        args.push(format!("--since={}", since));
    }
    if let Some(until) = options.until.as_deref() {
        args.push(format!("--until={}", until));
    }
    args.extend(rev_args.iter().cloned());

    let output = exec_git(&args)?;
    let stdout = String::from_utf8(output.stdout)?;
    let commits = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    if commits.is_empty() {
        return Err(GitAiError::Generic(
            "No commits matched the requested report scope".to_string(),
        ));
    }

    Ok(commits)
}

fn upstream_for_branch(repo: &Repository, branch: &str) -> Result<Option<String>, GitAiError> {
    let mut args = repo.global_args_for_exec();
    args.push("rev-parse".to_string());
    args.push("--abbrev-ref".to_string());
    args.push(format!("{}@{{upstream}}", branch));
    match exec_git(&args) {
        Ok(output) => Ok(Some(String::from_utf8(output.stdout)?.trim().to_string())),
        Err(GitAiError::GitCliError { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

fn report_commit(
    repo: &Repository,
    sha: &str,
    has_authorship_note: bool,
    stats: CommitStats,
) -> Result<ReportCommit, GitAiError> {
    let commit = repo.revparse_single(sha)?.peel_to_commit()?;
    let author = commit.author()?;
    let author_name = author.name().unwrap_or("Unknown");
    let author_email = author.email().unwrap_or("");
    let author_text = if author_email.is_empty() {
        author_name.to_string()
    } else {
        format!("{} <{}>", author_name, author_email)
    };
    let author_time = chrono::DateTime::<chrono::Utc>::from_timestamp(author.when().seconds(), 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "1970-01-01T00:00:00+00:00".to_string());

    Ok(ReportCommit {
        sha: sha.to_string(),
        author: author_text,
        author_time,
        subject: commit.summary()?,
        has_authorship_note,
        stats,
    })
}

fn report_repo_info(repo: &Repository) -> Result<ReportRepoInfo, GitAiError> {
    let workdir = repo
        .workdir()
        .ok()
        .map(|path| path.to_string_lossy().to_string());
    let head_commit = repo.head().ok().and_then(|h| h.target().ok());
    let branch = repo.head().ok().and_then(|h| h.shorthand().ok());
    let remote_url_hash = crate::repo_url::repository_identifier(repo)?
        .map(|identifier| hash_repository_identifier(&identifier));

    Ok(ReportRepoInfo {
        workdir,
        remote_url_hash,
        branch,
        head_commit,
    })
}

fn default_remote_url(repo: &Repository) -> Result<Option<String>, GitAiError> {
    let Some(default_remote) = repo.get_default_remote()? else {
        return Ok(None);
    };
    Ok(repo
        .remotes_with_urls()?
        .into_iter()
        .find(|(name, _)| name == &default_remote)
        .map(|(_, url)| url))
}

fn hash_repository_identifier(identifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(identifier.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

fn report_range_info(
    options: &ReportOptions,
    commits: &[String],
    commits_with_authorship: usize,
) -> ReportRangeInfo {
    let (mode, from, to) = if let Some(range) = options.range.as_deref() {
        let (from, to) = range.split_once("..").unwrap_or((range, ""));
        (
            ReportRangeMode::Range,
            Some(from.to_string()),
            Some(to.to_string()),
        )
    } else if let Some(branch) = options.branch.as_deref() {
        (ReportRangeMode::Branch, None, Some(branch.to_string()))
    } else if options.since.is_some() || options.until.is_some() {
        (ReportRangeMode::Date, None, Some("HEAD".to_string()))
    } else {
        (ReportRangeMode::Head, None, commits.first().cloned())
    };

    ReportRangeInfo {
        mode,
        from,
        to,
        since: options.since.clone(),
        until: options.until.clone(),
        commit_count: commits.len(),
        commits_with_authorship,
        commits_without_authorship: commits.len().saturating_sub(commits_with_authorship),
    }
}

fn merge_tool_model_stats(
    target: &mut BTreeMap<String, crate::authorship::stats::ToolModelHeadlineStats>,
    stats: &CommitStats,
) {
    for (key, value) in &stats.tool_model_breakdown {
        let entry = target.entry(key.clone()).or_default();
        entry.ai_additions += value.ai_additions;
        entry.mixed_additions += value.mixed_additions;
        entry.ai_accepted += value.ai_accepted;
        entry.total_ai_additions += value.total_ai_additions;
        entry.total_ai_deletions += value.total_ai_deletions;
        entry.time_waiting_for_ai += value.time_waiting_for_ai;
    }
}

// ---------------------------------------------------------------------------
// Simplified project summary: all-history scan + per-developer aggregation
// ---------------------------------------------------------------------------

pub fn build_project_summary(
    repo: &Repository,
    options: &ReportOptions,
) -> Result<ProjectSummaryReport, GitAiError> {
    // Force full-history: scan all commits reachable from HEAD
    let _all_options = ReportOptions {
        repo_path: options.repo_path.clone(),
        range: None,
        branch: None,
        since: None,
        until: None,
        ignore_patterns: options.ignore_patterns.clone(),
    };
    // Use --range with a special sentinel to get ALL commits
    let commit_shas = all_history_commits(repo)?;
    let _notes = commits_with_authorship_notes(repo, &commit_shas)?;
    let ignore_patterns = effective_ignore_patterns(repo, &options.ignore_patterns, &[]);

    // Accumulate per-developer stats
    let mut dev_map: BTreeMap<String, DeveloperAccum> = BTreeMap::new();
    let mut total_ai: u32 = 0;
    let mut total_human: u32 = 0;

    for sha in &commit_shas {
        let stats = stats_for_commit_stats(repo, sha, &ignore_patterns)?;
        let commit = repo.revparse_single(sha)?.peel_to_commit()?;
        let author = commit.author()?;
        let author_name = author.name().unwrap_or("Unknown").to_string();
        let author_email = author.email().unwrap_or("").to_string();

        // Use email as the dedup key so same person with different name spellings merges
        let key = if author_email.is_empty() {
            author_name.clone()
        } else {
            author_email.clone()
        };

        let acc = dev_map.entry(key).or_insert_with(|| DeveloperAccum {
            name: author_name,
            email: author_email,
            ..Default::default()
        });
        acc.commits += 1;
        acc.added_lines += stats.git_diff_added_lines;
        acc.ai_additions += stats.ai_additions;
        acc.human_additions += stats.human_additions + stats.unknown_additions;

        total_ai += stats.ai_additions;
        total_human += stats.human_additions + stats.unknown_additions;
    }

    // Build developer summaries sorted by added_lines descending
    let mut developers: Vec<DeveloperSummary> = dev_map
        .into_values()
        .map(|acc| {
            let total = acc.ai_additions + acc.human_additions;
            let ai_ratio = if total > 0 {
                acc.ai_additions as f64 / total as f64
            } else {
                0.0
            };
            let human_ratio = if total > 0 {
                acc.human_additions as f64 / total as f64
            } else {
                0.0
            };
            DeveloperSummary {
                name: acc.name,
                email: acc.email,
                commits: acc.commits,
                added_lines: acc.added_lines,
                ai_additions: acc.ai_additions,
                human_additions: acc.human_additions,
                ai_ratio,
                human_ratio,
            }
        })
        .collect();
    developers.sort_by_key(|developer| std::cmp::Reverse(developer.added_lines));

    // Project-level ratios
    let total = total_ai + total_human;
    let project_ratios = ProjectRatios {
        ai: if total > 0 {
            total_ai as f64 / total as f64
        } else {
            0.0
        },
        human: if total > 0 {
            total_human as f64 / total as f64
        } else {
            0.0
        },
    };

    // Project name & git URL
    let project_name = derive_project_name(repo)?;
    let git_url = resolve_git_url(repo)?;

    Ok(ProjectSummaryReport {
        project_name,
        git_url,
        branch: repo.head().ok().and_then(|h| h.shorthand().ok()),
        total_commits: commit_shas.len(),
        developers,
        project_ratios,
        organization: None,
        department: None,
        reporter_name: read_git_config(repo, "user.name"),
        reporter_email: read_git_config(repo, "user.email"),
        report_period: None,
    })
}

#[derive(Default)]
struct DeveloperAccum {
    name: String,
    email: String,
    commits: usize,
    added_lines: u32,
    ai_additions: u32,
    human_additions: u32,
}

fn all_history_commits(repo: &Repository) -> Result<Vec<String>, GitAiError> {
    let mut args = repo.global_args_for_exec();
    args.push("rev-list".to_string());
    args.push("--reverse".to_string());
    args.push("HEAD".to_string());

    let output = exec_git(&args)?;
    let stdout = String::from_utf8(output.stdout)?;
    let commits = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    if commits.is_empty() {
        return Err(GitAiError::Generic(
            "No commits found in repository".to_string(),
        ));
    }

    Ok(commits)
}

fn derive_project_name(repo: &Repository) -> Result<String, GitAiError> {
    // Try to extract project name from remote URL, fall back to directory name
    if let Ok(Some(url)) = default_remote_url(repo) {
        // Extract the last path segment, stripping .git suffix
        let name = url
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .rsplit('/')
            .next()
            .unwrap_or(&url)
            .to_string();
        if !name.is_empty() {
            return Ok(name);
        }
    }
    // Fallback: use directory name of the workdir
    let name = repo
        .workdir()
        .ok()
        .and_then(|wd| wd.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "unknown".to_string());
    Ok(name)
}

fn resolve_git_url(repo: &Repository) -> Result<Option<String>, GitAiError> {
    default_remote_url(repo)
}

/// 读取 git config 中的指定键，失败时返回 None
fn read_git_config(repo: &Repository, key: &str) -> Option<String> {
    let mut args = repo.global_args_for_exec();
    args.push("config".to_string());
    args.push("--get".to_string());
    args.push(key.to_string());
    exec_git(&args)
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
