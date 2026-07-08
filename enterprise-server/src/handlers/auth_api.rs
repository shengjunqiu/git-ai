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
    pub org_id: Option<Uuid>,
    pub department_id: Option<Uuid>,
    pub org_slug: Option<String>,
    pub department_slug: Option<String>,
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
    pub personal_org_id: Option<Uuid>,
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
        Redirect::to("/me").into_response()
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
    let (org_id, department_id) = resolve_register_scope(state, req).await?;

    crate::services::registration::validate_org_domain(&state.db, &email, org_id).await?;
    crate::services::registration::validate_department(&state.db, org_id, department_id).await?;

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
    let password_hash = crate::services::passwords::hash_password(&req.password)?;

    let mut tx = state.db.begin().await.map_err(AppError::Database)?;

    sqlx::query(
        "INSERT INTO users \
         (id, email, name, password_hash, default_org_id, status) \
         VALUES ($1, $2, $3, $4, $5, 'active')",
    )
    .bind(user_id)
    .bind(&email)
    .bind(&name)
    .bind(&password_hash)
    .bind(org_id)
    .execute(&mut *tx)
    .await
    .map_err(map_user_insert_error)?;

    sqlx::query(
        "INSERT INTO org_members (user_id, org_id, department_id, role) \
         VALUES ($1, $2, $3, 'member') \
         ON CONFLICT (user_id, org_id) \
         DO UPDATE SET department_id = EXCLUDED.department_id",
    )
    .bind(user_id)
    .bind(org_id)
    .bind(department_id)
    .execute(&mut *tx)
    .await
    .map_err(AppError::Database)?;

    tx.commit().await.map_err(AppError::Database)?;

    Ok(AuthUserResponse {
        id: user_id,
        email,
        name,
        personal_org_id: None,
        default_org_id: org_id,
    })
}

fn map_user_insert_error(error: sqlx::Error) -> AppError {
    if let sqlx::Error::Database(database_error) = &error {
        if database_error.code().as_deref() == Some("23505")
            && database_error.constraint() == Some("users_email_key")
        {
            return AppError::Conflict("Email already exists".into());
        }
    }

    AppError::Database(error)
}

async fn resolve_register_scope(
    state: &AppState,
    req: &RegisterRequest,
) -> Result<(Uuid, Uuid), AppError> {
    let org_id = match req.org_id {
        Some(org_id) => org_id,
        None => {
            let org_slug = req
                .org_slug
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("Organization is required".into()))?
                .trim();
            if org_slug.is_empty() {
                return Err(AppError::BadRequest("Organization is required".into()));
            }
            crate::services::registration::find_org_id_by_slug(&state.db, org_slug).await?
        }
    };

    let department_id = match req.department_id {
        Some(department_id) => department_id,
        None => {
            let department_slug = req
                .department_slug
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("Department is required".into()))?
                .trim();
            if department_slug.is_empty() {
                return Err(AppError::BadRequest("Department is required".into()));
            }
            crate::services::registration::find_department_id_by_slug(
                &state.db,
                org_id,
                department_slug,
            )
            .await?
        }
    };

    Ok((org_id, department_id))
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
            org_id: parse_optional_uuid_field(&fields, "org_id")?,
            department_id: parse_optional_uuid_field(&fields, "department_id")?,
            org_slug: optional_field(&fields, "org_slug"),
            department_slug: optional_field(&fields, "department_slug"),
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

fn parse_optional_uuid_field(
    fields: &HashMap<String, String>,
    name: &str,
) -> Result<Option<Uuid>, AppError> {
    let Some(value) = optional_field(fields, name) else {
        return Ok(None);
    };
    Uuid::parse_str(&value)
        .map(Some)
        .map_err(|_| AppError::BadRequest(format!("Invalid {}", name)))
}

