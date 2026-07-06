use axum::extract::OriginalUri;
use axum::response::{Html, IntoResponse, Redirect, Response};

use crate::auth::middleware::WebSessionUser;

pub async fn authorize_page(
    WebSessionUser(user_id): WebSessionUser,
    OriginalUri(uri): OriginalUri,
) -> Response {
    if user_id.is_none() {
        return Redirect::to(&login_url(uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/auth/cli/authorize"))).into_response();
    }

    Html(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>git-ai — CLI 授权</title>
</head>
<body>
  <main>
    <h1>git-ai CLI 授权</h1>
  </main>
</body>
</html>"#
            .to_string(),
    )
    .into_response()
}

pub async fn authorize_submit() -> Response {
    (
        axum::http::StatusCode::NOT_IMPLEMENTED,
        "CLI authorization is not implemented yet.",
    )
        .into_response()
}

fn login_url(return_to: &str) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(return_to.as_bytes()).collect();
    format!("/auth/login?return_to={}", encoded)
}
