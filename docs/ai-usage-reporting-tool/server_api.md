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

## Enterprise API Pagination

The enterprise server uses cursor pagination for list and dashboard aggregate endpoints that can grow over time. Existing clients can ignore the `pagination` field and keep reading the original top-level list fields.

Request parameters:

- `limit`: optional page size. The server clamps values to the endpoint limit.
- `cursor`: optional opaque cursor returned from the previous response. Clients should not parse it or persist it as a permanent bookmark.

Response metadata:

```json
{
  "pagination": {
    "limit": 25,
    "has_more": true,
    "next_cursor": "opaque-cursor"
  }
}
```

First page example:

```bash
curl -sS "$BASE_URL/api/v1/aggregate/developers?limit=25" \
  -H "Authorization: Bearer $TOKEN"
```

Next page example:

```bash
curl -sS "$BASE_URL/api/v1/aggregate/developers?limit=25&cursor=$NEXT_CURSOR" \
  -H "Authorization: Bearer $TOKEN"
```

Paginated enterprise endpoints:

| Endpoint | List field | Notes |
|---|---|---|
| `GET /api/v1/audit-log` | `entries` | Supports existing filters plus `limit` and `cursor`. |
| `GET /api/admin/cas-access-log` | `entries` | Supports existing filters plus `limit` and `cursor`. |
| `GET /api/admin/users/list` | `users` | Users remain sorted by newest first. |
| `GET /api/admin/api-keys` | `api_keys` | Revoked keys are excluded. |
| `GET /api/admin/users/{id}/api-keys` | `api_keys` | Same pagination contract as the global API key list. |
| `GET /api/admin/organizations/list` | `organizations` | `include_personal` remains available. |
| `GET /api/admin/departments` | `departments` | `org_id` remains available. |
| `GET /api/v1/aggregate/pull-requests` | `pull_requests` | `summary` is computed over the full filtered set, not only the current page. |
| `GET /api/v1/aggregate/organizations` | `organizations` | Dashboard aggregate list. |
| `GET /api/v1/aggregate/departments` | `departments` | Dashboard aggregate list. |
| `GET /api/v1/aggregate/developers` | `developers` | Dashboard aggregate list. |
| `GET /api/v1/aggregate/projects` | `projects` | Dashboard aggregate list. |
| `GET /api/v1/aggregate/tools` | `tools` | Dashboard aggregate list. |

Time-series endpoints are bounded by range rather than cursor pagination:

- `GET /api/v1/aggregate/trends` defaults to a bounded range and rejects ranges that produce too many buckets.
- `GET /api/v1/ai-code-persistence` supports `since` and `until`, and defaults to the most recent year.
- `GET /api/v1/ai-code-lifecycle` limits CI and alert child lists and returns `truncated` / `truncation` when data is clipped.

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
