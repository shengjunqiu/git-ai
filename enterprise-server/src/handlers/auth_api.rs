use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::error::AppError;
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
pub struct OrganizationsQuery {
    pub email: String,
}

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    pub email: String,
    pub name: String,
    pub password: String,
    pub confirm_password: Option<String>,
    pub org_id: Uuid,
    pub department_id: Uuid,
    pub return_to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    pub email: String,
    pub password: String,
    pub return_to: Option<String>,
}

#[derive(Debug, Serialize)]
struct AuthUserResponse {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub personal_org_id: Uuid,
    pub default_org_id: Uuid,
}

pub async fn organizations(
    State(state): State<AppState>,
    Query(query): Query<OrganizationsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let organizations =
        crate::services::registration::list_registerable_organizations(&state.db, &query.email)
            .await?;

    Ok(Json(json!({ "organizations": organizations })))
}

pub async fn departments(
    State(state): State<AppState>,
    Path(org_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let departments =
        crate::services::registration::list_departments_for_org(&state.db, org_id).await?;

    Ok(Json(json!({ "departments": departments })))
}

pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let wants_json = is_json_request(&headers);
    let req = parse_register_request(&headers, &body)?;
    let response_user = register_user(&state, &req).await?;
    let session_token =
        crate::services::sessions::create_web_session(&state.db, response_user.id).await?;

    crate::services::audit::log_action(
        &state.db,
        Some(response_user.id),
        Some(response_user.default_org_id),
        "user.register",
        Some("user"),
        Some(&response_user.id.to_string()),
        Some(json!({"email": response_user.email})),
        None,
        None,
    )
    .await
    .ok();
    crate::services::audit::log_action(
        &state.db,
        Some(response_user.id),
        Some(response_user.default_org_id),
        "org_member.create",
        Some("org_member"),
        Some(&format!(
            "{}:{}",
            response_user.id, response_user.default_org_id
        )),
        Some(json!({"role": "member"})),
        None,
        None,
    )
    .await
    .ok();

    let mut response = if wants_json {
        (StatusCode::CREATED, Json(json!({ "user": response_user }))).into_response()
    } else if let Some(return_to) = safe_return_to(req.return_to.as_deref()) {
        Redirect::to(&return_to).into_response()
    } else {
        crate::handlers::auth_pages::success_page("注册成功", "账号已创建。").into_response()
    };

    set_session_cookie(&mut response, &state, &session_token);
    Ok(response)
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let wants_json = is_json_request(&headers);
    let req = parse_login_request(&headers, &body)?;
    let email = normalize_email(&req.email)?;

    let row: Option<(Uuid, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, email, name, password_hash \
         FROM users \
         WHERE lower(email) = lower($1) AND status = 'active'",
    )
    .bind(&email)
    .fetch_optional(&state.db)
    .await
    .map_err(AppError::Database)?;

    let Some((user_id, user_email, user_name, password_hash)) = row else {
        return Err(AppError::Unauthorized("Invalid email or password".into()));
    };

    let Some(password_hash) = password_hash else {
        return Err(AppError::Unauthorized("Invalid email or password".into()));
    };

    if !crate::services::passwords::verify_password(&req.password, &password_hash)? {
        return Err(AppError::Unauthorized("Invalid email or password".into()));
    }

    let session_token = crate::services::sessions::create_web_session(&state.db, user_id).await?;

    crate::services::audit::log_action(
        &state.db,
        Some(user_id),
        None,
        "user.login",
        Some("user"),
        Some(&user_id.to_string()),
        Some(json!({"email": user_email})),
        None,
        None,
    )
    .await
    .ok();

    let mut response = if wants_json {
        Json(json!({
            "user": {
                "id": user_id,
                "email": user_email,
                "name": user_name,
            }
        }))
        .into_response()
    } else if let Some(return_to) = safe_return_to(req.return_to.as_deref()) {
        Redirect::to(&return_to).into_response()
    } else {
        crate::handlers::auth_pages::success_page("登录成功", "账号已登录。").into_response()
    };

    set_session_cookie(&mut response, &state, &session_token);
    Ok(response)
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    if let Some(session_token) =
        cookie_value(&headers, crate::services::sessions::WEB_SESSION_COOKIE)
    {
        let user_id =
            crate::services::sessions::load_web_session_user(&state.db, &session_token).await?;
        crate::services::sessions::revoke_web_session(&state.db, &session_token).await?;

        if let Some(user_id) = user_id {
            crate::services::audit::log_action(
                &state.db,
                Some(user_id),
                None,
                "user.logout",
                Some("user"),
                Some(&user_id.to_string()),
                None,
                None,
                None,
            )
            .await
            .ok();
        }
    }

    let mut response = Redirect::to("/auth/login").into_response();
    clear_session_cookie(&mut response, &state);
    Ok(response)
}

