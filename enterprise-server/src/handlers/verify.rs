//! OAuth verification page handler
//!
//! Provides the web page where users enter their user_code to authorize
//! the device flow. This is the verification_uri shown in the terminal.
//! Deprecated for new CLI login: browser session + authorization code is the
//! primary flow. This handler remains for old client compatibility.

use axum::extract::{Form, Query, State};
use axum::response::{Html, IntoResponse};
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
pub struct VerifyQuery {
    pub user_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VerifyForm {
    pub user_code: String,
}

/// GET /verify — Show verification page (optionally pre-filled with user_code)
pub async fn verify_page(
    State(_state): State<AppState>,
    Query(query): Query<VerifyQuery>,
) -> Html<String> {
    let user_code_prefilled = query.user_code.unwrap_or_default();
    let user_code_escaped = html_escape(&user_code_prefilled);

    Html(format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>git-ai — Authorize Device</title>
    <style>
        :root {{ font-size: 112.5%; }}
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
                background: #0f172a; color: #e2e8f0; min-height: 100vh;
                display: flex; align-items: center; justify-content: center; }}
        .card {{ background: #1e293b; border: 1px solid #334155; border-radius: 16px;
                 padding: 2.5rem; max-width: 440px; width: 100%; }}
        h1 {{ font-size: 1.5rem; margin-bottom: 0.5rem; }}
        .logo {{ color: #818cf8; font-weight: 800; }}
        p {{ color: #94a3b8; margin-bottom: 1.5rem; font-size: 0.9rem; line-height: 1.5; }}
        label {{ display: block; color: #94a3b8; font-size: 0.8rem; margin-bottom: 0.5rem; text-transform: uppercase; letter-spacing: 0.05em; }}
        input[type="text"] {{ width: 100%; padding: 0.75rem 1rem; border-radius: 8px; border: 1px solid #334155;
                             background: #0f172a; color: #e2e8f0; font-size: 1.25rem; text-align: center;
                             letter-spacing: 0.15em; font-family: monospace; }}
        input[type="text"]:focus {{ outline: none; border-color: #6366f1; box-shadow: 0 0 0 3px rgba(99,102,241,0.2); }}
        button {{ width: 100%; padding: 0.75rem; border: none; border-radius: 8px;
                  background: linear-gradient(135deg, #6366f1, #818cf8); color: white;
                  font-size: 1rem; font-weight: 600; cursor: pointer; margin-top: 1rem; }}
        button:hover {{ opacity: 0.9; }}
        .msg {{ margin-top: 1rem; padding: 0.75rem; border-radius: 8px; font-size: 0.9rem; text-align: center; }}
        .msg.success {{ background: #064e3b; color: #6ee7b7; }}
        .msg.error {{ background: #7f1d1d; color: #fca5a5; }}
        .msg.info {{ background: #1e3a5f; color: #93c5fd; }}
    </style>
</head>
<body>
    <div class="card">
        <h1><span class="logo">git-ai</span> Authorize Device</h1>
        <p>Enter the code displayed in your terminal to authorize this device.</p>
        <form id="verify-form" method="POST" action="/verify">
            <label for="user_code">Device Code</label>
            <input type="text" id="user_code" name="user_code" value="{user_code_escaped}"
                   placeholder="XXXXXXXX" maxlength="16" required autofocus />
            <button type="submit">Authorize</button>
        </form>
        <div id="message"></div>
    </div>
</body>
</html>"##,
    ))
}

/// POST /verify — Process verification form submission
pub async fn verify_submit(
    State(state): State<AppState>,
    Form(form): Form<VerifyForm>,
) -> Result<impl IntoResponse, AppError> {
    let user_code = form.user_code.trim().to_uppercase();

    // Look up the device code by user_code
    let row: Option<(String, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("SELECT device_code, expires_at FROM oauth_devices WHERE user_code = $1")
            .bind(&user_code)
            .fetch_optional(&state.db)
            .await
            .map_err(|e| AppError::Database(e))?;

    let (device_code, expires_at) = match row {
        Some(r) => r,
        None => {
            return Ok(Html(format!(
                r#"<div class="msg error">Invalid code. Please check and try again.</div>
                <script>document.getElementById('message').outerHTML = document.body.querySelector('.msg').outerHTML;</script>"#
            )).into_response());
        }
    };

    if expires_at < chrono::Utc::now() {
        return Ok(Html(format!(
            r#"<div class="msg error">This code has expired. Please generate a new one.</div>"#
        ))
        .into_response());
    }

    // TODO: In a full implementation, the user would need to be logged in to authorize.
    // For now, we auto-authorize: find or create a default user.
    // In production, this should redirect to a login page first.
    let user_row: (uuid::Uuid,) =
        sqlx::query_as("SELECT id FROM users ORDER BY created_at LIMIT 1")
            .fetch_one(&state.db)
            .await
            .map_err(|_| {
                AppError::Internal("No users found. Create a user first via admin API.".into())
            })?;

    // Mark the device code as authorized
    sqlx::query(
        "UPDATE oauth_devices SET user_id = $1, authorized_at = now() WHERE device_code = $2",
    )
    .bind(user_row.0)
    .bind(&device_code)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    tracing::info!(
        "Device authorized: user_code={}, user_id={}",
        user_code,
        user_row.0
    );

    // Return success page
    Ok(Html(format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>git-ai — Authorized</title>
    <style>
        :root {{ font-size: 112.5%; }}
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
                background: #0f172a; color: #e2e8f0; min-height: 100vh;
                display: flex; align-items: center; justify-content: center; }}
        .card {{ background: #1e293b; border: 1px solid #334155; border-radius: 16px;
                 padding: 2.5rem; max-width: 440px; width: 100%; text-align: center; }}
        h1 {{ font-size: 1.5rem; margin-bottom: 1rem; }}
        .logo {{ color: #818cf8; font-weight: 800; }}
        .check {{ font-size: 3rem; margin-bottom: 1rem; }}
        p {{ color: #94a3b8; font-size: 0.9rem; line-height: 1.5; }}
    </style>
</head>
<body>
    <div class="card">
        <div class="check">✅</div>
        <h1><span class="logo">git-ai</span> Device Authorized</h1>
        <p>Your device has been successfully authorized.<br/>You can now return to your terminal.</p>
    </div>
</body>
</html>"##
    )).into_response())
}

/// HTML escape helper
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
