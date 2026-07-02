# Git AI Usage Reporting Tool Development Plan

## 1. Goal

Build a reporting tool that helps developers and teams understand AI-assisted code contribution ratios for projects tracked by `git-ai`.

The tool should let a local user select a Git project, scan commits and `refs/notes/ai`, generate AI versus human code statistics, export reports, and eventually upload sanitized aggregate data to a server.

The first deliverable is a CLI tool. Server and desktop features should be designed now but implemented later.

## 2. Product Scope

### Phase 1: CLI Tool

The CLI should:

- Accept a local Git repository path.
- Accept a commit range, branch, or time window.
- Read `git-ai` authorship data from Git notes.
- Generate project-level summary statistics.
- Generate per-commit statistics.
- Generate tool/model breakdowns.
- Export reports as JSON and CSV.
- Keep an upload command/interface, but implement it as a stub or optional feature.

The CLI should not require a server to be useful.

### Phase 2: Server

The server should:

- Receive sanitized report payloads from the CLI.
- Store project, user, commit, and summary statistics.
- Deduplicate uploaded data by repository identity and commit SHA.
- Provide APIs for dashboards and future desktop clients.
- Avoid storing source code by default.
- Avoid storing raw prompts/transcripts by default.

### Phase 3: Desktop App

The desktop app should:

- Let users choose repositories through a UI.
- Show AI/human/mixed/unknown contribution charts.
- Support export and upload.
- Show privacy preview before upload.
- Reuse the same scanning logic as the CLI where possible.

## 3. Key Concepts

### Authorship Source

The authoritative source is Git AI authorship notes:

```text
refs/notes/ai
```

Each note is attached to a commit and contains:

- File-level attestation section.
- Prompt/tool/model metadata.
- AI accepted lines.
- AI generated/deleted totals.
- Known-human attribution when available.

### Statistics Scope

The tool should not treat statistics as naturally daily or weekly. The underlying unit is a commit.

Supported scopes:

- Single commit.
- Commit range.
- Branch range.
- Since/until date range converted into commit list.
- Optional future grouping by day/week/month after commit-level data is collected.

### Contribution Categories

Reports should use these categories:

- `ai`: lines committed with AI attribution.
- `human`: lines committed with known-human attribution.
- `mixed`: AI-authored lines that were later modified by a human before commit.
- `unknown`: lines without explicit attribution.

Unknown does not mean human. It means untracked by Git AI.

## 4. CLI Design

### Proposed Binary Name

Use a new command namespace inside this repository first:

```bash
git-ai report ...
```

If keeping it separate is simpler during development, use:

```bash
git-ai-report ...
```

Recommendation for this codebase: add a `report` subcommand to `git-ai`, because it can reuse existing repository, authorship, stats, and config modules.

### Commands

#### Scan

```bash
git-ai report scan [repo-path]
git-ai report scan [repo-path] --range main~100..main
git-ai report scan [repo-path] --since 30d
git-ai report scan [repo-path] --since 2026-01-01 --until 2026-04-21
git-ai report scan [repo-path] --branch main
```

Behavior:

- Resolve repository.
- Resolve commit set.
- Fetch local `refs/notes/ai` data only.
- Compute summary and per-commit stats.
- Print a concise terminal summary.
- Optionally write JSON output.

#### Export

```bash
git-ai report export [repo-path] --range main~100..main --format json --output report.json
git-ai report export [repo-path] --since 30d --format csv --output report.csv
```

Behavior:

- Runs scan.
- Writes selected output format.
- Supported MVP formats: `json`, `csv`.
- Future formats: `xlsx`, `html`, `pdf`.

#### Upload

```bash
git-ai report upload [report.json] --server https://example.com
git-ai report upload [repo-path] --range main~100..main --server https://example.com
```

MVP behavior:

- Validate payload.
- Print what would be uploaded.
- Return a clear message that upload transport is reserved/stubbed unless server config is enabled.

Future behavior:

- POST sanitized payload to server.
- Include auth token if configured.
- Retry safely.
- Store upload receipt locally.

## 5. Report Data Model

### JSON Report