async fn register_user(
    state: &AppState,
    req: &RegisterRequest,
) -> Result<AuthUserResponse, AppError> {
    let email = normalize_email(&req.email)?;
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::BadRequest("Name is required".into()));
    }

    if let Some(confirm_password) = &req.confirm_password {
        if confirm_password != &req.password {
            return Err(AppError::BadRequest(
                "Password confirmation does not match".into(),
            ));
        }
    }

    crate::services::passwords::validate_password_strength(&req.password)?;
    crate::services::registration::validate_org_domain(&state.db, &email, req.org_id).await?;
    crate::services::registration::validate_department(&state.db, req.org_id, req.department_id)
        .await?;

    let email_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE lower(email) = lower($1))")
            .bind(&email)
            .fetch_one(&state.db)
            .await
            .map_err(AppError::Database)?;

    if email_exists {
        return Err(AppError::Conflict("Email already exists".into()));
    }

    let user_id = Uuid::new_v4();
    let personal_org_id = Uuid::new_v4();
    let personal_org_slug = format!("personal-{}", &user_id.to_string()[..8]);
    let password_hash = crate::services::passwords::hash_password(&req.password)?;

    let mut tx = state.db.begin().await.map_err(AppError::Database)?;

    sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)")
        .bind(personal_org_id)
        .bind(format!("{}'s Org", name))
        .bind(&personal_org_slug)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

    sqlx::query(
        "INSERT INTO users \
         (id, email, name, personal_org_id, password_hash, default_org_id, status) \
         VALUES ($1, $2, $3, $4, $5, $6, 'active')",
    )
    .bind(user_id)
    .bind(&email)
    .bind(&name)
    .bind(personal_org_id)
    .bind(&password_hash)
    .bind(req.org_id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    sqlx::query("INSERT INTO org_members (user_id, org_id, role) VALUES ($1, $2, 'owner')")
        .bind(user_id)
        .bind(personal_org_id)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;

    sqlx::query(
        "INSERT INTO org_members (user_id, org_id, department_id, role) \
         VALUES ($1, $2, $3, 'member') \
         ON CONFLICT (user_id, org_id) \
         DO UPDATE SET department_id = EXCLUDED.department_id",
    )
    .bind(user_id)
    .bind(req.org_id)
    .bind(req.department_id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    tx.commit().await.map_err(AppError::Database)?;

    Ok(AuthUserResponse {
        id: user_id,
        email,
        name,
        personal_org_id,
        default_org_id: req.org_id,
    })
}

fn parse_register_request(headers: &HeaderMap, body: &Bytes) -> Result<RegisterRequest, AppError> {
    if is_json_request(headers) {
        serde_json::from_slice(body).map_err(AppError::Json)
    } else {
        let fields = parse_form_body(body);
        Ok(RegisterRequest {
            email: required_field(&fields, "email")?,
            name: required_field(&fields, "name")?,
            password: required_field(&fields, "password")?,
            confirm_password: fields.get("confirm_password").cloned(),
            org_id: parse_uuid_field(&fields, "org_id")?,
            department_id: parse_uuid_field(&fields, "department_id")?,
            return_to: fields.get("return_to").cloned(),
        })
    }
}

fn parse_login_request(headers: &HeaderMap, body: &Bytes) -> Result<LoginRequest, AppError> {
    if is_json_request(headers) {
        serde_json::from_slice(body).map_err(AppError::Json)
    } else {
        let fields = parse_form_body(body);
        Ok(LoginRequest {
            email: required_field(&fields, "email")?,
            password: required_field(&fields, "password")?,
            return_to: fields.get("return_to").cloned(),
        })
    }
}

fn parse_form_body(body: &Bytes) -> HashMap<String, String> {
    url::form_urlencoded::parse(body)
        .into_owned()
        .collect::<HashMap<_, _>>()
}

fn required_field(fields: &HashMap<String, String>, name: &str) -> Result<String, AppError> {
    let value = fields
        .get(name)
        .map(|value| value.trim().to_string())
        .unwrap_or_default();

    if value.is_empty() {
        Err(AppError::BadRequest(format!("{} is required", name)))
    } else {
        Ok(value)
    }
}

fn parse_uuid_field(fields: &HashMap<String, String>, name: &str) -> Result<Uuid, AppError> {
    let value = required_field(fields, name)?;
    Uuid::parse_str(&value).map_err(|_| AppError::BadRequest(format!("Invalid {}", name)))
}

fn normalize_email(email: &str) -> Result<String, AppError> {
    crate::services::registration::email_domain(email)?;
    Ok(email.trim().to_ascii_lowercase())
}

fn is_json_request(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.starts_with("application/json"))
        .unwrap_or(false)
}

fn safe_return_to(return_to: Option<&str>) -> Option<String> {
    let return_to = return_to?.trim();
    if return_to.starts_with('/') && !return_to.starts_with("//") {
        Some(return_to.to_string())
    } else {
        None
    }
}

fn set_session_cookie(response: &mut Response, state: &AppState, session_token: &str) {
    let cookie = session_cookie_header(&state.config.base_url, session_token);
    response
        .headers_mut()
        .insert(header::SET_COOKIE, cookie.parse().unwrap());
}

