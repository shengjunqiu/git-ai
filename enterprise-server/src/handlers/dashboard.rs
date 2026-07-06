use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse, Json, Redirect};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::middleware::{DashboardAuth, OptionalAuth};
use crate::error::AppError;
use crate::routes::AppState;

/// GET /me — Dashboard home page
pub async fn dashboard_me(
    State(_state): State<AppState>,
    auth: OptionalAuth,
) -> impl IntoResponse {
    // If not authenticated, redirect to login page
    let auth = match auth.0 {
        Some(a) => a,
        None => return Redirect::to("/auth/login?return_to=/me").into_response(),
    };
    let is_admin = auth.is_admin();
    let user_id_str = auth.user_id.to_string();
    Html(format!(
        r##"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>git-ai 企业仪表盘</title>
    <script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.7/dist/chart.umd.min.js"></script>
    <style>
        :root {{
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
        }}
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'PingFang SC', 'Microsoft YaHei', sans-serif;
                background: var(--bg-primary); color: var(--text-primary); }}
        .layout {{ display: flex; min-height: 100vh; }}

        /* Sidebar */
        .sidebar {{ width: 260px; background: var(--bg-card); border-right: 1px solid var(--border);
                    padding: 1.5rem; flex-shrink: 0; display: flex; flex-direction: column; }}
        .sidebar-logo {{ font-size: 1.25rem; font-weight: 800; margin-bottom: 0.25rem; }}
        .sidebar-logo span {{ color: var(--accent); }}
        .sidebar-subtitle {{ color: var(--text-muted); font-size: 0.75rem; margin-bottom: 2rem; }}
        .nav-item {{ display: flex; align-items: center; gap: 0.75rem; padding: 0.625rem 0.75rem;
                     border-radius: 8px; color: var(--text-secondary); text-decoration: none;
                     font-size: 0.875rem; margin-bottom: 0.25rem; transition: all 0.15s; cursor: pointer; }}
        .nav-item:hover {{ background: var(--bg-card-hover); color: var(--text-primary); }}
        .nav-item.active {{ background: rgba(129,140,248,0.15); color: var(--accent); }}
        .nav-icon {{ width: 20px; text-align: center; font-size: 1rem; }}
        .nav-section {{ color: var(--text-muted); font-size: 0.7rem; text-transform: uppercase;
                        letter-spacing: 0.1em; margin: 1.5rem 0 0.5rem 0.75rem; }}
        .sidebar-footer {{ margin-top: auto; padding-top: 1rem; border-top: 1px solid var(--border); }}
        .sidebar-user {{ padding: 0.5rem 0.75rem; color: var(--text-secondary); font-size: 0.8rem; }}
        .sidebar-user-name {{ color: var(--text-primary); font-weight: 500; }}
        .sidebar-user-email {{ color: var(--text-muted); font-size: 0.75rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
        .logout-btn {{ display: flex; align-items: center; gap: 0.5rem; padding: 0.5rem 0.75rem; color: var(--text-muted);
                       font-size: 0.8rem; cursor: pointer; border: none; background: none; width: 100%;
                       border-radius: 6px; transition: all 0.15s; margin-top: 0.5rem; }}
        .logout-btn:hover {{ background: var(--bg-card-hover); color: var(--danger); }}

        /* Main content */
        .main {{ flex: 1; padding: 2rem; overflow-y: auto; }}
        .page-header {{ display: flex; justify-content: space-between; align-items: center; margin-bottom: 2rem; flex-wrap: wrap; gap: 1rem; }}
        .page-title {{ font-size: 1.75rem; font-weight: 700; }}
        .page-subtitle {{ color: var(--text-secondary); font-size: 0.875rem; margin-top: 0.25rem; }}

        /* Toolbar */
        .toolbar {{ display: flex; align-items: center; gap: 1rem; flex-wrap: wrap; }}
        .refresh-info {{ color: var(--text-muted); font-size: 0.75rem; display: flex; align-items: center; gap: 0.5rem; }}
        .refresh-dot {{ width: 8px; height: 8px; border-radius: 50%; background: var(--success); animation: pulse 2s infinite; }}
        @keyframes pulse {{ 0%, 100% {{ opacity: 1; }} 50% {{ opacity: 0.4; }} }}
        .btn {{ padding: 0.5rem 1rem; border-radius: 8px; border: 1px solid var(--border);
                background: var(--bg-card); color: var(--text-primary); font-size: 0.8rem;
                cursor: pointer; transition: all 0.15s; display: inline-flex; align-items: center; gap: 0.5rem; }}
        .btn:hover {{ background: var(--bg-card-hover); border-color: var(--accent); }}
        .btn-primary {{ background: linear-gradient(135deg, #6366f1, #818cf8); border: none; color: white; }}
        .btn-primary:hover {{ opacity: 0.9; }}
        .btn-danger {{ border-color: var(--danger); color: var(--danger); }}
        .btn-danger:hover {{ background: rgba(248,113,113,0.1); }}
        .btn-sm {{ padding: 0.35rem 0.75rem; font-size: 0.75rem; }}
        select {{ padding: 0.5rem 0.75rem; border-radius: 8px; border: 1px solid var(--border);
                  background: var(--bg-card); color: var(--text-primary); font-size: 0.8rem; cursor: pointer; }}
        select:focus {{ outline: none; border-color: var(--accent); }}

        /* Stats cards */
        .stats-grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
                       gap: 1rem; margin-bottom: 2rem; }}
        .stat-card {{ background: var(--bg-card); border: 1px solid var(--border); border-radius: 12px;
                      padding: 1.25rem; transition: border-color 0.15s; }}
        .stat-card:hover {{ border-color: var(--accent); }}
        .stat-label {{ color: var(--text-muted); font-size: 0.75rem; letter-spacing: 0.05em; margin-bottom: 0.5rem; }}
        .stat-value {{ font-size: 1.75rem; font-weight: 700; }}
        .stat-value.ai {{ color: var(--accent); }}
        .stat-value.human {{ color: var(--success); }}
        .stat-value.total {{ color: var(--text-primary); }}
        .stat-value.pct {{ color: var(--warning); }}

        /* Data tables */
        .table-card {{ background: var(--bg-card); border: 1px solid var(--border); border-radius: 12px;
                       overflow: hidden; margin-bottom: 1.5rem; }}
        .table-header {{ display: flex; justify-content: space-between; align-items: center;
                         padding: 1rem 1.25rem; border-bottom: 1px solid var(--border); }}
        .table-title {{ font-size: 1rem; font-weight: 600; }}
        table {{ width: 100%; border-collapse: collapse; }}
        th {{ text-align: left; padding: 0.75rem 1.25rem; color: var(--text-muted); font-size: 0.75rem;
              text-transform: uppercase; letter-spacing: 0.05em; border-bottom: 1px solid var(--border);
              white-space: nowrap; }}
        td {{ padding: 0.75rem 1.25rem; font-size: 0.875rem; border-bottom: 1px solid var(--border); }}
        tr:last-child td {{ border-bottom: none; }}
        tr:hover {{ background: var(--bg-card-hover); }}
        .bar {{ height: 6px; border-radius: 3px; background: var(--border); min-width: 80px; }}
        .bar-fill {{ height: 100%; border-radius: 3px; background: linear-gradient(90deg, var(--accent-light), var(--accent)); }}
        .badge {{ display: inline-block; padding: 0.125rem 0.5rem; border-radius: 4px; font-size: 0.7rem;
                  font-weight: 600; }}
        .badge.ai {{ background: rgba(129,140,248,0.2); color: var(--accent); }}
        .badge.human {{ background: rgba(52,211,153,0.2); color: var(--success); }}
        .badge.active {{ background: rgba(52,211,153,0.2); color: var(--success); }}
        .badge.revoked {{ background: rgba(248,113,113,0.2); color: var(--danger); }}
        .badge.role {{ background: rgba(129,140,248,0.2); color: var(--accent); }}

        /* Sections */
        .section {{ display: none; }}
        .section.active {{ display: block; }}

        /* Chart */
        .chart-card {{ background: var(--bg-card); border: 1px solid var(--border); border-radius: 12px;
                       padding: 1.5rem; margin-bottom: 1.5rem; }}
        .chart-title {{ font-size: 1rem; font-weight: 600; margin-bottom: 1rem; }}
        .chart-container {{ position: relative; height: 300px; }}
        .chart-bar {{ display: flex; align-items: center; gap: 0.75rem; margin-bottom: 0.75rem; }}
        .chart-label {{ width: 140px; text-align: right; font-size: 0.8rem; color: var(--text-secondary); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }}
        .chart-track {{ flex: 1; height: 24px; background: var(--bg-primary); border-radius: 6px; overflow: hidden; position: relative; }}
        .chart-fill {{ height: 100%; border-radius: 6px; display: flex; transition: width 0.5s ease; }}
        .chart-fill .ai-part {{ background: var(--accent); flex-shrink: 0; }}
        .chart-fill .human-part {{ background: var(--success); flex-shrink: 0; }}
        .chart-value {{ font-size: 0.75rem; color: var(--text-muted); min-width: 50px; }}

        /* Modal */
        .modal-overlay {{ position: fixed; top: 0; left: 0; width: 100%; height: 100%;
                          background: rgba(0,0,0,0.6); display: flex; align-items: center;
                          justify-content: center; z-index: 1000; }}
        .modal {{ background: var(--bg-card); border: 1px solid var(--border); border-radius: 16px;
                  padding: 2rem; max-width: 500px; width: 90%; max-height: 90vh; overflow-y: auto; }}
        .modal-title {{ font-size: 1.25rem; font-weight: 700; margin-bottom: 1.5rem; }}
        .form-group {{ margin-bottom: 1rem; }}
        .form-label {{ display: block; color: var(--text-secondary); font-size: 0.8rem;
                       margin-bottom: 0.5rem; font-weight: 500; }}
        .form-input {{ width: 100%; padding: 0.625rem 0.875rem; border-radius: 8px;
                       border: 1px solid var(--border); background: var(--bg-primary);
                       color: var(--text-primary); font-size: 0.875rem; }}
        .form-input:focus {{ outline: none; border-color: var(--accent); box-shadow: 0 0 0 3px rgba(99,102,241,0.2); }}
        .form-input::placeholder {{ color: var(--text-muted); }}
        .form-actions {{ display: flex; gap: 0.75rem; justify-content: flex-end; margin-top: 1.5rem; }}
        .api-key-display {{ background: var(--bg-primary); border: 1px solid var(--border); border-radius: 8px;
                            padding: 0.75rem 1rem; font-family: monospace; font-size: 0.8rem;
                            color: var(--warning); word-break: break-all; margin-top: 0.5rem;
                            position: relative; }}
        .copy-btn {{ position: absolute; top: 0.5rem; right: 0.5rem; padding: 0.25rem 0.5rem;
                     font-size: 0.7rem; border-radius: 4px; border: 1px solid var(--border);
                     background: var(--bg-card); color: var(--text-secondary); cursor: pointer; }}
        .copy-btn:hover {{ border-color: var(--accent); color: var(--accent); }}
        .empty-state {{ text-align: center; padding: 3rem; color: var(--text-muted); }}
        .empty-icon {{ font-size: 2.5rem; margin-bottom: 1rem; }}

        /* Toast */
        .toast {{ position: fixed; top: 1rem; right: 1rem; padding: 0.75rem 1.25rem;
                   border-radius: 8px; font-size: 0.85rem; z-index: 2000;
                   animation: slideIn 0.3s ease, fadeOut 0.3s ease 2.7s; }}
        .toast.success {{ background: #065f46; color: #6ee7b7; border: 1px solid #059669; }}
        .toast.error {{ background: #7f1d1d; color: #fca5a5; border: 1px solid #dc2626; }}
        .toast.info {{ background: #1e3a5f; color: #93c5fd; border: 1px solid #3b82f6; }}
        @keyframes slideIn {{ from {{ transform: translateX(100%); opacity: 0; }} to {{ transform: translateX(0); opacity: 1; }} }}
        @keyframes fadeOut {{ from {{ opacity: 1; }} to {{ opacity: 0; }} }}

        /* Responsive */
        @media (max-width: 768px) {{
            .sidebar {{ display: none; }}
            .stats-grid {{ grid-template-columns: 1fr 1fr; }}
            .page-header {{ flex-direction: column; align-items: flex-start; }}
        }}
    </style>
    <script>
        const isAdmin = {is_admin};
        const currentUserId = '{user_id_str}';
    </script>
</head>
<body>
    <div class="layout">
        <aside class="sidebar">
            <div class="sidebar-logo"><span>git-ai</span> Enterprise</div>
            <div class="sidebar-subtitle">AI 代码归属分析平台</div>

            <div class="nav-section">数据概览</div>
            <a class="nav-item active" onclick="showSection('overview')">
                <span class="nav-icon">📊</span> 总览
            </a>
            <a class="nav-item" onclick="showSection('trends')">
                <span class="nav-icon">📈</span> 趋势分析
            </a>
            <a class="nav-item" onclick="showSection('organizations')">
                <span class="nav-icon">🏢</span> 组织
            </a>
            <a class="nav-item" onclick="showSection('developers')">
                <span class="nav-icon">👥</span> 开发者
            </a>
            <a class="nav-item" onclick="showSection('projects')">
                <span class="nav-icon">📁</span> 项目
            </a>
            <a class="nav-item" onclick="showSection('tools')">
                <span class="nav-icon">🤖</span> AI 工具
            </a>

            <div class="nav-section" id="admin-nav-section">系统管理</div>
            <a class="nav-item" id="admin-nav-users" onclick="showSection('users')">
                <span class="nav-icon">👤</span> 用户管理
            </a>
            <a class="nav-item" id="admin-nav-apikeys" onclick="showSection('apikeys')">
                <span class="nav-icon">🔑</span> API 密钥
            </a>

            <div class="sidebar-footer">
                <div class="sidebar-user">
                    <div class="sidebar-user-name">{name}</div>
                    <div class="sidebar-user-email" title="{email}">{email}</div>
                </div>
                <button class="logout-btn" onclick="window.location.href='/logout'">
                    <span>🚪</span> 退出登录
                </button>
            </div>
        </aside>

        <main class="main">
            <!-- Overview Section -->
            <div id="section-overview" class="section active">
                <div class="page-header">
                    <div>
                        <div class="page-title">数据总览</div>
                        <div class="page-subtitle">AI 代码归属分析 - 组织级概览</div>
                    </div>
                    <div class="toolbar">
                        <select id="time-range" onchange="refreshCurrentSection()">
                            <option value="7d">最近 7 天</option>
                            <option value="30d" selected>最近 30 天</option>
                            <option value="90d">最近 90 天</option>
                            <option value="all">全部时间</option>
                        </select>
                        <div class="refresh-info">
                            <span class="refresh-dot"></span>
                            <span id="last-refresh">刷新中...</span>
                        </div>
                        <button class="btn" onclick="refreshCurrentSection()">🔄 刷新</button>
                    </div>
                </div>
                <div class="stats-grid" id="overview-stats">
                    <div class="stat-card"><div class="stat-label">总提交数</div><div class="stat-value total" id="s-commits">—</div></div>
                    <div class="stat-card"><div class="stat-label">AI 生成代码行</div><div class="stat-value ai" id="s-ai-lines">—</div></div>
                    <div class="stat-card"><div class="stat-label">人工编写代码行</div><div class="stat-value human" id="s-human-lines">—</div></div>
                    <div class="stat-card"><div class="stat-label">AI 代码占比</div><div class="stat-value pct" id="s-ai-pct">—</div></div>
                    <div class="stat-card"><div class="stat-label">开发者数量</div><div class="stat-value total" id="s-devs">—</div></div>
                    <div class="stat-card"><div class="stat-label">项目数量</div><div class="stat-value total" id="s-projects">—</div></div>
                </div>

                <div class="chart-card">
                    <div class="chart-title">AI 代码趋势（近 30 天）</div>
                    <div class="chart-container"><canvas id="overview-trend-chart"></canvas></div>
                </div>

                <div class="table-card">
                    <div class="table-header">
                        <div class="table-title">AI 使用量 Top 开发者</div>
                    </div>
                    <div id="top-developers" style="padding: 1rem;">
                        <p style="color: var(--text-muted); font-size: 0.875rem;">加载中...</p>
                    </div>
                </div>
            </div>

            <!-- Trends Section -->
            <div id="section-trends" class="section">
                <div class="page-header">
                    <div>
                        <div class="page-title">趋势分析</div>
                        <div class="page-subtitle">AI 代码归属随时间的变化趋势</div>
                    </div>
                    <div class="toolbar">
                        <select id="trend-metric" onchange="loadTrends()">
                            <option value="ai_ratio">AI 占比</option>
                            <option value="ai_lines">AI 代码行数</option>
                            <option value="human_lines">人工代码行数</option>
                            <option value="commits">提交数</option>
                        </select>
                        <select id="trend-granularity" onchange="loadTrends()">
                            <option value="day">按天</option>
                            <option value="week" selected>按周</option>
                            <option value="month">按月</option>
                        </select>
                    </div>
                </div>
                <div class="chart-card">
                    <div class="chart-title" id="trend-chart-title">AI 代码占比趋势（按周）</div>
                    <div class="chart-container" style="height: 400px;"><canvas id="trend-chart"></canvas></div>
                </div>
                <div class="chart-card">
                    <div class="chart-title">AI 工具对比</div>
                    <div class="chart-container" style="height: 350px;"><canvas id="agent-comparison-chart"></canvas></div>
                </div>
            </div>

            <!-- Organizations Section -->
            <div id="section-organizations" class="section">
                <div class="page-header">
                    <div>
                        <div class="page-title">组织</div>
                        <div class="page-subtitle">按组织查看 AI 代码归属分析</div>
                    </div>
                </div>
                <div class="table-card">
                    <table>
                        <thead><tr><th>组织名称</th><th>提交数</th><th>AI 代码行</th><th>人工代码行</th><th>AI 占比</th></tr></thead>
                        <tbody id="org-table"><tr><td colspan="5">加载中...</td></tr></tbody>
                    </table>
                </div>
            </div>

            <!-- Developers Section -->
            <div id="section-developers" class="section">
                <div class="page-header">
                    <div>
                        <div class="page-title">开发者</div>
                        <div class="page-subtitle">开发者个人 AI 使用统计</div>
                    </div>
                </div>
                <div class="table-card">
                    <table>
                        <thead><tr><th>姓名/邮箱</th><th>提交数</th><th>总代码行</th><th>AI 代码行</th><th>人工代码行</th><th>AI 占比</th></tr></thead>
                        <tbody id="dev-table"><tr><td colspan="6">加载中...</td></tr></tbody>
                    </table>
                </div>
            </div>

            <!-- Projects Section -->
            <div id="section-projects" class="section">
                <div class="page-header">
                    <div>
                        <div class="page-title">项目</div>
                        <div class="page-subtitle">项目级 AI 代码归属分析</div>
                    </div>
                </div>
                <div class="table-card">
                    <table>
                        <thead><tr><th>项目名称</th><th>分支</th><th>提交数</th><th>AI 代码行</th><th>人工代码行</th><th>AI 占比</th></tr></thead>
                        <tbody id="proj-table"><tr><td colspan="6">加载中...</td></tr></tbody>
                    </table>
                </div>
            </div>

            <!-- Tools Section -->
            <div id="section-tools" class="section">
                <div class="page-header">
                    <div>
                        <div class="page-title">AI 工具</div>
                        <div class="page-subtitle">各 AI 工具和模型的使用情况</div>
                    </div>
                </div>
                <div class="table-card">
                    <table>
                        <thead><tr><th>工具 / 模型</th><th>AI 代码行</th><th>混合代码行</th><th>AI 采纳数</th><th>AI 总代码</th></tr></thead>
                        <tbody id="tools-table"><tr><td colspan="5">加载中...</td></tr></tbody>
                    </table>
                </div>
            </div>

            <!-- Users Management Section -->
            <div id="section-users" class="section admin-only">
                <div class="page-header">
                    <div>
                        <div class="page-title">用户管理</div>
                        <div class="page-subtitle">管理系统用户及其关联的 API 密钥</div>
                    </div>
                    <button class="btn btn-primary" onclick="showCreateUserModal()">+ 创建用户</button>
                </div>
                <div class="table-card">
                    <table>
                        <thead><tr><th>用户名</th><th>邮箱</th><th>API 密钥</th><th>创建时间</th><th>操作</th></tr></thead>
                        <tbody id="users-table"><tr><td colspan="5">加载中...</td></tr></tbody>
                    </table>
                </div>
            </div>

            <!-- API Keys Management Section -->
            <div id="section-apikeys" class="section admin-only">
                <div class="page-header">
                    <div>
                        <div class="page-title">API 密钥管理</div>
                        <div class="page-subtitle">创建和管理 API 访问密钥</div>
                    </div>
                    <button class="btn btn-primary" onclick="showCreateApiKeyModal()">+ 创建密钥</button>
                </div>
                <div class="table-card">
                    <table>
                        <thead><tr><th>名称</th><th>前缀</th><th>权限范围</th><th>创建时间</th><th>过期时间</th><th>最后使用</th><th>操作</th></tr></thead>
                        <tbody id="apikeys-table"><tr><td colspan="7">加载中...</td></tr></tbody>
                    </table>
                </div>
            </div>
        </main>
    </div>

    <!-- Modal Container -->
    <div id="modal-container"></div>

    <script>
        const name = "{name}";
        const email = "{email}";
        const fmt = n => typeof n === 'number' ? n.toLocaleString() : '0';
        const pctBar = (pct) => `<div class="bar"><div class="bar-fill" style="width:${{Math.min(pct,100)}}%"></div></div>`;

        // --- Auto refresh ---
        let refreshInterval = null;
        const AUTO_REFRESH_MS = 60000; // 60 seconds
        let currentSection = 'overview';

        // Role-based UI: hide admin sections for non-admin users
        if (!isAdmin) {{
            document.querySelectorAll('.admin-only').forEach(el => el.style.display = 'none');
            document.getElementById('admin-nav-section').style.display = 'none';
            document.getElementById('admin-nav-users').style.display = 'none';
            document.getElementById('admin-nav-apikeys').style.display = 'none';
        }}

        function startAutoRefresh() {{
            stopAutoRefresh();
            refreshInterval = setInterval(() => refreshCurrentSection(), AUTO_REFRESH_MS);
        }}
        function stopAutoRefresh() {{
            if (refreshInterval) {{ clearInterval(refreshInterval); refreshInterval = null; }}
        }}
        function updateRefreshTime() {{
            const now = new Date();
            document.getElementById('last-refresh').textContent =
                `上次刷新: ${{now.getHours().toString().padStart(2,'0')}}:${{now.getMinutes().toString().padStart(2,'0')}}:${{now.getSeconds().toString().padStart(2,'0')}}`;
        }}

        function refreshCurrentSection() {{
            loadSection(currentSection);
            updateRefreshTime();
        }}

        // --- Navigation ---
        function showSection(id) {{
            // Non-admin users cannot access admin sections
            if (!isAdmin && (id === 'users' || id === 'apikeys')) {{
                return;
            }}
            currentSection = id;
            document.querySelectorAll('.section').forEach(s => s.classList.remove('active'));
            document.querySelectorAll('.nav-item').forEach(n => n.classList.remove('active'));
            document.getElementById('section-' + id).classList.add('active');
            event.currentTarget.classList.add('active');
            loadSection(id);
        }}

        function loadSection(id) {{
            switch(id) {{
                case 'overview': loadOverview(); break;
                case 'trends': loadTrends(); break;
                case 'organizations': loadOrgs(); break;
                case 'developers': loadDevs(); break;
                case 'projects': loadProjects(); break;
                case 'tools': loadTools(); break;
                case 'users': loadUsers(); break;
                case 'apikeys': loadApiKeys(); break;
            }}
        }}

        // --- Toast notifications ---
        function showToast(message, type = 'info') {{
            const existing = document.querySelector('.toast');
            if (existing) existing.remove();
            const toast = document.createElement('div');
            toast.className = `toast ${{type}}`;
            toast.textContent = message;
            document.body.appendChild(toast);
            setTimeout(() => toast.remove(), 3000);
        }}

        // --- Time range helper ---
        function getTimeRangeParams() {{
            const range = document.getElementById('time-range')?.value || '30d';
            if (range === 'all') return '';
            const days = parseInt(range);
            const since = new Date(Date.now() - days * 86400000).toISOString();
            return `&since=${{encodeURIComponent(since)}}`;
        }}

        // --- Chart instances ---
        let overviewTrendChart = null;
        let trendChart = null;
        let agentComparisonChart = null;

        // --- Overview ---
        async function loadOverview() {{
            try {{
                const r = await fetch('/api/v1/aggregate/summary');
                const d = await r.json();
                document.getElementById('s-commits').textContent = fmt(d.total_commits);
                document.getElementById('s-ai-lines').textContent = fmt(d.total_ai_lines);
                document.getElementById('s-human-lines').textContent = fmt(d.total_human_lines);
                document.getElementById('s-ai-pct').textContent = (d.pct_ai_lines || 0).toFixed(1) + '%';
                document.getElementById('s-devs').textContent = fmt(d.total_developers);
                document.getElementById('s-projects').textContent = fmt(d.total_projects);
            }} catch(e) {{ console.error(e); }}

            try {{
                const r = await fetch('/api/v1/aggregate/developers');
                const d = await r.json();
                const top = (d.developers || []).slice(0, 5);
                const maxLines = top.length ? Math.max(...top.map(x => x.total_added_lines || 0)) : 1;
                document.getElementById('top-developers').innerHTML = top.map(dev => {{
                    const total = dev.total_added_lines || 0;
                    const ai = dev.ai_added_lines || 0;
                    const human = dev.human_added_lines || 0;
                    const aiW = maxLines > 0 ? (ai/maxLines*100) : 0;
                    const humanW = maxLines > 0 ? (human/maxLines*100) : 0;
                    const displayName = dev.name || dev.email || '未知';
                    const displayEmail = dev.email || '';
                    return `<div class="chart-bar">
                        <div class="chart-label" title="${{displayName}} ${{displayEmail}}">${{displayName}}</div>
                        <div class="chart-track"><div class="chart-fill"><div class="ai-part" style="width:${{aiW}}%"></div><div class="human-part" style="width:${{humanW}}%"></div></div></div>
                        <div class="chart-value">${{fmt(total)}} <span class="badge ai">${{(ai/(total||1)*100).toFixed(0)}}% AI</span></div>
                    </div>`;
                }}).join('') || '<div class="empty-state"><div class="empty-icon">📭</div><p>暂无开发者数据</p></div>';
            }} catch(e) {{ console.error(e); }}

            // Load mini trend chart for overview
            loadOverviewTrend();
        }}

        async function loadOverviewTrend() {{
            try {{
                const r = await fetch('/api/v1/aggregate/trends?metric=ai_lines&granularity=week');
                const d = await r.json();
                const data = d.data || [];
                if (data.length === 0) return;

                const labels = data.map(p => p.period);
                const aiValues = data.map(p => p.ai_lines);
                const humanValues = data.map(p => p.human_lines);

                if (overviewTrendChart) overviewTrendChart.destroy();
                const ctx = document.getElementById('overview-trend-chart').getContext('2d');
                overviewTrendChart = new Chart(ctx, {{
                    type: 'line',
                    data: {{
                        labels,
                        datasets: [
                            {{ label: 'AI 代码行', data: aiValues, borderColor: '#818cf8', backgroundColor: 'rgba(129,140,248,0.1)', fill: true, tension: 0.3 }},
                            {{ label: '人工代码行', data: humanValues, borderColor: '#34d399', backgroundColor: 'rgba(52,211,153,0.1)', fill: true, tension: 0.3 }},
                        ]
                    }},
                    options: {{
                        responsive: true, maintainAspectRatio: false,
                        plugins: {{ legend: {{ labels: {{ color: '#94a3b8' }} }} }},
                        scales: {{
                            x: {{ ticks: {{ color: '#64748b', maxRotation: 45 }}, grid: {{ color: '#1e293b' }} }},
                            y: {{ ticks: {{ color: '#64748b' }}, grid: {{ color: '#1e293b' }} }},
                        }}
                    }}
                }});
            }} catch(e) {{ console.error(e); }}
        }}

        // --- Trends ---
        async function loadTrends() {{
            const metric = document.getElementById('trend-metric').value;
            const granularity = document.getElementById('trend-granularity').value;

            const metricLabels = {{ ai_ratio: 'AI 占比', ai_lines: 'AI 代码行数', human_lines: '人工代码行数', commits: '提交数' }};
            const granLabels = {{ day: '按天', week: '按周', month: '按月' }};
            document.getElementById('trend-chart-title').textContent =
                `${{metricLabels[metric]}}趋势（${{granLabels[granularity]}}）`;

            try {{
                const r = await fetch(`/api/v1/aggregate/trends?metric=${{metric}}&granularity=${{granularity}}`);
                const d = await r.json();
                const data = d.data || [];

                const labels = data.map(p => p.period);
                const values = data.map(p => p.value);

                if (trendChart) trendChart.destroy();
                const ctx = document.getElementById('trend-chart').getContext('2d');
                trendChart = new Chart(ctx, {{
                    type: 'line',
                    data: {{
                        labels,
                        datasets: [{{
                            label: metricLabels[metric],
                            data: values,
                            borderColor: '#818cf8',
                            backgroundColor: 'rgba(129,140,248,0.15)',
                            fill: true, tension: 0.3, pointRadius: 3,
                        }}]
                    }},
                    options: {{
                        responsive: true, maintainAspectRatio: false,
                        plugins: {{ legend: {{ labels: {{ color: '#94a3b8' }} }} }},
                        scales: {{
                            x: {{ ticks: {{ color: '#64748b', maxRotation: 45 }}, grid: {{ color: '#1e293b' }} }},
                            y: {{ ticks: {{ color: '#64748b' }}, grid: {{ color: '#1e293b' }} }},
                        }}
                    }}
                }});
            }} catch(e) {{ console.error(e); }}

            // Agent comparison chart
            try {{
                const r = await fetch('/api/v1/aggregate/agent-comparison');
                const d = await r.json();
                const comps = (d.comparisons || []).slice(0, 10);
                if (comps.length > 0) {{
                    const labels = comps.map(c => c.tool_model);
                    const aiData = comps.map(c => c.ai_additions || 0);

                    if (agentComparisonChart) agentComparisonChart.destroy();
                    const ctx = document.getElementById('agent-comparison-chart').getContext('2d');
                    agentComparisonChart = new Chart(ctx, {{
                        type: 'bar',
                        data: {{
                            labels,
                            datasets: [{{
                                label: 'AI 代码行数',
                                data: aiData,
                                backgroundColor: 'rgba(129,140,248,0.7)',
                                borderColor: '#818cf8',
                                borderWidth: 1,
                            }}]
                        }},
                        options: {{
                            responsive: true, maintainAspectRatio: false, indexAxis: 'y',
                            plugins: {{ legend: {{ labels: {{ color: '#94a3b8' }} }} }},
                            scales: {{
                                x: {{ ticks: {{ color: '#64748b' }}, grid: {{ color: '#1e293b' }} }},
                                y: {{ ticks: {{ color: '#94a3b8' }}, grid: {{ color: '#1e293b' }} }},
                            }}
                        }}
                    }});
                }}
            }} catch(e) {{ console.error(e); }}
        }}

        // --- Organizations ---
        async function loadOrgs() {{
            try {{
                const r = await fetch('/api/v1/aggregate/organizations');
                const d = await r.json();
                document.getElementById('org-table').innerHTML = (d.organizations || []).map(o => {{
                    return `<tr>
                        <td><strong>${{o.organization}}</strong><br><span style="color:var(--text-muted);font-size:0.75rem">${{o.org_slug || ''}}</span></td>
                        <td>${{fmt(o.total_commits)}}</td>
                        <td>${{fmt(o.w_ai)}}</td>
                        <td>${{fmt(o.w_human)}}</td>
                        <td>${{pctBar(o.pct_ai || 0)}} <span style="font-size:0.8rem">${{(o.pct_ai || 0).toFixed(1)}}%</span></td>
                    </tr>`;
                }}).join('') || '<tr><td colspan="5" style="color:var(--text-muted)">暂无组织数据</td></tr>';
            }} catch(e) {{ console.error(e); }}
        }}

        // --- Developers ---
        async function loadDevs() {{
            try {{
                const r = await fetch('/api/v1/aggregate/developers');
                const d = await r.json();
                document.getElementById('dev-table').innerHTML = (d.developers || []).map(dev => {{
                    const total = dev.total_added_lines || 0;
                    const ai = dev.ai_added_lines || 0;
                    const emailDisplay = dev.email || '—';
                    const nameDisplay = dev.name || '';
                    const label = nameDisplay && nameDisplay !== emailDisplay
                        ? `<strong>${{nameDisplay}}</strong><br><span style="color:var(--text-muted);font-size:0.75rem">${{emailDisplay}}</span>`
                        : `<strong>${{emailDisplay}}</strong>`;
                    return `<tr>
                        <td>${{label}}</td>
                        <td>${{fmt(dev.total_commits)}}</td>
                        <td>${{fmt(total)}}</td>
                        <td>${{fmt(ai)}}</td>
                        <td>${{fmt(dev.human_added_lines)}}</td>
                        <td>${{pctBar(dev.pct_ai || 0)}} <span style="font-size:0.8rem">${{(dev.pct_ai || 0).toFixed(1)}}%</span></td>
                    </tr>`;
                }}).join('') || '<tr><td colspan="6" style="color:var(--text-muted)">暂无开发者数据</td></tr>';
            }} catch(e) {{ console.error(e); }}
        }}

        // --- Projects ---
        async function loadProjects() {{
            try {{
                const r = await fetch('/api/v1/aggregate/projects');
                const d = await r.json();
                document.getElementById('proj-table').innerHTML = (d.projects || []).map(p => {{
                    const displayName = p.project_name || (p.repo_url ? p.repo_url.split('/').pop() : '—');
                    const displayUrl = p.repo_url || p.remote_url_hash || '';
                    return `<tr>
                        <td title="${{displayUrl}}"><strong>${{displayName}}</strong></td>
                        <td>${{p.branch || '—'}}</td>
                        <td>${{fmt(p.total_commits)}}</td>
                        <td>${{fmt(p.total_ai)}}</td>
                        <td>${{fmt(p.total_human)}}</td>
                        <td>${{pctBar(p.pct_ai || 0)}} <span style="font-size:0.8rem">${{(p.pct_ai || 0).toFixed(1)}}%</span></td>
                    </tr>`;
                }}).join('') || '<tr><td colspan="6" style="color:var(--text-muted)">暂无项目数据</td></tr>';
            }} catch(e) {{ console.error(e); }}
        }}

        // --- Tools ---
        async function loadTools() {{
            try {{
                const r = await fetch('/api/v1/aggregate/tools');
                const d = await r.json();
                const tools = (d.tools || []);
                if (tools.length === 0) {{
                    document.getElementById('tools-table').innerHTML =
                        '<tr><td colspan="5" style="color:var(--text-muted)">暂无工具使用数据，数据将在报告上传或指标事件后显示</td></tr>';
                    return;
                }}
                document.getElementById('tools-table').innerHTML = tools.map(t => {{
                    const ai = t.ai_additions || 0;
                    const mixed = t.mixed_additions || 0;
                    const accepted = t.ai_accepted || 0;
                    const total = (t.total_ai_additions || 0) + ai;
                    const source = t.source === 'report'
                        ? '<span class="badge human" style="margin-left:0.5rem">报告</span>'
                        : '<span class="badge ai" style="margin-left:0.5rem">指标</span>';
                    return `<tr>
                        <td><strong>${{t.tool_model}}</strong>${{source}}</td>
                        <td>${{fmt(ai)}}</td>
                        <td>${{fmt(mixed)}}</td>
                        <td>${{fmt(accepted)}}</td>
                        <td>${{fmt(total)}}</td>
                    </tr>`;
                }}).join('');
            }} catch(e) {{ console.error(e); }}
        }}

        // --- Users Management ---
        async function loadUsers() {{
            try {{
                const r = await fetch('/api/admin/users/list');
                const d = await r.json();
                const users = d.users || [];
                if (users.length === 0) {{
                    document.getElementById('users-table').innerHTML =
                        '<tr><td colspan="5"><div class="empty-state"><div class="empty-icon">👤</div><p>暂无用户，点击上方按钮创建</p></div></td></tr>';
                    return;
                }}
                // For each user, also load their API keys
                const usersWithKeys = await Promise.all(users.map(async u => {{
                    try {{
                        const kr = await fetch(`/api/admin/users/${{u.id}}/api-keys`);
                        if (kr.ok) {{
                            const kd = await kr.json();
                            return {{ ...u, apiKeys: kd.api_keys || [] }};
                        }}
                    }} catch(e) {{}}
                    return {{ ...u, apiKeys: [] }};
                }}));

                document.getElementById('users-table').innerHTML = usersWithKeys.map(u => {{
                    const keyCount = u.apiKeys.length;
                    const keyBadges = u.apiKeys.slice(0, 3).map(k =>
                        `<span class="badge ai" style="margin-right:0.25rem">${{k.key_prefix}}...</span>`
                    ).join('');
                    const moreKeys = keyCount > 3 ? `<span style="color:var(--text-muted);font-size:0.75rem">+${{keyCount-3}}</span>` : '';
                    const created = u.created_at ? new Date(u.created_at).toLocaleDateString('zh-CN') : '—';
                    return `<tr>
                        <td><strong>${{u.name || '—'}}</strong></td>
                        <td>${{u.email}}</td>
                        <td>${{keyCount > 0 ? keyBadges + moreKeys : '<span style="color:var(--text-muted)">无密钥</span>'}}</td>
                        <td>${{created}}</td>
                        <td>
                            <button class="btn btn-sm" onclick="showCreateApiKeyForUser('${{u.id}}','${{u.name || u.email}}')">🔑 创建密钥</button>
                            <button class="btn btn-sm btn-danger" onclick="deleteUser('${{u.id}}','${{u.name || u.email}}')">删除</button>
                        </td>
                    </tr>`;
                }}).join('');
            }} catch(e) {{
                console.error(e);
                document.getElementById('users-table').innerHTML =
                    '<tr><td colspan="5" style="color:var(--danger)">加载用户列表失败</td></tr>';
            }}
        }}

        function showCreateUserModal() {{
            document.getElementById('modal-container').innerHTML = `
            <div class="modal-overlay" onclick="if(event.target===this)closeModal()">
                <div class="modal">
                    <div class="modal-title">创建用户</div>
                    <div class="form-group">
                        <label class="form-label">用户名</label>
                        <input type="text" id="create-user-name" class="form-input" placeholder="请输入用户名" />
                    </div>
                    <div class="form-group">
                        <label class="form-label">邮箱</label>
                        <input type="email" id="create-user-email" class="form-input" placeholder="请输入邮箱地址" />
                    </div>
                    <div class="form-group">
                        <label style="display:flex;align-items:center;gap:0.5rem;color:var(--text-secondary);font-size:0.85rem;cursor:pointer">
                            <input type="checkbox" id="create-user-nonce" checked />
                            生成安装令牌（一次性登录凭证）
                        </label>
                    </div>
                    <div class="form-actions">
                        <button class="btn" onclick="closeModal()">取消</button>
                        <button class="btn btn-primary" onclick="createUser()">创建</button>
                    </div>
                </div>
            </div>`;
        }}

        async function createUser() {{
            const name = document.getElementById('create-user-name').value.trim();
            const emailVal = document.getElementById('create-user-email').value.trim();
            const genNonce = document.getElementById('create-user-nonce').checked;

            if (!name || !emailVal) {{
                showToast('请填写用户名和邮箱', 'error');
                return;
            }}

            try {{
                const r = await fetch('/api/admin/users', {{
                    method: 'POST',
                    headers: {{ 'Content-Type': 'application/json' }},
                    body: JSON.stringify({{ name, email: emailVal, generate_nonce: genNonce }})
                }});
                const d = await r.json();
                if (r.ok) {{
                    let msg = `用户 ${{name}} 创建成功！`;
                    if (d.install_nonce) msg += `\\n安装令牌: ${{d.install_nonce}}`;
                    showToast(msg, 'success');
                    closeModal();
                    loadUsers();
                }} else {{
                    showToast(`创建失败: ${{d.error || '未知错误'}}`, 'error');
                }}
            }} catch(e) {{
                showToast('创建用户时发生错误', 'error');
            }}
        }}

        async function deleteUser(userId, userName) {{
            if (!confirm(`确定要删除用户「${{userName}}」吗？此操作不可撤销。`)) return;
            try {{
                const r = await fetch(`/api/admin/users/${{userId}}`, {{ method: 'DELETE' }});
                if (r.ok) {{
                    showToast(`用户「${{userName}}」已删除`, 'success');
                    loadUsers();
                }} else {{
                    const d = await r.json();
                    showToast(`删除失败: ${{d.error || '未知错误'}}`, 'error');
                }}
            }} catch(e) {{
                showToast('删除用户时发生错误', 'error');
            }}
        }}

        // --- API Key Management ---
        async function loadApiKeys() {{
            try {{
                const r = await fetch('/api/admin/api-keys');
                const d = await r.json();
                const keys = d.api_keys || [];
                if (keys.length === 0) {{
                    document.getElementById('apikeys-table').innerHTML =
                        '<tr><td colspan="7"><div class="empty-state"><div class="empty-icon">🔑</div><p>暂无 API 密钥，点击上方按钮创建</p></div></td></tr>';
                    return;
                }}
                document.getElementById('apikeys-table').innerHTML = keys.map(k => {{
                    const created = k.created_at ? new Date(k.created_at).toLocaleDateString('zh-CN') : '—';
                    const expires = k.expires_at ? new Date(k.expires_at).toLocaleDateString('zh-CN') : '永不过期';
                    const lastUsed = k.last_used_at ? new Date(k.last_used_at).toLocaleString('zh-CN') : '从未使用';
                    const scopes = (k.scopes || []).map(s => `<span class="badge role" style="margin:0.1rem">${{s}}</span>`).join(' ');
                    return `<tr>
                        <td><strong>${{k.name || '未命名'}}</strong></td>
                        <td><code style="color:var(--accent);font-size:0.8rem">${{k.key_prefix}}...</code></td>
                        <td>${{scopes}}</td>
                        <td>${{created}}</td>
                        <td>${{expires}}</td>
                        <td style="font-size:0.8rem">${{lastUsed}}</td>
                        <td><button class="btn btn-sm btn-danger" onclick="revokeApiKey('${{k.id}}','${{k.name || k.key_prefix}}')">撤销</button></td>
                    </tr>`;
                }}).join('');
            }} catch(e) {{
                console.error(e);
                document.getElementById('apikeys-table').innerHTML =
                    '<tr><td colspan="7" style="color:var(--danger)">加载密钥列表失败</td></tr>';
            }}
        }}

        function showCreateApiKeyModal() {{
            document.getElementById('modal-container').innerHTML = `
            <div class="modal-overlay" onclick="if(event.target===this)closeModal()">
                <div class="modal">
                    <div class="modal-title">创建 API 密钥</div>
                    <div class="form-group">
                        <label class="form-label">密钥名称</label>
                        <input type="text" id="create-key-name" class="form-input" placeholder="例如：CI/CD 流水线" />
                    </div>
                    <div class="form-group">
                        <label class="form-label">权限范围</label>
                        <div style="display:flex;flex-wrap:wrap;gap:0.5rem;margin-top:0.25rem">
                            <label style="display:flex;align-items:center;gap:0.25rem;font-size:0.8rem;color:var(--text-secondary)">
                                <input type="checkbox" class="key-scope" value="metrics:write" checked /> 指标写入
                            </label>
                            <label style="display:flex;align-items:center;gap:0.25rem;font-size:0.8rem;color:var(--text-secondary)">
                                <input type="checkbox" class="key-scope" value="cas:write" checked /> CAS 写入
                            </label>
                            <label style="display:flex;align-items:center;gap:0.25rem;font-size:0.8rem;color:var(--text-secondary)">
                                <input type="checkbox" class="key-scope" value="cas:read" checked /> CAS 读取
                            </label>
                            <label style="display:flex;align-items:center;gap:0.25rem;font-size:0.8rem;color:var(--text-secondary)">
                                <input type="checkbox" class="key-scope" value="reports:write" checked /> 报告写入
                            </label>
                        </div>
                    </div>
                    <div id="new-key-result" style="display:none">
                        <div class="form-label" style="color:var(--warning);margin-top:1rem">⚠️ 请妥善保存此密钥，关闭后将无法再次查看</div>
                        <div class="api-key-display" id="new-key-value">
                            <button class="copy-btn" onclick="copyKey()">复制</button>
                        </div>
                    </div>
                    <div class="form-actions">
                        <button class="btn" onclick="closeModal()">关闭</button>
                        <button class="btn btn-primary" id="create-key-btn" onclick="createApiKey()">创建</button>
                    </div>
                </div>
            </div>`;
        }}

        function showCreateApiKeyForUser(userId, userName) {{
            document.getElementById('modal-container').innerHTML = `
            <div class="modal-overlay" onclick="if(event.target===this)closeModal()">
                <div class="modal">
                    <div class="modal-title">为用户「${{userName}}」创建 API 密钥</div>
                    <input type="hidden" id="create-key-user-id" value="${{userId}}" />
                    <div class="form-group">
                        <label class="form-label">密钥名称</label>
                        <input type="text" id="create-key-name" class="form-input" placeholder="例如：CI/CD 流水线" />
                    </div>
                    <div class="form-group">
                        <label class="form-label">权限范围</label>
                        <div style="display:flex;flex-wrap:wrap;gap:0.5rem;margin-top:0.25rem">
                            <label style="display:flex;align-items:center;gap:0.25rem;font-size:0.8rem;color:var(--text-secondary)">
                                <input type="checkbox" class="key-scope" value="metrics:write" checked /> 指标写入
                            </label>
                            <label style="display:flex;align-items:center;gap:0.25rem;font-size:0.8rem;color:var(--text-secondary)">
                                <input type="checkbox" class="key-scope" value="cas:write" checked /> CAS 写入
                            </label>
                            <label style="display:flex;align-items:center;gap:0.25rem;font-size:0.8rem;color:var(--text-secondary)">
                                <input type="checkbox" class="key-scope" value="cas:read" checked /> CAS 读取
                            </label>
                            <label style="display:flex;align-items:center;gap:0.25rem;font-size:0.8rem;color:var(--text-secondary)">
                                <input type="checkbox" class="key-scope" value="reports:write" checked /> 报告写入
                            </label>
                        </div>
                    </div>
                    <div id="new-key-result" style="display:none">
                        <div class="form-label" style="color:var(--warning);margin-top:1rem">⚠️ 请妥善保存此密钥，关闭后将无法再次查看</div>
                        <div class="api-key-display" id="new-key-value">
                            <button class="copy-btn" onclick="copyKey()">复制</button>
                        </div>
                    </div>
                    <div class="form-actions">
                        <button class="btn" onclick="closeModal()">关闭</button>
                        <button class="btn btn-primary" id="create-key-btn" onclick="createApiKeyForUser()">创建</button>
                    </div>
                </div>
            </div>`;
        }}

        async function createApiKey() {{
            const name = document.getElementById('create-key-name').value.trim();
            if (!name) {{ showToast('请填写密钥名称', 'error'); return; }}

            const scopes = Array.from(document.querySelectorAll('.key-scope:checked')).map(cb => cb.value);
            if (scopes.length === 0) {{ showToast('请至少选择一个权限范围', 'error'); return; }}

            try {{
                const r = await fetch('/api/admin/api-keys', {{
                    method: 'POST',
                    headers: {{ 'Content-Type': 'application/json' }},
                    body: JSON.stringify({{ name, scopes }})
                }});
                const d = await r.json();
                if (r.ok) {{
                    document.getElementById('new-key-result').style.display = 'block';
                    document.getElementById('new-key-value').innerHTML =
                        `<button class="copy-btn" onclick="copyKey()">复制</button>${{d.key}}`;
                    document.getElementById('create-key-btn').style.display = 'none';
                    showToast('API 密钥创建成功', 'success');
                }} else {{
                    showToast(`创建失败: ${{d.error || '未知错误'}}`, 'error');
                }}
            }} catch(e) {{
                showToast('创建密钥时发生错误', 'error');
            }}
        }}

        async function createApiKeyForUser() {{
            const name = document.getElementById('create-key-name').value.trim();
            const userId = document.getElementById('create-key-user-id').value;
            if (!name) {{ showToast('请填写密钥名称', 'error'); return; }}

            const scopes = Array.from(document.querySelectorAll('.key-scope:checked')).map(cb => cb.value);
            if (scopes.length === 0) {{ showToast('请至少选择一个权限范围', 'error'); return; }}

            try {{
                const r = await fetch('/api/admin/api-keys', {{
                    method: 'POST',
                    headers: {{ 'Content-Type': 'application/json' }},
                    body: JSON.stringify({{ name, scopes, user_id: userId }})
                }});
                const d = await r.json();
                if (r.ok) {{
                    document.getElementById('new-key-result').style.display = 'block';
                    document.getElementById('new-key-value').innerHTML =
                        `<button class="copy-btn" onclick="copyKey()">复制</button>${{d.key}}`;
                    document.getElementById('create-key-btn').style.display = 'none';
                    showToast('API 密钥创建成功', 'success');
                }} else {{
                    showToast(`创建失败: ${{d.error || '未知错误'}}`, 'error');
                }}
            }} catch(e) {{
                showToast('创建密钥时发生错误', 'error');
            }}
        }}

        function copyKey() {{
            const keyEl = document.getElementById('new-key-value');
            const text = keyEl.textContent.replace('复制', '').trim();
            navigator.clipboard.writeText(text).then(() => showToast('已复制到剪贴板', 'success'));
        }}

        async function revokeApiKey(keyId, keyName) {{
            if (!confirm(`确定要撤销密钥「${{keyName}}」吗？撤销后此密钥将立即失效。`)) return;
            try {{
                const r = await fetch(`/api/admin/api-keys/${{keyId}}`, {{ method: 'DELETE' }});
                if (r.ok) {{
                    showToast(`密钥「${{keyName}}」已撤销`, 'success');
                    loadApiKeys();
                }} else {{
                    showToast('撤销失败', 'error');
                }}
            }} catch(e) {{
                showToast('撤销密钥时发生错误', 'error');
            }}
        }}

        function closeModal() {{
            document.getElementById('modal-container').innerHTML = '';
        }}

        // --- Init ---
        loadOverview();
        updateRefreshTime();
        startAutoRefresh();
    </script>
</body>
</html>"##,
        name = auth.name,
        email = auth.email,
    )).into_response()
}

#[derive(Debug, Deserialize)]
pub struct AggregateQuery {
    pub org: Option<String>,
}

/// GET /api/v1/aggregate/summary — Global aggregate summary
pub async fn aggregate_summary(
    State(state): State<AppState>,
    auth: DashboardAuth,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);

    let row: (Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(
        r#"SELECT
            COUNT(*) as total_commits,
            COALESCE(SUM(ai_additions), 0) as total_ai_lines,
            COALESCE(SUM(human_additions), 0) as total_human_lines,
            COUNT(DISTINCT author_email) as total_developers,
            COUNT(DISTINCT repo_url) as total_projects
        FROM metrics_events WHERE event_type = 1
          AND ($1::uuid IS NULL OR user_id = $1)
          AND ($2::uuid IS NULL OR org_id = $2)"#
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let report_row: (Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(
        r#"SELECT
            COUNT(cs.sha) as total_commits,
            COALESCE(SUM(cs.ai_additions), 0) as total_ai_lines,
            COALESCE(SUM(cs.human_additions), 0) as total_human_lines,
            COUNT(DISTINCT cs.author) as total_developers,
            COUNT(DISTINCT p.id) as total_projects
        FROM projects p
        JOIN commit_stats cs ON cs.project_id = p.id
        WHERE ($1::uuid IS NULL OR p.user_id = $1)
          AND ($2::uuid IS NULL OR p.org_id = $2)
          AND NOT EXISTS (
              SELECT 1 FROM metrics_events m
              WHERE m.event_type = 1
                AND m.commit_sha = cs.sha
                AND ($1::uuid IS NULL OR m.user_id = $1)
                AND ($2::uuid IS NULL OR m.org_id = $2)
          )"#,
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_one(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let total_commits = row.0.unwrap_or(0) + report_row.0.unwrap_or(0);
    let total_ai = row.1.unwrap_or(0) + report_row.1.unwrap_or(0);
    let total_human = row.2.unwrap_or(0) + report_row.2.unwrap_or(0);
    let total_developers = row.3.unwrap_or(0) + report_row.3.unwrap_or(0);
    let total_projects = row.4.unwrap_or(0) + report_row.4.unwrap_or(0);
    let total = total_ai + total_human;
    let pct_ai = if total > 0 { (total_ai as f64 / total as f64) * 100.0 } else { 0.0 };

    Ok(Json(json!({
        "total_commits": total_commits,
        "total_ai_lines": total_ai,
        "total_human_lines": total_human,
        "pct_ai_lines": pct_ai,
        "total_developers": total_developers,
        "total_projects": total_projects,
    })))
}

/// GET /api/v1/aggregate/organizations
pub async fn aggregate_organizations(
    State(state): State<AppState>,
    auth: DashboardAuth,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);

    let rows: Vec<(String, String, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"SELECT
            o.name, o.slug,
            COUNT(m.id),
            COALESCE(SUM(m.ai_additions), 0),
            COALESCE(SUM(m.human_additions), 0)
        FROM organizations o
        LEFT JOIN metrics_events m ON m.org_id = o.id AND m.event_type = 1
          AND ($1::uuid IS NULL OR m.user_id = $1)
        WHERE ($2::uuid IS NULL OR o.id = $2)
        GROUP BY o.id, o.name, o.slug
        ORDER BY o.name"#
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let result: Vec<Value> = rows.iter().map(|(name, slug, commits, ai, human)| {
        let ai = ai.unwrap_or(0);
        let human = human.unwrap_or(0);
        let total = ai + human;
        json!({
            "organization": name,
            "org_slug": slug,
            "total_commits": commits.unwrap_or(0),
            "w_ai": ai,
            "w_human": human,
            "pct_ai": if total > 0 { (ai as f64 / total as f64) * 100.0 } else { 0.0 },
        })
    }).collect();

    Ok(Json(json!({ "organizations": result })))
}

/// GET /api/v1/aggregate/departments
pub async fn aggregate_departments(
    State(state): State<AppState>,
    auth: DashboardAuth,
    Query(query): Query<AggregateQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);

    let rows: Vec<(String, String, String, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"SELECT
            d.name, d.slug, o.name as org_name,
            COUNT(m.id),
            COALESCE(SUM(m.ai_additions), 0),
            COALESCE(SUM(m.human_additions), 0)
        FROM departments d
        JOIN organizations o ON d.org_id = o.id
        LEFT JOIN org_members om ON om.department_id = d.id AND om.org_id = d.org_id
        LEFT JOIN metrics_events m ON m.user_id = om.user_id AND m.org_id = om.org_id AND m.event_type = 1
          AND ($1::uuid IS NULL OR m.user_id = $1)
        WHERE ($2::text IS NULL OR o.slug = $2)
          AND ($3::uuid IS NULL OR o.id = $3)
        GROUP BY d.id, d.name, d.slug, o.name
        ORDER BY o.name, d.name"#
    )
    .bind(user_filter)
    .bind(&query.org)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let result: Vec<Value> = rows.iter().map(|(name, slug, org_name, commits, ai, human)| {
        json!({
            "department": name,
            "dept_slug": slug,
            "organization": org_name,
            "total_commits": commits.unwrap_or(0),
            "w_ai": ai.unwrap_or(0),
            "w_human": human.unwrap_or(0),
        })
    }).collect();

    Ok(Json(json!({ "departments": result })))
}

/// GET /api/v1/aggregate/projects
pub async fn aggregate_projects(
    State(state): State<AppState>,
    auth: DashboardAuth,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);

    // Aggregate from metrics_events (primary source from client auto-upload)
    let metrics_rows: Vec<(String, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"SELECT
            repo_url,
            COUNT(*) as total_commits,
            COALESCE(SUM(ai_additions), 0),
            COALESCE(SUM(human_additions), 0)
        FROM metrics_events
        WHERE event_type = 1 AND repo_url IS NOT NULL AND repo_url != ''
          AND ($1::uuid IS NULL OR user_id = $1)
          AND ($2::uuid IS NULL OR org_id = $2)
        GROUP BY repo_url
        ORDER BY repo_url"#
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Also aggregate from projects + commit_stats (legacy report upload source)
    let report_rows: Vec<(i64, String, Option<String>, Option<String>, Option<String>, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"SELECT
            p.id, p.remote_url_hash, p.branch, p.organization, p.department,
            COUNT(cs.sha),
            COALESCE(SUM(cs.ai_additions), 0),
            COALESCE(SUM(cs.human_additions), 0)
        FROM projects p
        LEFT JOIN commit_stats cs ON cs.project_id = p.id
        WHERE ($1::uuid IS NULL OR p.user_id = $1)
          AND ($2::uuid IS NULL OR p.org_id = $2)
        GROUP BY p.id, p.remote_url_hash, p.branch, p.organization, p.department
        ORDER BY p.organization, p.department"#
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Merge: metrics_events data by repo_url, supplemented by report data
    let mut seen_urls = std::collections::HashSet::new();
    let mut result = Vec::new();

    // First pass: metrics_events data
    for (repo_url, commits, ai, human) in &metrics_rows {
        let ai = ai.unwrap_or(0);
        let human = human.unwrap_or(0);
        let total = ai + human;
        seen_urls.insert(repo_url.clone());
        // Extract a human-readable project name from the repo URL
        let project_name = repo_url
            .trim_end_matches('/')
            .split('/')
            .last()
            .unwrap_or(repo_url)
            .trim_end_matches(".git")
            .to_string();
        result.push(json!({
            "repo_url": repo_url,
            "project_name": project_name,
            "total_commits": commits.unwrap_or(0),
            "total_ai": ai,
            "total_human": human,
            "pct_ai": if total > 0 { (ai as f64 / total as f64) * 100.0 } else { 0.0 },
        }));
    }

    // Second pass: report data (only add if not already covered by metrics)
    for (id, url_hash, branch, org, dept, commits, ai, human) in &report_rows {
        // Try to match by url_hash — skip if we already have this repo from metrics
        let ai = ai.unwrap_or(0);
        let human = human.unwrap_or(0);
        let total = ai + human;
        if !seen_urls.contains(url_hash) {
            let project_name = url_hash
                .trim_end_matches('/')
                .split('/')
                .last()
                .unwrap_or(url_hash)
                .trim_end_matches(".git")
                .to_string();
            result.push(json!({
                "project_id": id,
                "repo_url": url_hash,
                "project_name": project_name,
                "branch": branch,
                "organization": org,
                "department": dept,
                "total_commits": commits.unwrap_or(0),
                "total_ai": ai,
                "total_human": human,
                "pct_ai": if total > 0 { (ai as f64 / total as f64) * 100.0 } else { 0.0 },
            }));
        }
    }

    Ok(Json(json!({ "projects": result })))
}

/// GET /api/v1/aggregate/developers
pub async fn aggregate_developers(
    State(state): State<AppState>,
    auth: DashboardAuth,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);

    // The author_email column may contain "Name <email>" format or just a name.
    // Extract the email portion when present, otherwise use the value as-is.
    let rows: Vec<(String, String, Option<i64>, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"SELECT
            raw_author,
            display_email,
            SUM(commits)::bigint,
            SUM(added)::bigint,
            SUM(ai)::bigint,
            SUM(human)::bigint
        FROM (
            SELECT
                author_email as raw_author,
                CASE
                    WHEN author_email ~ '<[^>]+>' THEN substring(author_email from '<([^>]+)>')
                    ELSE author_email
                END as display_email,
                COUNT(*) as commits,
                COALESCE(SUM(git_diff_added_lines), 0) as added,
                COALESCE(SUM(ai_additions), 0) as ai,
                COALESCE(SUM(human_additions), 0) as human
            FROM metrics_events
            WHERE event_type = 1 AND author_email IS NOT NULL AND author_email != ''
              AND ($1::uuid IS NULL OR user_id = $1)
              AND ($2::uuid IS NULL OR org_id = $2)
            GROUP BY author_email, display_email

            UNION ALL

            SELECT
                cs.author as raw_author,
                CASE
                    WHEN cs.author ~ '<[^>]+>' THEN substring(cs.author from '<([^>]+)>')
                    ELSE cs.author
                END as display_email,
                COUNT(*) as commits,
                COALESCE(SUM(cs.git_diff_added_lines), 0) as added,
                COALESCE(SUM(cs.ai_additions), 0) as ai,
                COALESCE(SUM(cs.human_additions), 0) as human
            FROM projects p
            JOIN commit_stats cs ON cs.project_id = p.id
            WHERE cs.author IS NOT NULL AND cs.author != ''
              AND ($1::uuid IS NULL OR p.user_id = $1)
              AND ($2::uuid IS NULL OR p.org_id = $2)
              AND NOT EXISTS (
                  SELECT 1 FROM metrics_events m
                  WHERE m.event_type = 1
                    AND m.commit_sha = cs.sha
                    AND ($1::uuid IS NULL OR m.user_id = $1)
                    AND ($2::uuid IS NULL OR m.org_id = $2)
              )
            GROUP BY cs.author, display_email
        ) combined
        GROUP BY raw_author, display_email
        ORDER BY SUM(commits) DESC"#
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let result: Vec<Value> = rows.iter().map(|(raw_author, email, commits, added, ai, human)| {
        let ai = ai.unwrap_or(0);
        let human = human.unwrap_or(0);
        let total = ai + human;
        // Extract display name from "Name <email>" format
        let name = if raw_author.contains('<') {
            raw_author.split('<').next().unwrap_or("").trim().to_string()
        } else {
            raw_author.clone()
        };
        json!({
            "email": email,
            "name": name,
            "total_commits": commits.unwrap_or(0),
            "total_added_lines": added.unwrap_or(0),
            "ai_added_lines": ai,
            "human_added_lines": human,
            "pct_ai": if total > 0 { (ai as f64 / total as f64) * 100.0 } else { 0.0 },
        })
    }).collect();

    Ok(Json(json!({ "developers": result })))
}

/// GET /api/v1/aggregate/tools — Tool/Model breakdown statistics
pub async fn aggregate_tools(
    State(state): State<AppState>,
    auth: DashboardAuth,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);

    // First try tool_model_stats from report uploads (richer data)
    let report_rows: Vec<(String, Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"SELECT
            tms.tool_model,
            COALESCE(SUM(tms.ai_additions), 0),
            COALESCE(SUM(tms.mixed_additions), 0),
            COALESCE(SUM(tms.ai_accepted), 0),
            COALESCE(SUM(tms.total_ai_additions), 0),
            COALESCE(SUM(tms.total_ai_deletions), 0)
        FROM tool_model_stats tms
        JOIN projects p ON tms.project_id = p.id
        WHERE ($1::uuid IS NULL OR p.user_id = $1)
          AND ($2::uuid IS NULL OR p.org_id = $2)
        GROUP BY tms.tool_model
        ORDER BY SUM(tms.ai_additions) DESC"#
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // From metrics_events: expand tool_model_pairs JSON array to get per-tool stats
    let metrics_rows: Vec<(Option<serde_json::Value>, Option<i32>, Option<i32>, Option<i32>)> = sqlx::query_as(
        r#"SELECT
            tool_model_pairs,
            ai_additions,
            human_additions,
            mixed_additions
        FROM metrics_events
        WHERE event_type = 1
          AND ($1::uuid IS NULL OR user_id = $1)
          AND ($2::uuid IS NULL OR org_id = $2)"#
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // Also get Checkpoint events (type 4) which have tool/model directly
    let checkpoint_rows: Vec<(Option<String>, Option<String>, Option<i64>)> = sqlx::query_as(
        r#"SELECT
            tool,
            model,
            COALESCE(SUM(ai_additions), 0)
        FROM metrics_events
        WHERE event_type IN (2, 4) AND tool IS NOT NULL AND tool != ''
          AND ($1::uuid IS NULL OR user_id = $1)
          AND ($2::uuid IS NULL OR org_id = $2)
        GROUP BY tool, model
        ORDER BY SUM(ai_additions) DESC"#
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let mut tool_map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();

    // Expand tool_model_pairs from Committed events
    for (pairs_json, ai_add, human_add, mixed_add) in &metrics_rows {
        let ai_total = ai_add.unwrap_or(0) as i64;
        let _human_total = human_add.unwrap_or(0) as i64;
        let mixed_total = mixed_add.unwrap_or(0) as i64;

        if let Some(pairs) = pairs_json {
            if let Some(arr) = pairs.as_array() {
                let count = arr.len().max(1) as i64;
                let ai_per_tool = ai_total / count;
                let mixed_per_tool = mixed_total / count;

                for pair_val in arr {
                    if let Some(pair_str) = pair_val.as_str() {
                        if pair_str == "all" { continue; }
                        *tool_map.entry(pair_str.to_string()).or_insert(0) += ai_per_tool;
                        let mixed_key = format!("{}__mixed", pair_str);
                        *tool_map.entry(mixed_key).or_insert(0) += mixed_per_tool;
                    }
                }
            }
        }
    }

    // Also add Checkpoint/AgentUsage events which have tool/model directly
    for (tool, model, ai_add) in &checkpoint_rows {
        let tool_name = tool.as_deref().unwrap_or("unknown");
        let model_name = model.as_deref().unwrap_or("");
        let tool_model = if model_name.is_empty() {
            tool_name.to_string()
        } else {
            format!("{}::{}", tool_name, model_name)
        };
        *tool_map.entry(tool_model).or_insert(0) += ai_add.unwrap_or(0) as i64;
    }

    let mut tools: Vec<Value> = Vec::new();

    // Add report-based tool stats
    for (tool_model, ai_add, mixed_add, ai_accept, total_ai_add, total_ai_del) in &report_rows {
        tools.push(json!({
            "tool_model": tool_model,
            "source": "report",
            "ai_additions": ai_add.unwrap_or(0),
            "mixed_additions": mixed_add.unwrap_or(0),
            "ai_accepted": ai_accept.unwrap_or(0),
            "total_ai_additions": total_ai_add.unwrap_or(0),
            "total_ai_deletions": total_ai_del.unwrap_or(0),
        }));
    }

    // Add metrics-based tool stats (from tool_model_pairs expansion)
    for (tool_model, ai_additions) in &tool_map {
        // Skip internal mixed-tracking keys
        if tool_model.contains("__mixed") { continue; }

        let mixed_key = format!("{}__mixed", tool_model);
        let mixed_additions = tool_map.get(&mixed_key).copied().unwrap_or(0);

        // Check if this tool_model already exists from report data
        let already_exists = tools.iter().any(|t| {
            t.get("tool_model").and_then(|v| v.as_str()) == Some(tool_model)
                && t.get("source").and_then(|v| v.as_str()) == Some("report")
        });

        if !already_exists {
            tools.push(json!({
                "tool_model": tool_model,
                "source": "metrics",
                "ai_additions": *ai_additions,
                "mixed_additions": mixed_additions,
                "ai_accepted": 0,
                "total_ai_additions": 0,
                "total_ai_deletions": 0,
            }));
        }
    }

    // Sort by ai_additions descending
    tools.sort_by(|a, b| {
        let a_val = a.get("ai_additions").and_then(|v| v.as_i64()).unwrap_or(0);
        let b_val = b.get("ai_additions").and_then(|v| v.as_i64()).unwrap_or(0);
        b_val.cmp(&a_val)
    });

    Ok(Json(json!({ "tools": tools })))
}

// ================================================================
// Phase 6: Advanced Dashboard Enhancement APIs
// ================================================================

#[derive(Debug, Deserialize)]
pub struct TrendsQuery {
    pub metric: Option<String>,        // "ai_ratio", "ai_lines", "human_lines", "commits"
    pub granularity: Option<String>,   // "day", "week", "month"
    pub org: Option<String>,
    pub since: Option<String>,
    pub until: Option<String>,
}

/// GET /api/v1/aggregate/trends — AI code attribution trends over time
pub async fn aggregate_trends(
    State(state): State<AppState>,
    auth: DashboardAuth,
    Query(query): Query<TrendsQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);

    let metric = query.metric.as_deref().unwrap_or("ai_ratio");
    let granularity = query.granularity.as_deref().unwrap_or("week");

    let valid_metrics = ["ai_ratio", "ai_lines", "human_lines", "commits"];
    if !valid_metrics.contains(&metric) {
        return Err(AppError::BadRequest(format!(
            "metric must be one of: {}", valid_metrics.join(", ")
        )));
    }

    let valid_granularities = ["day", "week", "month"];
    if !valid_granularities.contains(&granularity) {
        return Err(AppError::BadRequest(format!(
            "granularity must be one of: {}", valid_granularities.join(", ")
        )));
    }

    let date_trunc = match granularity {
        "day" => "day",
        "week" => "week",
        "month" => "month",
        _ => "week",
    };

    let rows: Vec<(chrono::NaiveDate, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        &format!(r#"SELECT
            period,
            COALESCE(SUM(ai_lines), 0)::bigint AS ai_lines,
            COALESCE(SUM(human_lines), 0)::bigint AS human_lines,
            COALESCE(SUM(commits), 0)::bigint AS commits
        FROM (
            SELECT
                DATE_TRUNC('{0}', created_at)::date AS period,
                COALESCE(SUM(ai_additions), 0)::bigint AS ai_lines,
                COALESCE(SUM(human_additions), 0)::bigint AS human_lines,
                COUNT(*)::bigint AS commits
            FROM metrics_events
            WHERE event_type = 1
              AND ($1::uuid IS NULL OR user_id = $1)
              AND ($2::uuid IS NULL OR org_id = $2)
              AND ($3::text IS NULL OR org_id = (SELECT id FROM organizations WHERE slug = $3))
              AND ($4::timestamptz IS NULL OR created_at >= $4::timestamptz)
              AND ($5::timestamptz IS NULL OR created_at <= $5::timestamptz)
            GROUP BY DATE_TRUNC('{0}', created_at)

            UNION ALL

            SELECT
                DATE_TRUNC('{0}', cs.author_time::timestamptz)::date AS period,
                COALESCE(SUM(cs.ai_additions), 0)::bigint AS ai_lines,
                COALESCE(SUM(cs.human_additions), 0)::bigint AS human_lines,
                COUNT(*)::bigint AS commits
            FROM projects p
            JOIN commit_stats cs ON cs.project_id = p.id
            WHERE cs.author_time IS NOT NULL AND cs.author_time != ''
              AND ($1::uuid IS NULL OR p.user_id = $1)
              AND ($2::uuid IS NULL OR p.org_id = $2)
              AND ($3::text IS NULL OR p.org_id = (SELECT id FROM organizations WHERE slug = $3))
              AND ($4::timestamptz IS NULL OR cs.author_time::timestamptz >= $4::timestamptz)
              AND ($5::timestamptz IS NULL OR cs.author_time::timestamptz <= $5::timestamptz)
              AND NOT EXISTS (
                  SELECT 1 FROM metrics_events m
                  WHERE m.event_type = 1
                    AND m.commit_sha = cs.sha
                    AND ($1::uuid IS NULL OR m.user_id = $1)
                    AND ($2::uuid IS NULL OR m.org_id = $2)
              )
            GROUP BY DATE_TRUNC('{0}', cs.author_time::timestamptz)
        ) combined
        GROUP BY period
        ORDER BY period"#, date_trunc)
    )
    .bind(user_filter)
    .bind(org_filter)
    .bind(&query.org)
    .bind(&query.since)
    .bind(&query.until)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let data: Vec<Value> = rows.iter().map(|(period, ai, human, commits)| {
        let ai = ai.unwrap_or(0);
        let human = human.unwrap_or(0);
        let total = ai + human;
        let ai_ratio = if total > 0 { (ai as f64 / total as f64) * 100.0 } else { 0.0 };

        let value = match metric {
            "ai_ratio" => ai_ratio,
            "ai_lines" => ai as f64,
            "human_lines" => human as f64,
            "commits" => commits.unwrap_or(0) as f64,
            _ => 0.0,
        };

        json!({
            "period": period.to_string(),
            "granularity": granularity,
            "value": (value * 100.0).round() / 100.0,
            "ai_lines": ai,
            "human_lines": human,
            "commits": commits.unwrap_or(0),
            "ai_ratio": (ai_ratio * 100.0).round() / 100.0,
        })
    }).collect();

    Ok(Json(json!({
        "metric": metric,
        "granularity": granularity,
        "data": data,
    })))
}

#[derive(Debug, Deserialize)]
pub struct AgentComparisonQuery {
    pub org: Option<String>,
}

/// GET /api/v1/aggregate/agent-comparison — Compare AI tools/models
pub async fn aggregate_agent_comparison(
    State(state): State<AppState>,
    auth: DashboardAuth,
    Query(query): Query<AgentComparisonQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);
    // From report data
    let report_rows: Vec<(String, Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"SELECT
            tms.tool_model,
            COALESCE(SUM(tms.ai_additions), 0),
            COALESCE(SUM(tms.mixed_additions), 0),
            COALESCE(SUM(tms.ai_accepted), 0),
            COALESCE(SUM(tms.total_ai_additions), 0),
            COALESCE(SUM(tms.total_ai_deletions), 0)
        FROM tool_model_stats tms
        JOIN projects p ON tms.project_id = p.id
        WHERE ($1::uuid IS NULL OR p.user_id = $1)
          AND ($2::uuid IS NULL OR p.org_id = $2)
        GROUP BY tms.tool_model
        ORDER BY SUM(tms.ai_additions) DESC"#
    )
    .bind(user_filter)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    // From metrics events (real-time)
    let metrics_rows: Vec<(Option<String>, Option<String>, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"SELECT
            tool, model,
            COALESCE(SUM(ai_additions), 0),
            COALESCE(SUM(human_additions), 0),
            COUNT(*)
        FROM metrics_events
        WHERE event_type = 1
          AND tool IS NOT NULL
          AND ($1::uuid IS NULL OR user_id = $1)
          AND ($2::uuid IS NULL OR org_id = $2)
          AND ($3::text IS NULL OR org_id = (SELECT id FROM organizations WHERE slug = $3))
        GROUP BY tool, model
        ORDER BY SUM(ai_additions) DESC"#
    )
    .bind(user_filter)
    .bind(org_filter)
    .bind(&query.org)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let mut comparisons: Vec<Value> = Vec::new();

    // Report-based
    for (tool_model, ai_add, mixed_add, ai_accept, total_ai_add, total_ai_del) in &report_rows {
        let ai_add = ai_add.unwrap_or(0);
        let ai_accept = ai_accept.unwrap_or(0);
        let total_ai_add = total_ai_add.unwrap_or(0);
        let acceptance_rate = if ai_add > 0 { (ai_accept as f64 / ai_add as f64) * 100.0 } else { 0.0 };
        let net_ai = total_ai_add - total_ai_del.unwrap_or(0);

        comparisons.push(json!({
            "tool_model": tool_model,
            "source": "report",
            "ai_additions": ai_add,
            "mixed_additions": mixed_add.unwrap_or(0),
            "ai_accepted": ai_accept,
            "total_ai_additions": total_ai_add,
            "total_ai_deletions": total_ai_del.unwrap_or(0),
            "net_ai_lines": net_ai,
            "acceptance_rate": (acceptance_rate * 100.0).round() / 100.0,
        }));
    }

    // Metrics-based (supplementary)
    for (tool, model, ai_add, human_add, commits) in &metrics_rows {
        let tool_name = tool.as_deref().unwrap_or("unknown");
        let model_name = model.as_deref().unwrap_or("");
        let tool_model = if model_name.is_empty() {
            tool_name.to_string()
        } else {
            format!("{}::{}", tool_name, model_name)
        };

        let already = comparisons.iter().any(|c| {
            c.get("tool_model").and_then(|v| v.as_str()) == Some(&tool_model)
                && c.get("source").and_then(|v| v.as_str()) == Some("report")
        });

        if !already {
            let ai = ai_add.unwrap_or(0);
            let human = human_add.unwrap_or(0);
            let total = ai + human;
            let acceptance_rate = if total > 0 { (ai as f64 / total as f64) * 100.0 } else { 0.0 };

            comparisons.push(json!({
                "tool_model": tool_model,
                "source": "metrics",
                "ai_additions": ai,
                "human_additions": human,
                "commits": commits.unwrap_or(0),
                "acceptance_rate": (acceptance_rate * 100.0).round() / 100.0,
            }));
        }
    }

    // Sort by ai_additions descending
    comparisons.sort_by(|a, b| {
        let a_val = a.get("ai_additions").and_then(|v| v.as_i64()).unwrap_or(0);
        let b_val = b.get("ai_additions").and_then(|v| v.as_i64()).unwrap_or(0);
        b_val.cmp(&a_val)
    });

    Ok(Json(json!({ "comparisons": comparisons })))
}

#[derive(Debug, Deserialize)]
pub struct TeamComparisonQuery {
    pub org: Option<String>,
}

/// GET /api/v1/aggregate/team-comparison — Compare AI adoption across teams/departments
pub async fn aggregate_team_comparison(
    State(state): State<AppState>,
    auth: DashboardAuth,
    Query(query): Query<TeamComparisonQuery>,
) -> Result<Json<Value>, AppError> {
    let (user_filter, org_filter) = build_data_filters(&auth.0);

    let rows: Vec<(String, String, String, Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
        r#"SELECT
            d.name AS dept_name,
            d.slug AS dept_slug,
            o.name AS org_name,
            COUNT(m.id) AS total_commits,
            COALESCE(SUM(m.ai_additions), 0) AS ai_lines,
            COALESCE(SUM(m.human_additions), 0) AS human_lines
        FROM departments d
        JOIN organizations o ON d.org_id = o.id
        LEFT JOIN org_members om ON om.department_id = d.id AND om.org_id = d.org_id
        LEFT JOIN metrics_events m ON m.user_id = om.user_id AND m.org_id = om.org_id AND m.event_type = 1
          AND ($1::uuid IS NULL OR m.user_id = $1)
        WHERE ($2::text IS NULL OR o.slug = $2)
          AND ($3::uuid IS NULL OR o.id = $3)
        GROUP BY d.id, d.name, d.slug, o.name
        ORDER BY o.name, d.name"#
    )
    .bind(user_filter)
    .bind(&query.org)
    .bind(org_filter)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Database(e))?;

    let teams: Vec<Value> = rows.iter().map(|(dept_name, dept_slug, org_name, commits, ai, human)| {
        let ai = ai.unwrap_or(0);
        let human = human.unwrap_or(0);
        let total = ai + human;
        let pct_ai = if total > 0 { (ai as f64 / total as f64) * 100.0 } else { 0.0 };

        json!({
            "department": dept_name,
            "dept_slug": dept_slug,
            "organization": org_name,
            "total_commits": commits.unwrap_or(0),
            "ai_lines": ai,
            "human_lines": human,
            "pct_ai": (pct_ai * 100.0).round() / 100.0,
            "adoption_level": if pct_ai >= 60.0 { "high" } else if pct_ai >= 30.0 { "medium" } else { "low" },
        })
    }).collect();

    Ok(Json(json!({ "teams": teams })))
}

/// Build data filter parameters based on the user's role.
/// Returns (user_id_filter, org_id_filter):
/// - Admin users: (None, Some(org_id)) — sees all data within their organization
/// - Non-admin users: (Some(user_id), Some(org_id)) — sees only their own data within their organization
/// - If org_id is not available, falls back to no org filter (should not happen in practice)
pub fn build_data_filters(auth: &crate::models::user::AuthIdentity) -> (Option<uuid::Uuid>, Option<uuid::Uuid>) {
    if auth.is_admin() {
        // Admin sees all data within their organization (no user filter, but org filter applies)
        (None, auth.org_id)
    } else {
        // Non-admin sees only their own data within their organization
        (Some(auth.user_id), auth.org_id)
    }
}
