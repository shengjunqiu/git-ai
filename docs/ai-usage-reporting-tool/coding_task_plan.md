# Git AI Usage Reporting Tool Coding Task Plan

## Phase 0: Preparation

### Task 0.1: Confirm Command Shape

Decision:

- Implement as `git-ai report ...`.
- Add a new command handler under `src/commands/report.rs`.
- Add reusable logic under `src/report/`.

Acceptance criteria:

- `git-ai report --help` prints report command help.
- Unknown report subcommands return a clear error.

### Task 0.2: Create Report Module Skeleton

Files to add:

```text
src/report/mod.rs
src/report/model.rs
src/report/scan.rs
src/report/export.rs
src/report/upload.rs
src/commands/report.rs
```

Files to update:

```text
src/lib.rs
src/commands/mod.rs
src/commands/git_ai_handlers.rs
```

Acceptance criteria:

- Project compiles.
- `git-ai report scan` reaches a stub implementation.

## Phase 1: CLI Scan MVP

### Task 1.1: Define Report Data Structures

Implement structs in `src/report/model.rs`:

- `ReportDocument`
- `ReportRepoInfo`
- `ReportRangeInfo`
- `ReportSummary`
- `ReportRatios`
- `ReportCommit`
- `ReportToolModelStats`
- `ReportOptions`

Requirements:

- Derive `Serialize`, `Deserialize`, `Debug`, `Clone`.
- Include `schema_version = "git-ai-report/1.0.0"`.
- Keep numeric counters as `u32` or `u64`.
- Keep ratios as `f64`.

Acceptance criteria:

- Unit test serializes a minimal report to JSON.
- JSON contains `schema_version`.

### Task 1.2: Resolve Repository

Implement in `src/report/scan.rs`:

```rust
pub fn resolve_report_repository(path: Option<&str>) -> Result<Repository, GitAiError>
```

Behavior:

- If path is provided, discover repository from that path.
- If no path is provided, discover from current directory.
- Support regular repos and worktrees.

Acceptance criteria:

- Test resolves current repo.
- Test fails clearly for non-repo directory.

### Task 1.3: Resolve Commit Range

Support MVP range options:

- No range: `HEAD`
- `--range <a..b>`
- `--branch <branch>` as commits reachable from branch and not from its upstream/base where possible.

Recommended first implementation:

- `HEAD`: one commit.
- `<a>..<b>`: use `git rev-list a..b`.
- Date filters can be added in Phase 2.

Implement:

```rust
pub fn resolve_commits(repo: &Repository, options: &ReportOptions) -> Result<Vec<String>, GitAiError>
```

Acceptance criteria:

- `HEAD` returns one commit.
- `HEAD~2..HEAD` returns expected commits.
- Invalid revision returns a clear error.

### Task 1.4: Compute Per-Commit Stats

For each commit:

- Call existing `stats_for_commit_stats(repo, commit_sha, ignore_patterns)`.
- Check whether authorship note exists with existing refs helpers.
- Read basic commit metadata using Git CLI.

Implement:

```rust
pub fn scan_report(repo: &Repository, options: ReportOptions) -> Result<ReportDocument, GitAiError>
```

Acceptance criteria:

- Report contains one `ReportCommit` per scanned commit.
- Each commit includes stats.
- Missing authorship notes are represented with `has_authorship_note = false`.

### Task 1.5: Aggregate Summary

Aggregate all per-commit stats:

- `git_diff_added_lines`
- `git_diff_deleted_lines`
- `ai_additions`
- `human_additions`
- `mixed_additions`
- `unknown_additions`
- `ai_accepted`
- `total_ai_additions`
- `total_ai_deletions`
- `time_waiting_for_ai`
- `tool_model_breakdown`

Calculate ratios:

```text
total = ai_additions + human_additions + unknown_additions
ai = ai_additions / total
human = human_additions / total
mixed = mixed_additions / total
unknown = unknown_additions / total
```

Use `0.0` when denominator is zero.

Acceptance criteria:

- Aggregation unit test verifies summed totals.
- Ratio unit test handles zero denominator.

### Task 1.6: Terminal Summary

Implement concise output:

```text
Repository: <name>
Range: <range>
Commits: 42
Added lines: 1000
AI: 42.0%
Human: 30.0%
Mixed: 8.0%
Unknown: 20.0%
```

Acceptance criteria:

- `git-ai report scan . --range A..B` prints summary.
- `--json` prints full JSON report instead.

## Phase 2: Export MVP

### Task 2.1: JSON Export

Implement:

```rust
pub fn export_json(report: &ReportDocument, output: Option<&Path>) -> Result<(), GitAiError>
```

Behavior:

- Pretty-print JSON.
- Write to file if `--output` is provided.
- Print to stdout otherwise.

Acceptance criteria:

- JSON file is created.
- JSON parses back into `ReportDocument`.

### Task 2.2: CSV Export

Implement:

```rust
pub fn export_csv(report: &ReportDocument, output: Option<&Path>) -> Result<(), GitAiError>
```

MVP can use manual CSV writing if all fields are safely escaped by a helper.

Columns:

```text
repo_hash,branch,commit_sha,author,author_time,subject,has_authorship_note,git_diff_added_lines,git_diff_deleted_lines,ai_additions,human_additions,mixed_additions,unknown_additions,ai_accepted,total_ai_additions,total_ai_deletions,time_waiting_for_ai
```

Acceptance criteria:

- CSV includes header.
- One row per commit.
- Subjects with commas or quotes are escaped correctly.

### Task 2.3: Export Command

