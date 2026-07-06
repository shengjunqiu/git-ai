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
        let department_options = crate::services::registration::DEFAULT_REGISTER_DEPARTMENTS
            .iter()
            .map(|(slug, name)| {
                format!(
                    r#"<option value="{}">{}</option>"#,
                    html_escape(slug),
                    html_escape(name)
                )
            })
            .collect::<String>();
        format!(
            r#"
      <label for="name">姓名</label>
      <input id="name" name="name" type="text" autocomplete="name" required />

      <label for="confirm_password">确认密码</label>
      <input id="confirm_password" name="confirm_password" type="password" autocomplete="new-password" required />

      <input type="hidden" name="org_slug" value="{org_slug}" />
      <div class="fixed-field">
        <span>组织</span>
        <strong>{org_name}</strong>
      </div>

      <label for="department_slug">部门</label>
      <select id="department_slug" name="department_slug" required>
        {department_options}
      </select>
"#,
            org_slug = html_escape(crate::services::registration::DEFAULT_REGISTER_ORG_SLUG),
            org_name = html_escape(crate::services::registration::DEFAULT_REGISTER_ORG_NAME),
            department_options = department_options,
        )
    } else {
        String::new()
    };

    let password_autocomplete = if is_register {
        "new-password"
    } else {
        "current-password"
    };

    let submit = if is_register { "注册" } else { "登录" };
    let alternate_href = auth_alternate_href(is_register, return_to);
    let alternate_text = if is_register {
        "已有账号？登录"
    } else {
        "没有账号？注册"
    };

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
    <p class="alternate"><a href="{alternate_href}">{alternate_text}</a></p>
  </main>
</body>
</html>"#,
        title = html_escape(title),
        action = action,
        return_to_field = return_to_field,
        password_autocomplete = password_autocomplete,
        register_fields = register_fields,
        submit = submit,
        alternate_href = html_escape(&alternate_href),
        alternate_text = alternate_text,
        styles = BASE_STYLES,
    )
}

fn auth_alternate_href(is_register: bool, return_to: &Option<String>) -> String {
    let path = if is_register {
        "/auth/login"
    } else {
        "/auth/register"
    };
    let Some(return_to) = return_to else {
        return path.to_string();
    };
    let encoded: String = url::form_urlencoded::byte_serialize(return_to.as_bytes()).collect();
    format!("{}?return_to={}", path, encoded)
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
input,
select {
  width: 100%;
  min-height: 42px;
  border: 1px solid #cbd5e1;
  border-radius: 6px;
  background: #fff;
  padding: 8px 10px;
  font-size: 15px;
}
.fixed-field {
  margin-top: 14px;
  min-height: 42px;
  border: 1px solid #cbd5e1;
  border-radius: 6px;
  background: #f8fafc;
  padding: 8px 10px;
}
.fixed-field span { display: block; font-size: 12px; color: #64748b; }
.fixed-field strong { display: block; margin-top: 2px; font-size: 15px; font-weight: 600; }
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
.alternate {
  margin-top: 16px;
  text-align: center;
  font-size: 14px;
}
.alternate a { color: #2563eb; text-decoration: none; }
.alternate a:hover { text-decoration: underline; }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_page_links_to_register_with_return_to() {
        let html = auth_page(
            "登录",
            "/auth/login",
            &Some("/auth/cli/authorize?client_id=git-ai-cli&state=abc".to_string()),
            false,
        );

        assert!(html.contains("没有账号？注册"));
        assert!(html.contains(
            r#"href="/auth/register?return_to=%2Fauth%2Fcli%2Fauthorize%3Fclient_id%3Dgit-ai-cli%26state%3Dabc""#
        ));
    }

    #[test]
    fn register_page_links_back_to_login_with_return_to() {
        let html = auth_page(
            "注册",
            "/auth/register",
            &Some("/auth/cli/authorize?client_id=git-ai-cli&state=abc".to_string()),
            true,
        );

        assert!(html.contains("已有账号？登录"));
        assert!(html.contains(
            r#"href="/auth/login?return_to=%2Fauth%2Fcli%2Fauthorize%3Fclient_id%3Dgit-ai-cli%26state%3Dabc""#
        ));
    }

    #[test]
    fn register_page_uses_default_linewell_department_options() {
        let html = auth_page("注册", "/auth/register", &None, true);

        assert!(html.contains(r#"name="org_slug" value="linewell.com""#));
        assert!(html.contains("<strong>Linewell</strong>"));
        assert!(html.contains(r#"<select id="department_slug" name="department_slug" required>"#));
        assert!(html.contains(r#"<option value="technology-center">技术中心</option>"#));
        assert!(html.contains(r#"<option value="rd-center">研发中心</option>"#));
        assert!(!html.contains("org_id"));
        assert!(!html.contains("department_id"));
    }
}