fn clear_session_cookie(response: &mut Response, state: &AppState) {
    let cookie = clear_session_cookie_header(&state.config.base_url);
    response
        .headers_mut()
        .insert(header::SET_COOKIE, cookie.parse().unwrap());
}

fn session_cookie_header(base_url: &str, session_token: &str) -> String {
    format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age=2592000{}",
        crate::services::sessions::WEB_SESSION_COOKIE,
        session_token,
        secure_cookie_suffix(base_url)
    )
}

fn clear_session_cookie_header(base_url: &str) -> String {
    format!(
        "{}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
        crate::services::sessions::WEB_SESSION_COOKIE,
        secure_cookie_suffix(base_url)
    )
}

fn secure_cookie_suffix(base_url: &str) -> &'static str {
    if base_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    }
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?;
    let cookie_str = cookie_header.to_str().ok()?;
    cookie_str.split(';').find_map(|cookie| {
        let cookie = cookie.trim();
        let (cookie_name, cookie_value) = cookie.split_once('=')?;
        if cookie_name == name {
            Some(cookie_value.to_string())
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        headers
    }

    #[test]
    fn parse_register_json_request() {
        let org_id = Uuid::new_v4();
        let department_id = Uuid::new_v4();
        let body = Bytes::from(
            serde_json::json!({
                "email": "Alice@Linewell.COM",
                "name": "Alice",
                "password": "correct-horse-battery",
                "confirm_password": "correct-horse-battery",
                "org_id": org_id,
                "department_id": department_id,
                "return_to": "/auth/cli/authorize"
            })
            .to_string(),
        );

        let req = parse_register_request(&json_headers(), &body).unwrap();
        assert_eq!(req.email, "Alice@Linewell.COM");
        assert_eq!(req.name, "Alice");
        assert_eq!(req.org_id, org_id);
        assert_eq!(req.department_id, department_id);
        assert_eq!(req.return_to.as_deref(), Some("/auth/cli/authorize"));
    }

    #[test]
    fn parse_register_form_request_trims_required_fields() {
        let org_id = Uuid::new_v4();
        let department_id = Uuid::new_v4();
        let body = Bytes::from(format!(
            "email=%20alice%40linewell.com%20&name=%20Alice%20&password=secret-password&confirm_password=secret-password&org_id={}&department_id={}",
            org_id, department_id
        ));

        let req = parse_register_request(&HeaderMap::new(), &body).unwrap();
        assert_eq!(req.email, "alice@linewell.com");
        assert_eq!(req.name, "Alice");
        assert_eq!(req.org_id, org_id);
        assert_eq!(req.department_id, department_id);
    }

    #[test]
    fn parse_register_form_rejects_missing_required_field() {
        let org_id = Uuid::new_v4();
        let body = Bytes::from(format!(
            "email=alice%40linewell.com&name=Alice&password=secret-password&org_id={}",
            org_id
        ));

        assert!(parse_register_request(&HeaderMap::new(), &body).is_err());
    }

    #[test]
    fn parse_login_form_request() {
        let body = Bytes::from(
            "email=%20alice%40linewell.com%20&password=secret-password&return_to=/dashboard",
        );

        let req = parse_login_request(&HeaderMap::new(), &body).unwrap();
        assert_eq!(req.email, "alice@linewell.com");
        assert_eq!(req.password, "secret-password");
        assert_eq!(req.return_to.as_deref(), Some("/dashboard"));
    }

    #[test]
    fn safe_return_to_only_allows_local_paths() {
        assert_eq!(
            safe_return_to(Some("/auth/cli/authorize")).as_deref(),
            Some("/auth/cli/authorize")
        );
        assert!(safe_return_to(Some("https://example.com")).is_none());
        assert!(safe_return_to(Some("//example.com/path")).is_none());
        assert!(safe_return_to(Some("")).is_none());
    }

    #[test]
    fn session_cookie_uses_secure_only_for_https_base_url() {
        let https_cookie = session_cookie_header("https://git-ai.example.com", "token");
        assert!(https_cookie.contains("Secure"));
        assert!(https_cookie.contains("HttpOnly"));
        assert!(https_cookie.contains("SameSite=Lax"));

        let http_cookie = session_cookie_header("http://localhost:8080", "token");
        assert!(!http_cookie.contains("Secure"));
    }

    #[test]
    fn clear_session_cookie_expires_cookie() {
        let cookie = clear_session_cookie_header("https://git-ai.example.com");
        assert!(cookie.contains("web_session="));
        assert!(cookie.contains("Max-Age=0"));
        assert!(cookie.contains("Secure"));
    }

    #[test]
    fn cookie_value_reads_named_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            "theme=dark; web_session=session-token; other=value"
                .parse()
                .unwrap(),
        );

        assert_eq!(
            cookie_value(&headers, crate::services::sessions::WEB_SESSION_COOKIE).as_deref(),
            Some("session-token")
        );
    }
}
