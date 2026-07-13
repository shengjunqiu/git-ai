use axum::extract::{Form, OriginalUri, Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use chrono::{Duration, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::jwt;
use crate::auth::middleware::WebSessionUser;
use crate::error::AppError;
use crate::routes::AppState;

#[derive(Debug, Clone, Deserialize)]
pub struct AuthorizeParams {
    pub client_id: String,
    pub redirect_uri: String,
    pub response_type: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeForm {
    pub client_id: String,
    pub redirect_uri: String,
    pub response_type: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
    pub decision: String,
}

struct UserAuthorizeContext {
    email: String,
    org_name: Option<String>,
    department_name: Option<String>,
}

pub async fn authorize_page(
    State(state): State<AppState>,
    WebSessionUser(user_id): WebSessionUser,
    OriginalUri(uri): OriginalUri,
    Query(params): Query<AuthorizeParams>,
) -> Result<Response, AppError> {
    validate_authorize_params(&params)?;

    let Some(user_id) = user_id else {
        return Ok(Redirect::to(&login_url(
            uri.path_and_query()
                .map(|pq| pq.as_str())
                .unwrap_or("/auth/cli/authorize"),
        ))
        .into_response());
    };

    let context = load_user_context(&state, user_id).await?;
    Ok(Html(authorize_html(&params, &context)).into_response())
}

pub async fn authorize_submit(
    State(state): State<AppState>,
    WebSessionUser(user_id): WebSessionUser,
    OriginalUri(uri): OriginalUri,
    Form(form): Form<AuthorizeForm>,
) -> Result<Response, AppError> {
    let params = form.to_params();
    validate_authorize_params(&params)?;

    let Some(user_id) = user_id else {
        return Ok(Redirect::to(&login_url(
            uri.path_and_query()
                .map(|pq| pq.as_str())
                .unwrap_or("/auth/cli/authorize"),
        ))
        .into_response());
    };

    if form.decision != "authorize" {
        return Ok(Redirect::to(&callback_url(
            &params.redirect_uri,
            &[("error", "access_denied"), ("state", &params.state)],
        )?)
        .into_response());
    }

    let code = jwt::generate_refresh_token();
    let code_hash = jwt::hash_token(&code);
    let expires_at = Utc::now() + Duration::minutes(5);

    sqlx::query(
        "INSERT INTO authorization_codes \
         (code_hash, user_id, client_id, redirect_uri, code_challenge, code_challenge_method, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&code_hash)
    .bind(user_id)
    .bind(&params.client_id)
    .bind(&params.redirect_uri)
    .bind(&params.code_challenge)
    .bind(&params.code_challenge_method)
    .bind(expires_at)
    .execute(&state.db)
    .await
    .map_err(AppError::Database)?;

    crate::services::audit::spawn_log_action(
        state.db.clone(),
        crate::services::audit::AuditPayload {
            user_id: Some(user_id),
            org_id: None,
            action: "cli.authorize".to_string(),
            resource_type: Some("authorization_code".to_string()),
            resource_id: Some(code_hash),
            details: Some(serde_json::json!({"client_id": params.client_id.clone()})),
            ip_address: None,
            user_agent: None,
        },
    );

    Ok(Redirect::to(&callback_url(
        &params.redirect_uri,
        &[("code", &code), ("state", &params.state)],
    )?)
    .into_response())
}

impl AuthorizeForm {
    fn to_params(&self) -> AuthorizeParams {
        AuthorizeParams {
            client_id: self.client_id.clone(),
            redirect_uri: self.redirect_uri.clone(),
            response_type: self.response_type.clone(),
            code_challenge: self.code_challenge.clone(),
            code_challenge_method: self.code_challenge_method.clone(),
            state: self.state.clone(),
        }
    }
}

fn validate_authorize_params(params: &AuthorizeParams) -> Result<(), AppError> {
    if params.client_id != "git-ai-cli" {
        return Err(AppError::BadRequest("Invalid client_id".into()));
    }
    if params.response_type != "code" {
        return Err(AppError::BadRequest("Invalid response_type".into()));
    }
    if params.code_challenge_method != "S256" {
        return Err(AppError::BadRequest("Invalid code_challenge_method".into()));
    }
    if params.code_challenge.trim().is_empty() {
        return Err(AppError::BadRequest("code_challenge is required".into()));
    }
    if params.state.trim().is_empty() {
        return Err(AppError::BadRequest("state is required".into()));
    }

    validate_redirect_uri(&params.redirect_uri)?;
    Ok(())
}

pub(crate) fn validate_redirect_uri(redirect_uri: &str) -> Result<(), AppError> {
    let url = url::Url::parse(redirect_uri)
        .map_err(|_| AppError::BadRequest("Invalid redirect_uri".into()))?;

    if url.scheme() != "http" {
        return Err(AppError::BadRequest("Invalid redirect_uri".into()));
    }

    let host = url
        .host_str()
        .ok_or_else(|| AppError::BadRequest("Invalid redirect_uri".into()))?;
    if host != "127.0.0.1" && host != "localhost" {
        return Err(AppError::BadRequest("Invalid redirect_uri".into()));
    }

    if url.port().is_none()
        || url.path() != "/callback"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::BadRequest("Invalid redirect_uri".into()));
    }

    Ok(())
}

fn callback_url(redirect_uri: &str, pairs: &[(&str, &str)]) -> Result<String, AppError> {
    validate_redirect_uri(redirect_uri)?;
    let mut url = url::Url::parse(redirect_uri)
        .map_err(|_| AppError::BadRequest("Invalid redirect_uri".into()))?;
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in pairs {
            query.append_pair(key, value);
        }
    }
    Ok(url.to_string())
}

async fn load_user_context(
    state: &AppState,
    user_id: Uuid,
) -> Result<UserAuthorizeContext, AppError> {
    let user_row: (String, Option<Uuid>, Option<Uuid>) =
        sqlx::query_as("SELECT email, personal_org_id, default_org_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&state.db)
            .await
            .map_err(AppError::Database)?;

    let membership: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT o.name, d.name \
         FROM org_members om \
         JOIN organizations o ON o.id = om.org_id \
         LEFT JOIN departments d ON d.id = om.department_id \
         WHERE om.user_id = $1 \
         ORDER BY CASE WHEN om.org_id = $2 THEN 0 ELSE 1 END, \
                  CASE WHEN om.org_id = $3 THEN 1 ELSE 0 END, \
                  o.created_at \
         LIMIT 1",
    )
    .bind(user_id)
    .bind(user_row.2)
    .bind(user_row.1)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?;

    Ok(UserAuthorizeContext {
        email: user_row.0,
        org_name: membership.as_ref().map(|row| row.0.clone()),
        department_name: membership.and_then(|row| row.1),
    })
}

fn authorize_html(params: &AuthorizeParams, context: &UserAuthorizeContext) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>AI编码统计 — CLI 授权</title>
  <style>{styles}</style>
</head>
<body>
  <main class="auth-shell">
    <section class="auth-card">
      <div class="brand-lockup">
        <div class="brand-title"><span>AI编码统计</span></div>
        <div class="brand-subtitle">AI 代码归属分析平台</div>
      </div>
      <div class="page-kicker">CLI 授权</div>
      <h1>允许 git-ai CLI 访问</h1>
      <div class="identity-list">
        <div class="identity-row">
          <span class="identity-label">当前账号</span>
          <span class="identity-value">{email}</span>
        </div>
        <div class="identity-row">
          <span class="identity-label">组织</span>
          <span class="identity-value">{org}</span>
        </div>
        <div class="identity-row">
          <span class="identity-label">部门</span>
          <span class="identity-value">{department}</span>
        </div>
      </div>
      <form method="POST" action="/auth/cli/authorize">
        {hidden}
        <div class="auth-actions">
          <button class="btn btn-primary" type="submit" name="decision" value="authorize">授权</button>
          <button class="btn btn-secondary" type="submit" name="decision" value="cancel">取消</button>
        </div>
      </form>
    </section>
  </main>
</body>
</html>"#,
        email = html_escape(&context.email),
        org = html_escape(context.org_name.as_deref().unwrap_or("未设置")),
        department = html_escape(context.department_name.as_deref().unwrap_or("未设置")),
        hidden = hidden_fields(params),
        styles = crate::handlers::auth_pages::AUTH_PAGE_STYLES,
    )
}

