use axum::extract::Query;
use axum::response::Html;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct AuthPageQuery {
    pub return_to: Option<String>,
}

pub async fn register_page(Query(query): Query<AuthPageQuery>) -> Html<String> {
    Html(auth_page("注册", "/auth/register", &query.return_to, true))
}

pub async fn login_page(Query(query): Query<AuthPageQuery>) -> Html<String> {
    Html(auth_page("登录", "/auth/login", &query.return_to, false))
}

pub fn success_page(title: &str, message: &str) -> Html<String> {
    Html(format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>git-ai — {title}</title>
  <style>{styles}</style>
</head>
<body>
  <main class="card">
    <h1>{title}</h1>
    <p>{message}</p>
  </main>
</body>
</html>"#,
        title = html_escape(title),
        message = html_escape(message),
        styles = BASE_STYLES,
    ))
}

fn auth_page(title: &str, action: &str, return_to: &Option<String>, is_register: bool) -> String {
    let return_to_field = return_to
        .as_ref()
        .map(|value| {
            format!(
                r#"<input type="hidden" name="return_to" value="{}" />"#,
                html_escape(value)
            )
        })
        .unwrap_or_default();

    let register_fields = if is_register {
        r#"
      <label for="name">姓名</label>
      <input id="name" name="name" type="text" autocomplete="name" required />

      <label for="confirm_password">确认密码</label>
      <input id="confirm_password" name="confirm_password" type="password" autocomplete="new-password" required />

      <label for="org_id">组织 ID</label>
      <input id="org_id" name="org_id" type="text" required />

      <label for="department_id">部门 ID</label>
      <input id="department_id" name="department_id" type="text" required />
"#
    } else {
        ""
    };

    let password_autocomplete = if is_register {
        "new-password"
    } else {
        "current-password"
    };

    let submit = if is_register { "注册" } else { "登录" };

    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>git-ai — {title}</title>
  <style>{styles}</style>
</head>
<body>
  <main class="card">
    <h1>{title}</h1>
    <form method="POST" action="{action}">
      {return_to_field}
      <label for="email">邮箱</label>
      <input id="email" name="email" type="email" autocomplete="email" required autofocus />

      <label for="password">密码</label>
      <input id="password" name="password" type="password" autocomplete="{password_autocomplete}" required />
{register_fields}
      <button type="submit">{submit}</button>
    </form>
  </main>
</body>
</html>"#,
        title = html_escape(title),
        action = action,
        return_to_field = return_to_field,
        password_autocomplete = password_autocomplete,
        register_fields = register_fields,
        submit = submit,
        styles = BASE_STYLES,
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

const BASE_STYLES: &str = r#"
* { box-sizing: border-box; }
body {
  margin: 0;
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #f8fafc;
  color: #0f172a;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}
.card {
  width: min(440px, calc(100vw - 32px));
  border: 1px solid #cbd5e1;
  border-radius: 8px;
  background: #fff;
  padding: 28px;
}
h1 { margin: 0 0 20px; font-size: 24px; }
label { display: block; margin: 14px 0 6px; font-size: 14px; color: #475569; }
input {
  width: 100%;
  min-height: 42px;
  border: 1px solid #cbd5e1;
  border-radius: 6px;
  padding: 8px 10px;
  font-size: 15px;
}
button {
  width: 100%;
  min-height: 42px;
  margin-top: 20px;
  border: 0;
  border-radius: 6px;
  background: #0f172a;
  color: #fff;
  font-size: 15px;
}
p { margin: 0; line-height: 1.5; color: #475569; }
"#;