Implement:

```bash
git-ai report export . --range HEAD~10..HEAD --format json --output report.json
git-ai report export . --range HEAD~10..HEAD --format csv --output report.csv
```

Acceptance criteria:

- JSON and CSV exports work.
- Invalid format gives clear error.

## Phase 3: Upload Interface Stub

### Task 3.1: Define Upload Payload

Implement sanitized conversion:

```rust
pub fn to_upload_payload(report: &ReportDocument) -> ReportDocument
```

Rules:

- Remove local absolute workdir.
- Keep `remote_url_hash`.
- Keep branch/head/range.
- Keep summary, tool/model breakdown, and commit stats.
- Do not include prompt messages or source code.

Acceptance criteria:

- Unit test verifies local path is removed or redacted.

### Task 3.2: Upload Stub Command

Implement:

```bash
git-ai report upload report.json --server https://example.com
git-ai report upload . --range HEAD~10..HEAD --server https://example.com
```

MVP behavior:

- Load or generate report.
- Build sanitized payload.
- Validate server URL.
- Print payload summary and "upload transport not enabled yet".
- Return success unless validation fails.

Acceptance criteria:

- Command exists.
- Invalid URL fails.
- Valid URL prints clear stub message.

### Task 3.3: Future Upload Trait

Add an internal trait:

```rust
pub trait ReportUploader {
    fn upload(&self, payload: &ReportDocument) -> Result<UploadResult, GitAiError>;
}
```

Implement:

- `DryRunUploader`
- Future `HttpUploader`

Acceptance criteria:

- Upload command uses `DryRunUploader`.
- Tests can mock uploader behavior.

## Phase 4: CLI Hardening

### Task 4.1: Date Filters

Add:

```bash
--since <time>
--until <time>
```

Use `git rev-list --since=<time> --until=<time>`.

Acceptance criteria:

- Date filters restrict commit list.
- Supports relative values like `30d` if compatible with existing time parsing conventions.

### Task 4.2: Ignore Patterns

Add:

```bash
--ignore <pattern>...
```

Reuse existing effective ignore logic.

Acceptance criteria:

- Ignored files do not affect totals.
- Works with exact paths and globs.

### Task 4.3: Missing Notes Diagnostics

Add warning fields:

- total commits
- commits with authorship notes
- commits without authorship notes

Terminal output should explain that unknown/untracked may mean Git AI was not installed or notes were not fetched.

Acceptance criteria:

- Report includes notes coverage.
- Terminal summary includes note coverage when below 100%.

### Task 4.4: Performance Improvements

Potential improvements:

- Batch note existence checks.
- Batch commit metadata reads.
- Avoid repeated Git subprocesses where possible.

Acceptance criteria:

- Add benchmark or integration test for 100+ commits.
- Avoid obvious O(n) expensive note show calls when note list can be batched.

## Phase 5: Tests

### Task 5.1: Unit Tests

Cover:

- Ratio calculation.
- Summary aggregation.
- CSV escaping.
- Upload sanitization.
- Range parsing.

### Task 5.2: Integration Tests

Add tests under `tests/integration/report.rs` or equivalent.

Scenarios:

- Single commit with AI lines.
- Mixed AI/human commit.
- Commit without authorship note.
- Range of multiple commits.
- JSON export.
- CSV export.
- Upload stub.

Acceptance criteria:

- Tests use `TestRepo`.
- Tests assert line-level authorship after commits where relevant.
- `task test TEST_FILTER=report` passes.

## Phase 6: Server MVP

### Task 6.1: Choose Server Location

Decision needed:

- Same repository under `server/`, or separate repository.

Recommendation:

- Start under `server/` for rapid iteration.

### Task 6.2: Define API Contract

Create OpenAPI or Markdown API spec:

```text
POST /api/v1/reports
GET /api/v1/projects
GET /api/v1/projects/{id}/summary
GET /api/v1/projects/{id}/commits
```

Acceptance criteria:

- Request and response examples exist.
- Error codes documented.

### Task 6.3: Implement Ingestion API

Server receives sanitized report payload.

Acceptance criteria:

- Valid report is accepted.
- Invalid schema version is rejected.
- Duplicate commit stats are deduplicated.

### Task 6.4: Add Storage

Tables:

- `projects`
- `report_uploads`
- `commit_stats`
- `tool_model_stats`

Acceptance criteria:

- Migration creates tables.
- Summary endpoint aggregates stored data.

## Phase 7: Desktop MVP

### Task 7.1: Select Desktop Stack

Recommendation:

- Tauri if we want Rust-native packaging.
- Electron if we want fastest web UI iteration.

### Task 7.2: Repository Picker

Desktop app should let user select a local repo directory.

Acceptance criteria:

- Selected repo path is passed to CLI scan.
- Errors display clearly.

### Task 7.3: Charts

Display:

- AI/human/mixed/unknown ratio.
- Per-tool/model breakdown.
- Commit trend.
- Notes coverage.

Acceptance criteria:

- UI renders report generated by CLI JSON.

### Task 7.4: Export and Upload Preview

Acceptance criteria:

- User can export JSON/CSV.
- User can preview sanitized upload payload.
- Upload button uses server API once available.

## Recommended First Coding Order

1. Add module skeleton and `git-ai report` command.
2. Implement report data structs.
3. Implement `HEAD` scan.
4. Implement `<a>..<b>` range scan.
5. Implement aggregation and ratios.
6. Implement terminal summary.
7. Implement JSON export.
8. Implement CSV export.
9. Implement upload stub.
10. Add integration tests.