fn hidden_fields(params: &AuthorizeParams) -> String {
    [
        ("client_id", params.client_id.as_str()),
        ("redirect_uri", params.redirect_uri.as_str()),
        ("response_type", params.response_type.as_str()),
        ("code_challenge", params.code_challenge.as_str()),
        (
            "code_challenge_method",
            params.code_challenge_method.as_str(),
        ),
        ("state", params.state.as_str()),
    ]
    .into_iter()
    .map(|(name, value)| {
        format!(
            r#"<input type="hidden" name="{}" value="{}" />"#,
            name,
            html_escape(value)
        )
    })
    .collect::<Vec<_>>()
    .join("\n      ")
}

fn login_url(return_to: &str) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(return_to.as_bytes()).collect();
    format!("/auth/login?return_to={}", encoded)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_localhost_callback() {
        assert!(validate_redirect_uri("http://127.0.0.1:54321/callback").is_ok());
        assert!(validate_redirect_uri("http://localhost:54321/callback").is_ok());
    }

    #[test]
    fn rejects_non_local_callback() {
        assert!(validate_redirect_uri("https://example.com/callback").is_err());
        assert!(validate_redirect_uri("http://example.com/callback").is_err());
        assert!(validate_redirect_uri("http://127.0.0.1:54321/other").is_err());
        assert!(validate_redirect_uri("http://127.0.0.1/callback").is_err());
    }
}
