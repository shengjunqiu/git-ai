use crate::error::GitAiError;
use crate::report::model::{ProjectSummaryReport, ReportDocument};
use std::io::Write;
use std::path::Path;

pub fn export_json(report: &ReportDocument, output: Option<&Path>) -> Result<String, GitAiError> {
    let content = serde_json::to_string_pretty(report)?;
    write_or_print(&content, output)?;
    Ok(content)
}

pub fn export_csv(report: &ReportDocument, output: Option<&Path>) -> Result<String, GitAiError> {
    let mut content = String::new();
    content.push_str("repo_hash,branch,commit_sha,author,author_time,subject,has_authorship_note,git_diff_added_lines,git_diff_deleted_lines,ai_additions,human_additions,mixed_additions,unknown_additions,ai_accepted,total_ai_additions,total_ai_deletions,time_waiting_for_ai\n");

    let repo_hash = report.repo.remote_url_hash.as_deref().unwrap_or("");
    let branch = report.repo.branch.as_deref().unwrap_or("");

    for commit in &report.commits {
        let stats = &commit.stats;
        let fields = [
            repo_hash.to_string(),
            branch.to_string(),
            commit.sha.clone(),
            commit.author.clone(),
            commit.author_time.clone(),
            commit.subject.clone(),
            commit.has_authorship_note.to_string(),
            stats.git_diff_added_lines.to_string(),
            stats.git_diff_deleted_lines.to_string(),
            stats.ai_additions.to_string(),
            stats.human_additions.to_string(),
            stats.mixed_additions.to_string(),
            stats.unknown_additions.to_string(),
            stats.ai_accepted.to_string(),
            stats.total_ai_additions.to_string(),
            stats.total_ai_deletions.to_string(),
            stats.time_waiting_for_ai.to_string(),
        ];
        content.push_str(
            &fields
                .iter()
                .map(|field| csv_escape(field))
                .collect::<Vec<_>>()
                .join(","),
        );
        content.push('\n');
    }

    write_or_print(&content, output)?;
    Ok(content)
}

fn write_or_print(content: &str, output: Option<&Path>) -> Result<(), GitAiError> {
    if let Some(path) = output {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(path)?;
        file.write_all(content.as_bytes())?;
    } else {
        println!("{}", content);
    }
    Ok(())
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

// ---------------------------------------------------------------------------
// Simplified project summary export
// ---------------------------------------------------------------------------

pub fn export_summary_json(
    summary: &ProjectSummaryReport,
    output: Option<&Path>,
) -> Result<String, GitAiError> {
    let content = serde_json::to_string_pretty(summary)?;
    write_or_print(&content, output)?;
    Ok(content)
}

pub fn export_summary_csv(
    summary: &ProjectSummaryReport,
    output: Option<&Path>,
) -> Result<String, GitAiError> {
    let mut content = String::new();
    content.push_str("project_name,git_url,branch,developer,developer_email,commits,added_lines,ai_additions,human_additions,ai_ratio,human_ratio,project_ai_ratio,project_human_ratio\n");

    let project_name = &summary.project_name;
    let git_url = summary.git_url.as_deref().unwrap_or("");
    let branch = summary.branch.as_deref().unwrap_or("");

    for dev in &summary.developers {
        let fields = [
            project_name.clone(),
            git_url.to_string(),
            branch.to_string(),
            dev.name.clone(),
            dev.email.clone(),
            dev.commits.to_string(),
            dev.added_lines.to_string(),
            dev.ai_additions.to_string(),
            dev.human_additions.to_string(),
            format!("{:.4}", dev.ai_ratio),
            format!("{:.4}", dev.human_ratio),
            format!("{:.4}", summary.project_ratios.ai),
            format!("{:.4}", summary.project_ratios.human),
        ];
        content.push_str(
            &fields
                .iter()
                .map(|field| csv_escape(field))
                .collect::<Vec<_>>()
                .join(","),
        );
        content.push('\n');
    }

    write_or_print(&content, output)?;
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::csv_escape;

    #[test]
    fn csv_escape_quotes_commas_and_quotes() {
        assert_eq!(csv_escape("hello"), "hello");
        assert_eq!(csv_escape("hello, world"), "\"hello, world\"");
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
    }
}
