//! Bundle share page handler
//!
//! Provides the public share page for viewing a bundle's content.

use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse};
use uuid::Uuid;

use crate::error::AppError;
use crate::routes::AppState;

/// GET /bundle/{id} — Public bundle share page
pub async fn view_bundle(
    State(state): State<AppState>,
    Path(bundle_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let row: Option<(String, serde_json::Value, i32, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT title, data, view_count, created_at FROM bundles WHERE id = $1"
    )
    .bind(bundle_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let (title, data, view_count, created_at) = match row {
        Some(r) => r,
        None => return Err(AppError::NotFound("Bundle not found".into())),
    };

    // Increment view count
    sqlx::query("UPDATE bundles SET view_count = view_count + 1 WHERE id = $1")
        .bind(bundle_id)
        .execute(&state.db)
        .await
        .ok();

    // Extract prompts and files from data
    let prompts_count = data.get("prompts")
        .and_then(|p| p.as_object())
        .map(|o| o.len())
        .unwrap_or(0);

    let files_count = data.get("files")
        .and_then(|f| f.as_object())
        .map(|o| o.len())
        .unwrap_or(0);

    // Build files HTML
    let files_html = if let Some(files) = data.get("files").and_then(|f| f.as_object()) {
        let mut html = String::new();
        for (path, record) in files {
            let diff = record.get("diff").and_then(|d| d.as_str()).unwrap_or("");
            let diff_escaped = html_escape(diff);
            html.push_str(&format!(
                r#"<div class="file-card">
                    <div class="file-header">{}</div>
                    <pre class="diff">{}</pre>
                </div>"#,
                html_escape(path),
                diff_escaped,
            ));
        }
        html
    } else {
        "<p>No files in this bundle.</p>".into()
    };

    let title_escaped = html_escape(&title);
    let date = created_at.format("%Y-%m-%d %H:%M UTC").to_string();

    Ok(Html(format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>git-ai Bundle — {title_escaped}</title>
    <style>
        :root {{ font-size: 112.5%; }}
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
                background: #0f172a; color: #e2e8f0; }}
        .container {{ max-width: 960px; margin: 0 auto; padding: 2rem; }}
        h1 {{ font-size: 1.75rem; margin-bottom: 0.5rem; }}
        .logo {{ color: #818cf8; font-weight: 800; }}
        .meta {{ color: #64748b; font-size: 0.85rem; margin-bottom: 2rem; }}
        .stats {{ display: flex; gap: 1.5rem; margin-bottom: 2rem; }}
        .stat {{ background: #1e293b; border: 1px solid #334155; border-radius: 8px;
                 padding: 0.75rem 1.25rem; }}
        .stat-label {{ color: #94a3b8; font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.05em; }}
        .stat-value {{ font-size: 1.25rem; font-weight: 700; color: #818cf8; }}
        .file-card {{ background: #1e293b; border: 1px solid #334155; border-radius: 8px;
                      margin-bottom: 1rem; overflow: hidden; }}
        .file-header {{ padding: 0.75rem 1rem; border-bottom: 1px solid #334155;
                        font-family: monospace; font-size: 0.85rem; color: #818cf8; }}
        .diff {{ padding: 1rem; font-family: monospace; font-size: 0.8rem;
                 white-space: pre-wrap; word-break: break-all; overflow-x: auto;
                 color: #94a3b8; max-height: 400px; overflow-y: auto; }}
    </style>
</head>
<body>
    <div class="container">
        <h1><span class="logo">git-ai</span> Bundle</h1>
        <p class="meta">{title_escaped} · Created {date} · {view_count} views</p>
        <div class="stats">
            <div class="stat"><div class="stat-label">Prompts</div><div class="stat-value">{prompts_count}</div></div>
            <div class="stat"><div class="stat-label">Files</div><div class="stat-value">{files_count}</div></div>
        </div>
        {files_html}
    </div>
</body>
</html>"##,
    )))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
