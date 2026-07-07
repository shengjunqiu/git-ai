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
  <main class="auth-shell">
    <section class="auth-card auth-card-compact">
      <div class="brand-lockup">
        <div class="brand-title"><span>git-ai</span> Enterprise</div>
        <div class="brand-subtitle">AI 代码归属分析平台</div>
      </div>
      <div class="page-kicker">账户状态</div>
      <h1>{title}</h1>
      <p class="page-copy">{message}</p>
    </section>
  </main>
</body>
</html>"#,
        title = html_escape(title),
        message = html_escape(message),
        styles = AUTH_PAGE_STYLES,
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

      <label for="org_id">组织</label>
      <select id="org_id" name="org_id" required disabled>
        <option value="">输入邮箱后选择组织</option>
      </select>

      <label for="department_id">部门</label>
      <select id="department_id" name="department_id" required disabled>
        <option value="">先选择组织</option>
      </select>
      <div class="form-hint" id="registration-scope-hint">输入邮箱后加载可加入的组织和部门</div>
"#
        .to_string()
    } else {
        String::new()
    };
    let page_script = if is_register {
        REGISTER_PAGE_SCRIPT
    } else {
        ""
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
  <main class="auth-shell">
    <section class="auth-card">
      <div class="brand-lockup">
        <div class="brand-title"><span>git-ai</span> Enterprise</div>
        <div class="brand-subtitle">AI 代码归属分析平台</div>
      </div>
      <div class="page-kicker">账号访问</div>
      <h1>{title}</h1>
      <form method="POST" action="{action}">
        {return_to_field}
        <label for="email">邮箱</label>
        <input id="email" name="email" type="email" autocomplete="email" required autofocus />

        <label for="password">密码</label>
        <input id="password" name="password" type="password" autocomplete="{password_autocomplete}" required />
{register_fields}
        <button class="btn btn-primary" type="submit">{submit}</button>
      </form>
      <p class="alternate"><a href="{alternate_href}">{alternate_text}</a></p>
    </section>
  </main>
{page_script}
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
        styles = AUTH_PAGE_STYLES,
        page_script = page_script,
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

const REGISTER_PAGE_SCRIPT: &str = r#"<script>
(() => {
  const emailInput = document.getElementById('email');
  const orgSelect = document.getElementById('org_id');
  const departmentSelect = document.getElementById('department_id');
  const hint = document.getElementById('registration-scope-hint');
  const submit = document.querySelector('button[type="submit"]');
  let emailTimer = null;

  function resetSelect(select, text) {
    select.innerHTML = '';
    const option = document.createElement('option');
    option.value = '';
    option.textContent = text;
    select.appendChild(option);
    select.disabled = true;
  }

  function setHint(text, type = '') {
    hint.textContent = text;
    hint.className = type ? `form-hint ${type}` : 'form-hint';
  }

  function updateSubmitState() {
    submit.disabled = !(orgSelect.value && departmentSelect.value);
  }

  function appendOption(select, value, label) {
    const option = document.createElement('option');
    option.value = value;
    option.textContent = label;
    select.appendChild(option);
  }

  async function loadDepartments(orgId) {
    resetSelect(departmentSelect, '加载部门中...');
    updateSubmitState();

    try {
      const response = await fetch(`/auth/organizations/${orgId}/departments`, {
        headers: { 'Accept': 'application/json' }
      });
      const data = await response.json();
      if (!response.ok) {
        throw new Error(data.error || '部门加载失败');
      }

      const departments = data.departments || [];
      resetSelect(departmentSelect, departments.length ? '请选择部门' : '暂无可选部门');
      if (departments.length === 0) {
        setHint('该组织还没有可选部门，请联系管理员添加部门。', 'error');
        updateSubmitState();
        return;
      }

      departments.forEach(department => {
        appendOption(departmentSelect, department.id, department.name);
      });
      departmentSelect.disabled = false;
      if (departments.length === 1) {
        departmentSelect.value = departments[0].id;
      }
      setHint('请选择你所在的部门。');
    } catch (error) {
      resetSelect(departmentSelect, '部门加载失败');
      setHint(error.message || '部门加载失败', 'error');
    }

    updateSubmitState();
  }

  async function loadOrganizations() {
    const email = emailInput.value.trim();
    resetSelect(orgSelect, '加载组织中...');
    resetSelect(departmentSelect, '先选择组织');
    updateSubmitState();

    if (!email.includes('@')) {
      resetSelect(orgSelect, '输入邮箱后选择组织');
      setHint('输入邮箱后加载可加入的组织和部门');
      return;
    }

    try {
      const response = await fetch(`/auth/organizations?email=${encodeURIComponent(email)}`, {
        headers: { 'Accept': 'application/json' }
      });
      const data = await response.json();
      if (!response.ok) {
        throw new Error(data.error || '组织加载失败');
      }

      const organizations = data.organizations || [];
      resetSelect(orgSelect, organizations.length ? '请选择组织' : '暂无可加入组织');
      if (organizations.length === 0) {
        setHint('当前邮箱域名没有可加入组织，请联系管理员。', 'error');
        return;
      }

      organizations.forEach(org => {
        appendOption(orgSelect, org.id, `${org.name} (${org.slug})`);
      });
      orgSelect.disabled = false;
      if (organizations.length === 1) {
        orgSelect.value = organizations[0].id;
        await loadDepartments(orgSelect.value);
      } else {
        setHint('请选择要加入的组织。');
      }
    } catch (error) {
      resetSelect(orgSelect, '组织加载失败');
      setHint(error.message || '组织加载失败', 'error');
    }

    updateSubmitState();
  }

  emailInput.addEventListener('input', () => {
    clearTimeout(emailTimer);
    emailTimer = setTimeout(loadOrganizations, 300);
  });
  orgSelect.addEventListener('change', () => {
    if (orgSelect.value) {
      loadDepartments(orgSelect.value);
    } else {
      resetSelect(departmentSelect, '先选择组织');
      updateSubmitState();
    }
  });
  departmentSelect.addEventListener('change', updateSubmitState);

  resetSelect(orgSelect, '输入邮箱后选择组织');
  resetSelect(departmentSelect, '先选择组织');
  updateSubmitState();
})();
</script>"#;

pub(crate) const AUTH_PAGE_STYLES: &str = r#"
:root {
  font-size: 112.5%;
  --bg-primary: #0f172a;
  --bg-card: #1e293b;
  --bg-card-hover: #263548;
  --border: #334155;
  --text-primary: #f1f5f9;
  --text-secondary: #94a3b8;
  --text-muted: #64748b;
  --accent: #818cf8;
  --accent-light: #6366f1;
  --success: #34d399;
  --warning: #fbbf24;
  --danger: #f87171;
}
* { margin: 0; padding: 0; box-sizing: border-box; }
body {
  margin: 0;
  min-height: 100vh;
  background: var(--bg-primary);
  color: var(--text-primary);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "PingFang SC", "Microsoft YaHei", sans-serif;
}
.auth-shell {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 2rem 1rem;
}
.auth-card {
  width: min(460px, calc(100vw - 32px));
  border: 1px solid var(--border);
  border-radius: 12px;
  background: var(--bg-card);
  padding: 2rem;
}
.auth-card-compact {
  width: min(420px, calc(100vw - 32px));
}
.brand-lockup {
  padding-bottom: 1.25rem;
  margin-bottom: 1.5rem;
  border-bottom: 1px solid var(--border);
}
.brand-title {
  font-size: 1.25rem;
  font-weight: 800;
}
.brand-title span {
  color: var(--accent);
}
.brand-subtitle {
  color: var(--text-muted);
  font-size: 0.75rem;
  margin-top: 0.25rem;
}
.page-kicker {
  color: var(--text-muted);
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.1em;
  margin-bottom: 0.5rem;
}
h1 {
  margin: 0 0 1.5rem;
  font-size: 1.5rem;
  line-height: 1.25;
  font-weight: 700;
}
label {
  display: block;
  margin: 1rem 0 0.5rem;
  font-size: 0.8rem;
  color: var(--text-secondary);
  font-weight: 500;
}
input,
select {
  width: 100%;
  min-height: 44px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg-primary);
  color: var(--text-primary);
  padding: 0.625rem 0.875rem;
  font-size: 0.875rem;
}
input:focus,
select:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 3px rgba(99,102,241,0.2);
}
input::placeholder {
  color: var(--text-muted);
}
select:disabled,
button:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}
.form-hint {
  margin-top: 0.75rem;
  color: var(--text-muted);
  font-size: 0.75rem;
  line-height: 1.45;
}
.form-hint.error {
  color: var(--danger);
}
.fixed-field {
  margin-top: 1rem;
  min-height: 44px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg-primary);
  padding: 0.625rem 0.875rem;
}
.fixed-field span {
  display: block;
  font-size: 0.7rem;
  color: var(--text-muted);
  margin-bottom: 0.25rem;
}
.fixed-field strong {
  display: block;
  color: var(--text-primary);
  font-size: 0.875rem;
  font-weight: 600;
}
.btn {
  width: 100%;
  min-height: 44px;
  margin-top: 1.25rem;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--bg-card);
  color: var(--text-primary);
  font-size: 0.875rem;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;
}
.btn:hover {
  background: var(--bg-card-hover);
  border-color: var(--accent);
}
.btn-primary {
  background: linear-gradient(135deg, var(--accent-light), var(--accent));
  border: none;
  color: #fff;
}
.btn-primary:hover {
  opacity: 0.9;
}
.btn-secondary {
  color: var(--text-secondary);
}
.btn-secondary:hover {
  color: var(--text-primary);
}
p {
  margin: 0;
  line-height: 1.5;
  color: var(--text-secondary);
}
.page-copy {
  font-size: 0.875rem;
}
.alternate {
  margin-top: 1rem;
  text-align: center;
  font-size: 0.8rem;
  color: var(--text-secondary);
}
.alternate a {
  color: var(--accent);
  text-decoration: none;
}
.alternate a:hover {
  text-decoration: underline;
}
.identity-list {
  border: 1px solid var(--border);
  border-radius: 8px;
  overflow: hidden;
  margin-bottom: 1.25rem;
}
.identity-row {
  display: flex;
  justify-content: space-between;
  gap: 1rem;
  padding: 0.75rem 0.875rem;
  border-bottom: 1px solid var(--border);
  background: var(--bg-primary);
}
.identity-row:last-child {
  border-bottom: none;
}
.identity-label {
  color: var(--text-muted);
  font-size: 0.75rem;
  white-space: nowrap;
}
.identity-value {
  color: var(--text-primary);
  font-size: 0.875rem;
  text-align: right;
  overflow-wrap: anywhere;
}
.auth-actions {
  display: flex;
  gap: 0.75rem;
}
.auth-actions .btn {
  margin-top: 0;
}
@media (max-width: 520px) {
  .auth-card {
    padding: 1.5rem;
  }
  .auth-actions {
    flex-direction: column;
  }
  .identity-row {
    flex-direction: column;
    gap: 0.25rem;
  }
  .identity-value {
    text-align: left;
  }
}
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
    fn register_page_uses_dynamic_department_options() {
        let html = auth_page("注册", "/auth/register", &None, true);

        assert!(html.contains(r#"<select id="org_id" name="org_id" required disabled>"#));
        assert!(html.contains(
            r#"<select id="department_id" name="department_id" required disabled>"#
        ));
        assert!(html.contains("/auth/organizations?email="));
        assert!(html.contains("/departments"));
        assert!(!html.contains(r#"name="org_slug" value="linewell.com""#));
        assert!(!html.contains(r#"name="department_slug""#));
    }
}
