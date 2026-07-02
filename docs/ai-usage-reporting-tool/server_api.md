# Git AI Usage Report Server API

This is the MVP API implemented by `git-ai report server`.

## Run Locally

```bash
git-ai report server --addr 127.0.0.1:8787 --db git-ai-report-server.sqlite
```

The server stores sanitized report payloads in SQLite. It is intended as a local/dev MVP, not a hardened production service.

## Endpoints

### `POST /api/v1/reports`

Receives a `git-ai-report/1.0.0` JSON report payload.

Example upload from the CLI:

```bash
git-ai report upload report.json --server http://127.0.0.1:8787
```

Preview only:

```bash
git-ai report upload report.json --server http://127.0.0.1:8787 --dry-run
```

Behavior:

- Rejects unsupported `schema_version`.
- Creates or updates a project by `repo.remote_url_hash`.
- Falls back to `local:<head_commit>` when no remote hash exists.
- Inserts commit stats with `(project_id, sha)` deduplication.
- Records one upload row per accepted request.

Response:

```json
{
  "project_id": 1,
  "upload_id": 1,
  "inserted_commits": 12,
  "duplicate_commits": 0
}
```

Error response:

```json
{
  "error": "Generic error: Unsupported report schema '...'"
}
```

### `GET /api/v1/projects`

Lists known projects.

Response:

```json
[
  {
    "id": 1,
    "remote_url_hash": "sha256:...",
    "branch": "main",
    "head_commit": "...",
    "commit_count": 12
  }
]
```

### `GET /api/v1/projects/{id}/summary`

Returns aggregate counters computed from stored commit rows.

### `GET /api/v1/projects/{id}/commits`

Returns per-commit counters for the project.

## Storage

Tables:

- `projects`
- `report_uploads`
- `commit_stats`
- `tool_model_stats`

Deduplication key:

```text
project_id + commit_sha
```

## Privacy

The server expects clients to send sanitized payloads. The existing CLI upload path removes `repo.workdir` before upload preparation. Report payloads do not include source code, raw prompts, or transcript content.

## HTTP Status Codes

- `200 OK`: Read request succeeded.
- `201 Created`: Report payload accepted.
- `400 Bad Request`: Payload, schema, project id, or storage error.
- `404 Not Found`: Unknown API path.
