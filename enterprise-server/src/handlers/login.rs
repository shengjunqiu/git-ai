//! Dashboard login page handler
//!
//! Provides a web page where users can paste their API Key or Bearer Token
//! to authenticate and access the dashboard. Sets an HttpOnly cookie.

use axum::extract::{Form, State};
use axum::response::{Html, IntoResponse, Redirect};
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub token: String,
}

/// GET /login — Show login page
pub async fn login_page() -> Html<String> {
    Html(r##"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>git-ai — 登录</title>
    <style>
        :root { font-size: 112.5%; }
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'PingFang SC', 'Microsoft YaHei', sans-serif;
               background: #0f172a; color: #e2e8f0; min-height: 100vh;
               display: flex; align-items: center; justify-content: center; }
        .card { background: #1e293b; border: 1px solid #334155; border-radius: 16px;
                padding: 2.5rem; max-width: 480px; width: 100%; }
        h1 { font-size: 1.5rem; margin-bottom: 0.5rem; }
        .logo { color: #818cf8; font-weight: 800; }
        p { color: #94a3b8; margin-bottom: 1.5rem; font-size: 0.9rem; line-height: 1.5; }
        label { display: block; color: #94a3b8; font-size: 0.8rem; margin-bottom: 0.5rem;
                letter-spacing: 0.05em; }
        input[type="password"], input[type="text"] { width: 100%; padding: 0.75rem 1rem;
                             border-radius: 8px; border: 1px solid #334155;
                             background: #0f172a; color: #e2e8f0; font-size: 0.95rem;
                             font-family: monospace; }
        input:focus { outline: none; border-color: #6366f1; box-shadow: 0 0 0 3px rgba(99,102,241,0.2); }
        button { width: 100%; padding: 0.75rem; border: none; border-radius: 8px;
                  background: linear-gradient(135deg, #6366f1, #818cf8); color: white;
                  font-size: 1rem; font-weight: 600; cursor: pointer; margin-top: 1rem; }
        button:hover { opacity: 0.9; }
        .msg { margin-top: 1rem; padding: 0.75rem; border-radius: 8px; font-size: 0.9rem; text-align: center; }
        .msg.error { background: #7f1d1d; color: #fca5a5; }
        .hint { margin-top: 1.25rem; color: #64748b; font-size: 0.8rem; line-height: 1.6; }
        .hint code { background: #0f172a; padding: 0.15rem 0.4rem; border-radius: 4px; color: #94a3b8; font-size: 0.78rem; }
    </style>
</head>
<body>
    <div class="card">
        <h1><span class="logo">git-ai</span> 仪表盘登录</h1>
        <p>请输入您的 API 密钥或 Bearer 令牌以访问仪表盘。</p>
        <form id="login-form" method="POST" action="/login">
            <label for="token">API 密钥或 Bearer 令牌</label>
            <input type="password" id="token" name="token"
                   placeholder="gai_... 或 eyJ..." required autofocus />
            <button type="submit">登 录</button>
        </form>
        <div id="message"></div>
        <div class="hint">
            <strong>如何获取令牌：</strong><br/>
            1. 运行 <code>git-ai login --server http://localhost:8080</code><br/>
            2. 在浏览器中完成设备授权<br/>
            3. CLI 将输出访问令牌 — 请粘贴到上方输入框<br/>
            <br/>
            或使用通过 <code>POST /api/admin/api-keys</code> 创建的 API 密钥
        </div>
    </div>
</body>
</html>"##.into())
}

/// POST /login — Process login form submission
pub async fn login_submit(
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Result<impl IntoResponse, AppError> {
    let token = form.token.trim();

    if token.is_empty() {
        return Ok(login_error_page("令牌不能为空。").into_response());
    }

    // Try as Bearer Token (JWT)
    if token.starts_with("eyJ") {
        match crate::auth::jwt::validate_access_token(token, &state.config) {
            Ok(_claims) => {
                let cookie = format!(
                    "access_token={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=3600",
                    token
                );
                let mut response = Redirect::to("/me").into_response();
                response.headers_mut().insert(
                    axum::http::header::SET_COOKIE,
                    cookie.parse().unwrap(),
                );
                return Ok(response);
            }
            Err(e) => {
                return Ok(login_error_page(&format!("令牌无效：{}", e)).into_response());
            }
        }
    }

    // Try as API Key (gai_...)
    if token.starts_with("gai_") {
        let key_hash = crate::auth::jwt::hash_token(token);

        let row: Option<(uuid::Uuid, Option<uuid::Uuid>, Vec<String>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
            "SELECT user_id, org_id, scopes, expires_at \
             FROM api_keys WHERE key_hash = $1 AND revoked_at IS NULL"
        )
        .bind(&key_hash)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AppError::Database(e))?;

        if let Some((_user_id, _org_id, _scopes, expires_at)) = &row {
            if let Some(expires) = expires_at {
                if expires < &chrono::Utc::now() {
                    return Ok(login_error_page("API 密钥已过期。").into_response());
                }
            }

            // API Key valid — store it as a cookie too
            let cookie = format!(
                "api_key={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000",
                token
            );
            let mut response = Redirect::to("/me").into_response();
            response.headers_mut().insert(
                axum::http::header::SET_COOKIE,
                cookie.parse().unwrap(),
            );
            return Ok(response);
        }

        return Ok(login_error_page("API 密钥无效或已被撤销。").into_response());
    }

    Ok(login_error_page("无法识别的令牌格式。请使用 Bearer 令牌（eyJ...）或 API 密钥（gai_...）。").into_response())
}

/// GET /logout — Clear auth cookies and redirect to login
pub async fn logout() -> impl IntoResponse {
    let mut response = Redirect::to("/login").into_response();
    let clear_access = "access_token=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0".parse().unwrap();
    let clear_api = "api_key=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0".parse().unwrap();
    response.headers_mut().insert(axum::http::header::SET_COOKIE, clear_access);
    response.headers_mut().append(axum::http::header::SET_COOKIE, clear_api);
    response
}

fn login_error_page(msg: &str) -> Html<String> {
    Html(format!(r##"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>git-ai — 登录失败</title>
    <style>
        :root {{ font-size: 112.5%; }}
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'PingFang SC', 'Microsoft YaHei', sans-serif;
               background: #0f172a; color: #e2e8f0; min-height: 100vh;
               display: flex; align-items: center; justify-content: center; }}
        .card {{ background: #1e293b; border: 1px solid #334155; border-radius: 16px;
                padding: 2.5rem; max-width: 480px; width: 100%; text-align: center; }}
        h1 {{ font-size: 1.5rem; margin-bottom: 1rem; }}
        .logo {{ color: #818cf8; font-weight: 800; }}
        .error {{ background: #7f1d1d; color: #fca5a5; padding: 0.75rem; border-radius: 8px;
                  font-size: 0.9rem; margin-bottom: 1.5rem; }}
        a {{ color: #818cf8; text-decoration: none; font-weight: 600; }}
        a:hover {{ text-decoration: underline; }}
    </style>
</head>
<body>
    <div class="card">
        <h1><span class="logo">git-ai</span> 登录失败</h1>
        <div class="error">{msg}</div>
        <a href="/login">重新登录</a>
    </div>
</body>
</html>"##,
        msg = html_escape(msg),
    ))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