```json
{
  "schema_version": "git-ai-report/1.0.0",
  "generated_at": "2026-04-21T00:00:00Z",
  "tool_version": "1.3.2",
  "repo": {
    "workdir": "/path/to/repo",
    "remote_url_hash": "sha256:...",
    "branch": "main",
    "head_commit": "..."
  },
  "range": {
    "mode": "range",
    "from": "main~100",
    "to": "main",
    "commit_count": 100
  },
  "summary": {
    "git_diff_added_lines": 0,
    "git_diff_deleted_lines": 0,
    "ai_additions": 0,
    "human_additions": 0,
    "mixed_additions": 0,
    "unknown_additions": 0,
    "ai_accepted": 0,
    "total_ai_additions": 0,
    "total_ai_deletions": 0,
    "time_waiting_for_ai": 0
  },
  "ratios": {
    "ai": 0.0,
    "human": 0.0,
    "mixed": 0.0,
    "unknown": 0.0
  },
  "tool_model_breakdown": {},
  "commits": []
}
```

### Per-Commit Entry

```json
{
  "sha": "...",
  "author": "Name <email>",
  "author_time": "2026-04-21T00:00:00Z",
  "subject": "commit subject",
  "has_authorship_note": true,
  "stats": {
    "git_diff_added_lines": 0,
    "git_diff_deleted_lines": 0,
    "ai_additions": 0,
    "human_additions": 0,
    "mixed_additions": 0,
    "unknown_additions": 0
  }
}
```

### CSV Report

MVP CSV columns:

```text
repo_hash,branch,commit_sha,author,author_time,subject,has_authorship_note,git_diff_added_lines,git_diff_deleted_lines,ai_additions,human_additions,mixed_additions,unknown_additions,ai_accepted,total_ai_additions,total_ai_deletions,time_waiting_for_ai
```

## 6. Privacy Design

Default upload/export payloads should not include:

- Source code.
- Raw file contents.
- Raw prompt messages.
- Full transcript messages.
- Absolute local paths unless explicitly requested.

The CLI may include local `workdir` in local-only JSON export, but upload payloads should remove or hash it.

Remote URLs should be hashed by default:

```text
sha256(normalized_remote_url)
```

Future enterprise mode may allow clear remote URLs under explicit configuration.

## 7. Implementation Strategy

### Preferred Integration

Add the feature inside the Rust project as a new `git-ai report` command.

New modules:

```text
src/commands/report.rs
src/report/mod.rs
src/report/model.rs
src/report/scan.rs
src/report/export.rs
src/report/upload.rs
```

The command layer should parse arguments and call `src/report`.

The report layer should reuse:

- `git::repository::Repository`
- `git::refs::get_authorship`
- `git::refs::commits_with_authorship_notes`
- `authorship::stats::stats_for_commit_stats`
- `authorship::range_authorship`
- `authorship::ignore`

### MVP Shortcut

For fastest delivery, `scan` can compute per-commit stats by calling `stats_for_commit_stats` for each commit in the resolved range and summing the fields.

This is easy to verify and consistent with existing `git-ai stats`.

Later optimization can batch note reads and diff stats.

## 8. Server Architecture

### API Endpoints

MVP server endpoints:

```text
POST /api/v1/reports
GET  /api/v1/projects
GET  /api/v1/projects/{project_id}/summary
GET  /api/v1/projects/{project_id}/commits
```

### Storage Tables

Suggested tables:

```text
users
projects
report_uploads
commit_stats
tool_model_stats
```

Deduplication key:

```text
project.remote_url_hash + commit_sha
```

### Upload Rules

The server should:

- Reject invalid schema versions.
- Deduplicate commit records.
- Store upload metadata separately from commit stats.
- Keep raw uploaded JSON optionally for audit/debug, controlled by config.

## 9. Desktop Architecture

Recommended desktop approach:

- Use the CLI as the local scanning backend.
- Desktop UI invokes `git-ai report scan --json`.
- UI renders charts and export/upload controls.
- This avoids duplicating Git scanning logic.

Potential stacks:

- Tauri for Rust-native integration.
- Electron for faster web UI iteration.
- Native app later if needed.

## 10. Milestones

### Milestone 1: CLI MVP

Deliver:

- `git-ai report scan`
- `git-ai report export`
- JSON export
- CSV export
- Upload stub
- Basic tests

### Milestone 2: CLI Hardening

Deliver:

- Date filtering.
- Branch filtering.
- Better terminal summary.
- Privacy-safe upload payload.
- More tests for missing notes, empty repos, merge commits, ignored files.

### Milestone 3: Server MVP

Deliver:

- Report ingestion API.
- Database schema.
- Deduplication.
- Project summary endpoint.

### Milestone 4: Desktop MVP

Deliver:

- Repository picker.
- Local scan execution.
- Summary charts.
- Export.
- Upload preview.