fn optional_field(fields: &HashMap<String, String>, name: &str) -> Option<String> {
    let value = fields.get(name)?.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
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
        assert_eq!(req.org_id, Some(org_id));
        assert_eq!(req.department_id, Some(department_id));
        assert_eq!(req.return_to.as_deref(), Some("/auth/cli/authorize"));
    }

    #[test]
    fn parse_register_form_request_trims_required_fields() {
        let body = Bytes::from(
            "email=%20alice%40linewell.com%20&name=%20Alice%20&password=secret-password&confirm_password=secret-password&org_slug=%20linewell.com%20&department_slug=%20technology-center%20",
        );

        let req = parse_register_request(&HeaderMap::new(), &body).unwrap();
        assert_eq!(req.email, "alice@linewell.com");
        assert_eq!(req.name, "Alice");
        assert_eq!(req.org_id, None);
        assert_eq!(req.department_id, None);
        assert_eq!(req.org_slug.as_deref(), Some("linewell.com"));
        assert_eq!(req.department_slug.as_deref(), Some("technology-center"));
    }

    #[test]
    fn parse_register_form_supports_legacy_uuid_fields() {
        let org_id = Uuid::new_v4();
        let department_id = Uuid::new_v4();
        let body = Bytes::from(format!(
            "email=alice%40linewell.com&name=Alice&password=secret-password&org_id={}&department_id={}",
            org_id, department_id
        ));

        let req = parse_register_request(&HeaderMap::new(), &body).unwrap();
        assert_eq!(req.org_id, Some(org_id));
        assert_eq!(req.department_id, Some(department_id));
    }

    #[test]
    fn parse_register_form_does_not_default_org_slug() {
        let body = Bytes::from(
            "email=alice%40linewell.com&name=Alice&password=secret-password&department_slug=rd-center",
        );

        let req = parse_register_request(&HeaderMap::new(), &body).unwrap();
        assert_eq!(req.org_slug.as_deref(), None);
        assert_eq!(req.department_slug.as_deref(), Some("rd-center"));
    }

    #[test]
    fn parse_register_form_rejects_missing_required_field() {
        let body = Bytes::from("email=alice%40linewell.com&name=Alice");

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

    struct TestDatabase {
        state: AppState,
        admin_pool: sqlx::PgPool,
        db_name: String,
    }

    impl TestDatabase {
        async fn new() -> anyhow::Result<Option<Self>> {
            let database_url = test_database_url();
            let db_name = unique_test_database_name();
            let admin_url = database_url_for_database(&database_url, "postgres")?;
            let test_url = database_url_for_database(&database_url, &db_name)?;

            let admin_pool = match sqlx::postgres::PgPoolOptions::new()
                .max_connections(2)
                .connect(&admin_url)
                .await
            {
                Ok(pool) => pool,
                Err(error) => {
                    eprintln!(
                        "skipping duplicate registration test: could not connect to admin database: {error}"
                    );
                    return Ok(None);
                }
            };

            if let Err(error) = create_database(&admin_pool, &db_name).await {
                eprintln!(
                    "skipping duplicate registration test: could not create isolated database {db_name}: {error}"
                );
                return Ok(None);
            }

            let pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(6)
                .connect(&test_url)
                .await?;
            crate::db::run_migrations(&pool).await?;

            let config = test_config(&test_url);
            let redis = redis::Client::open(config.redis_url.clone())?;
            let cas_store = crate::services::cas::CasStore::new(&config)?;
            let state = AppState {
                db: pool,
                redis,
                config,
                cas_store,
                rate_limiter: crate::services::rate_limit::RateLimiter::new(),
            };

            Ok(Some(Self {
                state,
                admin_pool,
                db_name,
            }))
        }

        async fn cleanup(self) -> anyhow::Result<()> {
            self.state.db.close().await;
            drop_database(&self.admin_pool, &self.db_name).await?;
            self.admin_pool.close().await;
            Ok(())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_register_same_email_succeeds_once_and_conflicts_once() -> anyhow::Result<()>
    {
        let Some(db) = TestDatabase::new().await? else {
            return Ok(());
        };
        let (org_id, department_id) = insert_registration_scope(&db.state.db).await?;
        let first_req = register_request("duplicate@example.com", org_id, department_id);
        let second_req = register_request("Duplicate@Example.com", org_id, department_id);

        let first = register_user(&db.state, &first_req);
        let second = register_user(&db.state, &second_req);
        let (first_result, second_result) = tokio::join!(first, second);

        let successes = [&first_result, &second_result]
            .iter()
            .filter(|result| result.is_ok())
            .count();
        let conflicts = [first_result, second_result]
            .into_iter()
            .filter(|result| matches!(result, Err(AppError::Conflict(message)) if message == "Email already exists"))
            .count();

        assert_eq!(successes, 1);
        assert_eq!(conflicts, 1);
        assert_eq!(table_count(&db.state.db, "users").await?, 1);

        db.cleanup().await?;
        Ok(())
    }

    fn register_request(email: &str, org_id: Uuid, department_id: Uuid) -> RegisterRequest {
        RegisterRequest {
            email: email.to_string(),
            name: "Duplicate User".to_string(),
            password: "correct-horse-battery".to_string(),
            confirm_password: Some("correct-horse-battery".to_string()),
            org_id: Some(org_id),
            department_id: Some(department_id),
            org_slug: None,
            department_slug: None,
            return_to: None,
        }
    }

    async fn insert_registration_scope(pool: &sqlx::PgPool) -> anyhow::Result<(Uuid, Uuid)> {
        let org_id = Uuid::new_v4();
        let department_id = Uuid::new_v4();

        sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)")
            .bind(org_id)
            .bind("Registration Test Org")
            .bind(format!("registration-test-{}", org_id.simple()))
            .execute(pool)
            .await?;
        sqlx::query(
            "INSERT INTO organization_domains (org_id, domain, verified) VALUES ($1, $2, true)",
        )
        .bind(org_id)
        .bind("example.com")
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO departments (id, org_id, name, slug) VALUES ($1, $2, $3, $4)")
            .bind(department_id)
            .bind(org_id)
            .bind("Engineering")
            .bind(format!("engineering-{}", department_id.simple()))
            .execute(pool)
            .await?;

        Ok((org_id, department_id))
    }

    async fn table_count(pool: &sqlx::PgPool, table: &str) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(pool)
            .await?)
    }

    fn test_config(database_url: &str) -> crate::config::AppConfig {
        crate::config::AppConfig {
            database_url: database_url.to_string(),
            database_max_connections: 20,
            database_min_connections: 1,
            database_acquire_timeout_seconds: 5,
            redis_url: "redis://127.0.0.1:6379".to_string(),
            jwt_secret: "registration-test-secret".to_string(),
            s3_endpoint: "http://localhost:9000".to_string(),
            s3_bucket: "git-ai-cas".to_string(),
            s3_access_key: "minioadmin".to_string(),
            s3_secret_key: "minioadmin".to_string(),
            s3_region: "us-east-1".to_string(),
            base_url: "http://localhost:8080".to_string(),
            sentry_dsn: String::new(),
            posthog_host: String::new(),
            posthog_api_key: String::new(),
        }
    }

    fn test_database_url() -> String {
        dotenvy::dotenv().ok();
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://gitai:gitai@localhost:5433/gitai_enterprise".into())
    }

    fn unique_test_database_name() -> String {
        format!("git_ai_registration_test_{}", Uuid::new_v4().simple())
    }

    fn database_url_for_database(database_url: &str, database: &str) -> anyhow::Result<String> {
        let mut url = url::Url::parse(database_url)?;
        url.set_path(database);
        Ok(url.to_string())
    }

    async fn create_database(pool: &sqlx::PgPool, db_name: &str) -> anyhow::Result<()> {
        sqlx::query(&format!("CREATE DATABASE {}", quote_ident(db_name)))
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn drop_database(pool: &sqlx::PgPool, db_name: &str) -> anyhow::Result<()> {
        sqlx::query(&format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            quote_ident(db_name)
        ))
        .execute(pool)
        .await?;
        Ok(())
    }

    fn quote_ident(identifier: &str) -> String {
        format!("\"{}\"", identifier.replace('"', "\"\""))
    }
}
