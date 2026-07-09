const fmt = n => typeof n === 'number' ? n.toLocaleString() : '0';
const pctBar = (pct) => `<div class="bar"><div class="bar-fill" style="width:${Math.min(pct,100)}%"></div></div>`;
function escapeHtml(value) {
    const div = document.createElement('div');
    div.textContent = value ?? '';
    return div.innerHTML;
}
function jsString(value) {
    return JSON.stringify(String(value ?? ''))
        .replace(/&/g, '&amp;')
        .replace(/"/g, '&quot;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;');
}
function fmtTimeAgo(value) {
    if (!value) return '从未';
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return '未知';
    const seconds = Math.max(0, Math.floor((Date.now() - date.getTime()) / 1000));
    if (seconds < 60) return '刚刚';
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes} 分钟前`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours} 小时前`;
    const days = Math.floor(hours / 24);
    return `${days} 天前`;
}

// --- Auto refresh ---
let refreshInterval = null;
const AUTO_REFRESH_MS = 60000; // 60 seconds
let currentSection = 'overview';
const TABLE_PAGE_SIZE = 25;
const tablePageState = {};
const tablePagerContainers = {
    organizations: 'org-pagination',
    departments: 'departments-pagination',
    developers: 'dev-pagination',
    projects: 'proj-pagination',
    tools: 'tools-pagination',
    users: 'users-pagination',
    apikeys: 'apikeys-pagination',
};

function getTablePageState(key) {
    if (!tablePageState[key]) {
        tablePageState[key] = {
            page: 1,
            cursors: [null],
            nextCursor: null,
            hasMore: false,
            loading: false,
        };
    }
    return tablePageState[key];
}

function resetTablePage(key) {
    tablePageState[key] = {
        page: 1,
        cursors: [null],
        nextCursor: null,
        hasMore: false,
        loading: false,
    };
}

function addPaginationParams(url, key) {
    const state = getTablePageState(key);
    const cursor = state.cursors[state.page - 1];
    const params = new URLSearchParams({ limit: String(TABLE_PAGE_SIZE) });
    if (cursor) params.set('cursor', cursor);
    return `${url}${url.includes('?') ? '&' : '?'}${params.toString()}`;
}

async function fetchPaginatedJson(key, url, errorMessage) {
    const state = getTablePageState(key);
    state.loading = true;
    try {
        const r = await fetch(addPaginationParams(url, key));
        const d = await r.json();
        if (!r.ok) {
            throw new Error(d.error || errorMessage);
        }
        const pagination = d.pagination || {};
        state.nextCursor = pagination.next_cursor || null;
        state.hasMore = Boolean(pagination.has_more);
        return d;
    } catch (error) {
        state.nextCursor = null;
        state.hasMore = false;
        throw error;
    } finally {
        state.loading = false;
    }
}

function pageItems(data, field) {
    return (data[field] || []).slice(0, TABLE_PAGE_SIZE);
}

function setTableLoading(tbodyId, colspan) {
    document.getElementById(tbodyId).innerHTML =
        `<tr><td colspan="${colspan}" style="color:var(--text-muted)">加载中...</td></tr>`;
}

function renderPaginationControls(key) {
    const containerId = tablePagerContainers[key];
    const container = containerId ? document.getElementById(containerId) : null;
    if (!container) return;
    const state = getTablePageState(key);
    container.innerHTML = `
        <button class="btn btn-sm" onclick="goToTablePage('${key}', 'prev')" ${state.page <= 1 || state.loading ? 'disabled' : ''}>上一页</button>
        <span class="pagination-status">第 ${state.page} 页</span>
        <button class="btn btn-sm" onclick="goToTablePage('${key}', 'next')" ${!state.hasMore || state.loading ? 'disabled' : ''}>下一页</button>
    `;
}

async function goToTablePage(key, direction) {
    const state = getTablePageState(key);
    if (state.loading) return;
    if (direction === 'next') {
        if (!state.hasMore || !state.nextCursor) return;
        state.cursors[state.page] = state.nextCursor;
        state.page += 1;
    } else if (direction === 'prev') {
        if (state.page <= 1) return;
        state.page -= 1;
        state.cursors = state.cursors.slice(0, state.page);
    }
    await reloadPaginatedTable(key);
}

function reloadPaginatedTable(key) {
    switch (key) {
        case 'organizations': return loadOrgs();
        case 'departments': return loadDepartments();
        case 'developers': return loadDevs();
        case 'projects': return loadProjects();
        case 'tools': return loadTools();
        case 'users': return loadUsers();
        case 'apikeys': return loadApiKeys();
    }
}

async function fetchAllPaginated(url, field) {
    const values = [];
    let cursor = null;
    for (let page = 0; page < 50; page += 1) {
        const params = new URLSearchParams({ limit: '100' });
        if (cursor) params.set('cursor', cursor);
        const r = await fetch(`${url}${url.includes('?') ? '&' : '?'}${params.toString()}`);
        const d = await r.json();
        if (!r.ok) {
            throw new Error(d.error || '加载分页数据失败');
        }
        values.push(...(d[field] || []));
        if (!d.pagination?.has_more || !d.pagination?.next_cursor) break;
        cursor = d.pagination.next_cursor;
    }
    return values;
}

// Role-based UI: hide admin sections for non-admin users
if (!isAdmin) {
    document.querySelectorAll('.admin-only').forEach(el => el.style.display = 'none');
    document.getElementById('admin-nav-section').style.display = 'none';
    document.getElementById('admin-nav-users').style.display = 'none';
    document.getElementById('admin-nav-apikeys').style.display = 'none';
    document.getElementById('gitai-status-card').style.display = '';
    document.getElementById('developer-count-card').style.display = 'none';
}
document.getElementById('sidebar-gitai').style.display = 'none';

function startAutoRefresh() {
    stopAutoRefresh();
    refreshInterval = setInterval(() => refreshCurrentSection(), AUTO_REFRESH_MS);
}
function stopAutoRefresh() {
    if (refreshInterval) { clearInterval(refreshInterval); refreshInterval = null; }
}
function updateRefreshTime() {
    const now = new Date();
    document.getElementById('last-refresh').textContent =
        `上次刷新: ${now.getHours().toString().padStart(2,'0')}:${now.getMinutes().toString().padStart(2,'0')}:${now.getSeconds().toString().padStart(2,'0')}`;
}

function refreshCurrentSection() {
    loadSection(currentSection);
    if (!isAdmin) loadClientStatus();
    updateRefreshTime();
}

// --- Navigation ---
function showSection(id) {
    // Non-admin users cannot access admin sections
    if (!isAdmin && (id === 'users' || id === 'apikeys')) {
        return;
    }
    currentSection = id;
    document.querySelectorAll('.section').forEach(s => s.classList.remove('active'));
    document.querySelectorAll('.nav-item').forEach(n => n.classList.remove('active'));
    document.getElementById('section-' + id).classList.add('active');
    event.currentTarget.classList.add('active');
    loadSection(id);
}

function loadSection(id) {
    switch(id) {
        case 'overview': loadOverview(); break;
        case 'trends': loadTrends(); break;
        case 'organizations': loadOrgs(); break;
        case 'developers': loadDevs(); break;
        case 'projects': loadProjects(); break;
        case 'tools': loadTools(); break;
        case 'users': loadUsers(); break;
        case 'departments': loadDepartments(); break;
        case 'apikeys': loadApiKeys(); break;
    }
}

// --- Toast notifications ---
function showToast(message, type = 'info') {
    const existing = document.querySelector('.toast');
    if (existing) existing.remove();
    const toast = document.createElement('div');
    toast.className = `toast ${type}`;
    toast.textContent = message;
    document.body.appendChild(toast);
    setTimeout(() => toast.remove(), 3000);
}

// --- Time range helper ---
function getTimeRangeParams() {
    const range = document.getElementById('time-range')?.value || '30d';
    if (range === 'all') return '';
    const days = parseInt(range);
    const since = new Date(Date.now() - days * 86400000).toISOString();
    return `since=${encodeURIComponent(since)}`;
}
function withTimeRange(url) {
    const params = getTimeRangeParams();
    if (!params) return url;
    return `${url}${url.includes('?') ? '&' : '?'}${params}`;
}
function getTimeRangeLabel() {
    const range = document.getElementById('time-range')?.value || '30d';
    if (range === 'all') return '全部时间';
    return `最近 ${parseInt(range)} 天`;
}

// --- Chart instances ---
let overviewTrendChart = null;
let trendChart = null;
let agentComparisonChart = null;
let developerGitInfo = new Map();

// --- Overview ---
async function loadOverview() {
    const rangeLabel = getTimeRangeLabel();
    document.getElementById('overview-trend-title').textContent = `AI 代码趋势（${rangeLabel}）`;

    const summaryPromise = (async () => {
        const r = await fetch(withTimeRange('/api/v1/aggregate/summary'));
        const d = await r.json();
        document.getElementById('s-commits').textContent = fmt(d.total_commits);
        document.getElementById('s-ai-lines').textContent = fmt(d.total_ai_lines);
        document.getElementById('s-human-lines').textContent = fmt(d.total_human_lines);
        document.getElementById('s-ai-pct').textContent = (d.pct_ai_lines || 0).toFixed(1) + '%';
        if (isAdmin) document.getElementById('s-devs').textContent = fmt(d.total_developers);
        document.getElementById('s-projects').textContent = fmt(d.total_projects);
    })().catch(e => console.error(e));

    const developersPromise = (async () => {
        const r = await fetch(withTimeRange('/api/v1/aggregate/developers?limit=5'));
        const d = await r.json();
        const top = [...(d.developers || [])]
            .sort((a, b) => (b.ai_added_lines || 0) - (a.ai_added_lines || 0))
            .slice(0, 5);
        const maxLines = top.length ? Math.max(...top.map(x => x.total_added_lines || 0)) : 1;
        document.getElementById('top-developers').innerHTML = top.map(dev => {
            const total = dev.total_added_lines || 0;
            const ai = dev.ai_added_lines || 0;
            const human = dev.human_added_lines || 0;
            const aiW = maxLines > 0 ? (ai/maxLines*100) : 0;
            const humanW = maxLines > 0 ? (human/maxLines*100) : 0;
            const displayName = escapeHtml(dev.name || dev.email || '未知');
            const displayEmail = escapeHtml(dev.email || '');
            return `<div class="chart-bar">
                <div class="chart-label" title="${displayName} ${displayEmail}">${displayName}</div>
                <div class="chart-track"><div class="chart-fill"><div class="ai-part" style="width:${aiW}%"></div><div class="human-part" style="width:${humanW}%"></div></div></div>
                <div class="chart-value">${fmt(total)} <span class="badge ai">${(ai/(total||1)*100).toFixed(0)}% AI</span></div>
            </div>`;
        }).join('') || '<div class="empty-state"><div class="empty-icon">📭</div><p>暂无开发者数据</p></div>';
    })().catch(e => console.error(e));

    const trendPromise = loadOverviewTrend();
    await Promise.allSettled([summaryPromise, developersPromise, trendPromise]);
}

async function loadClientStatus() {
    if (isAdmin) return;
    const cardEl = document.getElementById('sidebar-gitai');
    const statusEl = document.getElementById('sidebar-gitai-status');
    const detailEl = document.getElementById('sidebar-gitai-detail');
    const dotEl = document.getElementById('sidebar-gitai-dot');
    const overviewStatusEl = document.getElementById('s-gitai-status');
    const overviewDetailEl = document.getElementById('s-gitai-detail');
    if ((!cardEl || !statusEl || !detailEl || !dotEl) && !overviewStatusEl) return;
    try {
        const r = await fetch('/api/v1/client/status');
        const d = await r.json();
        if (!d.detected) {
            if (overviewStatusEl) {
                overviewStatusEl.textContent = '未检测到';
                overviewStatusEl.className = 'stat-value human';
            }
            if (overviewDetailEl) {
                overviewDetailEl.textContent = 'CLI 登录后会显示同步信息';
                overviewDetailEl.title = overviewDetailEl.textContent;
            }
            if (statusEl && cardEl && dotEl && detailEl) {
                statusEl.textContent = 'git-ai 未检测到';
                cardEl.className = 'sidebar-gitai';
                dotEl.className = 'sidebar-gitai-dot';
                detailEl.textContent = 'CLI 登录后会显示状态';
                detailEl.title = '';
            }
            return;
        }

        const loggedIn = d.status === 'logged_in';
        const statusLabel = d.status_label || (loggedIn ? '已登录' : '已登出');
        if (overviewStatusEl) {
            overviewStatusEl.textContent = statusLabel;
            overviewStatusEl.className = loggedIn ? 'stat-value ai' : 'stat-value human';
        }
        if (statusEl && cardEl && dotEl) {
            statusEl.textContent = `git-ai ${statusLabel}`;
            cardEl.className = loggedIn ? 'sidebar-gitai online' : 'sidebar-gitai offline';
            dotEl.className = loggedIn ? 'sidebar-gitai-dot online' : 'sidebar-gitai-dot offline';
        }

        const parts = [];
        if (d.last_seen_at) {
            parts.push(`最近同步 ${fmtTimeAgo(d.last_seen_at)}`);
        } else if (d.last_status_at) {
            parts.push(`最近状态 ${fmtTimeAgo(d.last_status_at)}`);
        }
        if ((d.device_count || 0) > 1) parts.push(`${d.device_count} 台设备`);
        if (d.cli_version) parts.push(`v${d.cli_version}`);
        const syncDetail = parts.join(' · ') || '暂无同步记录';
        if (overviewDetailEl) {
            overviewDetailEl.textContent = syncDetail;
            overviewDetailEl.title = syncDetail;
        }
        if (detailEl) detailEl.textContent = syncDetail;
        const titleParts = [...parts];
        if (d.hostname) titleParts.push(d.hostname);
        if (d.os || d.arch) titleParts.push([d.os, d.arch].filter(Boolean).join('/'));
        if (Array.isArray(d.devices) && d.devices.length > 1) {
            titleParts.push(d.devices.map(device => {
                const deviceName = device.hostname || device.device_key || 'unknown';
                const deviceStatus = device.status === 'logged_in' ? '已登录' : '已登出';
                return `${deviceName}: ${deviceStatus}`;
            }).join(' / '));
        }
        if (detailEl) detailEl.title = titleParts.join(' · ');
    } catch(e) {
        console.error(e);
        if (overviewStatusEl) {
            overviewStatusEl.textContent = '检测失败';
            overviewStatusEl.className = 'stat-value pct';
        }
        if (overviewDetailEl) {
            overviewDetailEl.textContent = '无法读取同步信息';
            overviewDetailEl.title = overviewDetailEl.textContent;
        }
        if (statusEl && cardEl && dotEl && detailEl) {
            statusEl.textContent = 'git-ai 检测失败';
            cardEl.className = 'sidebar-gitai error';
            dotEl.className = 'sidebar-gitai-dot error';
            detailEl.textContent = '无法读取状态';
            detailEl.title = '';
        }
    }
}

async function loadOverviewTrend() {
    try {
        const r = await fetch(withTimeRange('/api/v1/aggregate/trends?metric=ai_lines&granularity=day'));
        const d = await r.json();
        const data = d.data || [];
        const canvas = document.getElementById('overview-trend-chart');
        const empty = document.getElementById('overview-trend-empty');
        if (data.length === 0) {
            if (overviewTrendChart) {
                overviewTrendChart.destroy();
                overviewTrendChart = null;
            }
            canvas.style.display = 'none';
            empty.style.display = 'block';
            return;
        }
        canvas.style.display = 'block';
        empty.style.display = 'none';

        const labels = data.map(p => p.period);
        const aiValues = data.map(p => p.ai_lines);
        const humanValues = data.map(p => p.human_lines);

        if (overviewTrendChart) overviewTrendChart.destroy();
        const ctx = canvas.getContext('2d');
        overviewTrendChart = new Chart(ctx, {
            type: 'line',
            data: {
                labels,
                datasets: [
                    { label: 'AI 代码行', data: aiValues, borderColor: '#818cf8', backgroundColor: 'rgba(129,140,248,0.1)', fill: true, tension: 0.3 },
                    { label: '非 AI 代码行', data: humanValues, borderColor: '#34d399', backgroundColor: 'rgba(52,211,153,0.1)', fill: true, tension: 0.3 },
                ]
            },
            options: {
                responsive: true, maintainAspectRatio: false,
                plugins: { legend: { labels: { color: '#94a3b8' } } },
                scales: {
                    x: { ticks: { color: '#64748b', maxRotation: 45 }, grid: { color: '#1e293b' } },
                    y: { ticks: { color: '#64748b' }, grid: { color: '#1e293b' } },
                }
            }
        });
    } catch(e) { console.error(e); }
}

// --- Trends ---
async function loadTrends() {
    const metric = document.getElementById('trend-metric').value;
    const granularity = document.getElementById('trend-granularity').value;

    const metricLabels = { ai_ratio: 'AI 占比', ai_lines: 'AI 代码行数', human_lines: '非 AI 代码行数', commits: '提交数' };
    const granLabels = { day: '按天', week: '按周', month: '按月' };
    document.getElementById('trend-chart-title').textContent =
        `${metricLabels[metric]}趋势（${granLabels[granularity]}）`;

    try {
        const r = await fetch(`/api/v1/aggregate/trends?metric=${metric}&granularity=${granularity}`);
        const d = await r.json();
        if (!r.ok) {
            throw new Error(d.error || '加载趋势数据失败');
        }
        const data = d.data || [];
        const canvas = document.getElementById('trend-chart');
        const empty = document.getElementById('trend-chart-empty');

        if (data.length === 0) {
            if (trendChart) {
                trendChart.destroy();
                trendChart = null;
            }
            canvas.style.display = 'none';
            empty.style.display = 'block';
            return;
        }

        canvas.style.display = 'block';
        empty.style.display = 'none';

        const labels = data.map(p => p.period);
        const values = data.map(p => p.value);
        const isSinglePoint = data.length === 1;

        if (trendChart) trendChart.destroy();
        const ctx = canvas.getContext('2d');
        trendChart = new Chart(ctx, {
            type: isSinglePoint ? 'bar' : 'line',
            data: {
                labels,
                datasets: [{
                    label: metricLabels[metric],
                    data: values,
                    borderColor: '#818cf8',
                    backgroundColor: isSinglePoint ? 'rgba(129,140,248,0.7)' : 'rgba(129,140,248,0.15)',
                    fill: !isSinglePoint, tension: 0.3, pointRadius: 4,
                    borderWidth: isSinglePoint ? 1 : 2,
                }]
            },
            options: {
                responsive: true, maintainAspectRatio: false,
                plugins: { legend: { labels: { color: '#94a3b8' } } },
                scales: {
                    x: { ticks: { color: '#64748b', maxRotation: 45 }, grid: { color: '#1e293b' } },
                    y: { ticks: { color: '#64748b' }, grid: { color: '#1e293b' } },
                }
            }
        });
    } catch(e) { console.error(e); }

    // Agent comparison chart
    try {
        const r = await fetch('/api/v1/aggregate/agent-comparison');
        const d = await r.json();
        const comps = (d.comparisons || []).slice(0, 10);
        if (comps.length > 0) {
            const labels = comps.map(c => c.tool_model);
            const aiData = comps.map(c => c.ai_additions || 0);

            if (agentComparisonChart) agentComparisonChart.destroy();
            const ctx = document.getElementById('agent-comparison-chart').getContext('2d');
            agentComparisonChart = new Chart(ctx, {
                type: 'bar',
                data: {
                    labels,
                    datasets: [{
                        label: 'AI 代码行数',
                        data: aiData,
                        backgroundColor: 'rgba(129,140,248,0.7)',
                        borderColor: '#818cf8',
                        borderWidth: 1,
                    }]
                },
                options: {
                    responsive: true, maintainAspectRatio: false, indexAxis: 'y',
                    plugins: { legend: { labels: { color: '#94a3b8' } } },
                    scales: {
                        x: { ticks: { color: '#64748b' }, grid: { color: '#1e293b' } },
                        y: { ticks: { color: '#94a3b8' }, grid: { color: '#1e293b' } },
                    }
                }
            });
        }
    } catch(e) { console.error(e); }
}

// --- Organizations ---
async function loadOrgs() {
    setTableLoading('org-table', 5);
    try {
        const d = await fetchPaginatedJson('organizations', '/api/v1/aggregate/organizations', '加载组织数据失败');
        document.getElementById('org-table').innerHTML = pageItems(d, 'organizations').map(o => {
            return `<tr>
                <td><strong>${escapeHtml(o.organization)}</strong><br><span style="color:var(--text-muted);font-size:0.75rem">${escapeHtml(o.org_slug || '')}</span></td>
                <td>${fmt(o.total_commits)}</td>
                <td>${fmt(o.w_ai)}</td>
                <td>${fmt(o.w_human)}</td>
                <td>${pctBar(o.pct_ai || 0)} <span style="font-size:0.8rem">${(o.pct_ai || 0).toFixed(1)}%</span></td>
            </tr>`;
        }).join('') || '<tr><td colspan="5" style="color:var(--text-muted)">暂无组织数据</td></tr>';
        renderPaginationControls('organizations');
    } catch(e) {
        console.error(e);
        document.getElementById('org-table').innerHTML =
            '<tr><td colspan="5" style="color:var(--danger)">加载组织数据失败</td></tr>';
        renderPaginationControls('organizations');
    }
}

// --- Developers ---
async function loadDevs() {
    setTableLoading('dev-table', 8);
    try {
        const d = await fetchPaginatedJson('developers', '/api/v1/aggregate/developers', '加载开发者数据失败');
        const developers = pageItems(d, 'developers');
        developerGitInfo = new Map();
        if (developers.length === 0) {
            document.getElementById('dev-table').innerHTML =
                '<tr><td colspan="8" style="color:var(--text-muted)">暂无开发者数据</td></tr>';
            renderPaginationControls('developers');
            return;
        }

        document.getElementById('dev-table').innerHTML = developers.map(dev => {
            const total = dev.total_added_lines || 0;
            const ai = dev.ai_added_lines || 0;
            const devId = dev.id || dev.email || '';
            const emailDisplay = escapeHtml(dev.email || '—');
            const nameDisplay = escapeHtml(dev.name || '');
            const departmentDisplay = escapeHtml(dev.department || '未设置');
            const actionDevId = jsString(devId);
            const label = dev.name && dev.name !== dev.email
                ? `<strong>${nameDisplay}</strong><br><span style="color:var(--text-muted);font-size:0.75rem">${emailDisplay}</span>`
                : `<strong>${emailDisplay}</strong>`;
            developerGitInfo.set(devId, {
                name: dev.name || '',
                email: dev.email || '',
                department: dev.department || '',
                gitIdentities: dev.git_identities || []
            });
            return `<tr>
                <td>${label}</td>
                <td>${departmentDisplay}</td>
                <td>${fmt(dev.total_commits)}</td>
                <td>${fmt(total)}</td>
                <td>${fmt(ai)}</td>
                <td>${fmt(dev.human_added_lines)}</td>
                <td>${pctBar(dev.pct_ai || 0)} <span style="font-size:0.8rem">${(dev.pct_ai || 0).toFixed(1)}%</span></td>
                <td><button class="btn btn-sm" onclick="showDeveloperGitInfo(${actionDevId})">Git 信息</button></td>
            </tr>`;
        }).join('');
        renderPaginationControls('developers');
    } catch(e) {
        console.error(e);
        document.getElementById('dev-table').innerHTML =
            '<tr><td colspan="8" style="color:var(--danger)">加载开发者数据失败</td></tr>';
        renderPaginationControls('developers');
    }
}

function showDeveloperGitInfo(devId) {
    const info = developerGitInfo.get(devId);
    if (!info) {
        showToast('未找到开发者 Git 信息', 'error');
        return;
    }

    const identities = info.gitIdentities || [];
    const platformName = escapeHtml(info.name || '—');
    const platformEmail = escapeHtml(info.email || '—');
    const department = escapeHtml(info.department || '未设置');
    const gitList = identities.length > 0
        ? identities.map(identity => {
            const gitName = escapeHtml(identity.name || '—');
            const gitEmail = escapeHtml(identity.email || '—');
            return `<div class="git-identity-item">
                <div class="git-identity-name">${gitName}</div>
                <div class="git-identity-email">${gitEmail}</div>
            </div>`;
        }).join('')
        : '<div class="empty-state"><div class="empty-icon">ℹ️</div><p>暂无 Git 用户名和邮箱信息</p></div>';

    document.getElementById('modal-container').innerHTML = `
    <div class="modal-overlay" onclick="if(event.target===this)closeModal()">
        <div class="modal">
            <div class="modal-title">Git 信息</div>
            <div class="detail-list">
                <div class="detail-row"><span class="detail-label">平台注册用户名</span><span class="detail-value">${platformName}</span></div>
                <div class="detail-row"><span class="detail-label">平台注册邮箱</span><span class="detail-value">${platformEmail}</span></div>
                <div class="detail-row"><span class="detail-label">部门</span><span class="detail-value">${department}</span></div>
            </div>
            <div class="form-label">Git 用户名和邮箱</div>
            <div class="git-identity-list">${gitList}</div>
            <div class="form-actions">
                <button class="btn" onclick="closeModal()">关闭</button>
            </div>
        </div>
    </div>`;
}

// --- Projects ---
async function loadProjects() {
    setTableLoading('proj-table', 6);
    try {
        const d = await fetchPaginatedJson('projects', '/api/v1/aggregate/projects', '加载项目数据失败');
        document.getElementById('proj-table').innerHTML = pageItems(d, 'projects').map(p => {
            const displayName = escapeHtml(p.project_name || (p.repo_url ? p.repo_url.split('/').pop() : '—'));
            const displayUrl = escapeHtml(p.repo_url || p.remote_url_hash || '');
            const branch = escapeHtml(p.branch || '—');
            return `<tr>
                <td title="${displayUrl}"><strong>${displayName}</strong></td>
                <td>${branch}</td>
                <td>${fmt(p.total_commits)}</td>
                <td>${fmt(p.total_ai)}</td>
                <td>${fmt(p.total_human)}</td>
                <td>${pctBar(p.pct_ai || 0)} <span style="font-size:0.8rem">${(p.pct_ai || 0).toFixed(1)}%</span></td>
            </tr>`;
        }).join('') || '<tr><td colspan="6" style="color:var(--text-muted)">暂无项目数据</td></tr>';
        renderPaginationControls('projects');
    } catch(e) {
        console.error(e);
        document.getElementById('proj-table').innerHTML =
            '<tr><td colspan="6" style="color:var(--danger)">加载项目数据失败</td></tr>';
        renderPaginationControls('projects');
    }
}

// --- Tools ---
async function loadTools() {
    setTableLoading('tools-table', 5);
    try {
        const d = await fetchPaginatedJson('tools', '/api/v1/aggregate/tools', '加载工具数据失败');
        const tools = pageItems(d, 'tools');
        if (tools.length === 0) {
            document.getElementById('tools-table').innerHTML =
                '<tr><td colspan="5" style="color:var(--text-muted)">暂无工具使用数据，数据将在报告上传或指标事件后显示</td></tr>';
            renderPaginationControls('tools');
            return;
        }
        document.getElementById('tools-table').innerHTML = tools.map(t => {
            const ai = t.ai_additions || 0;
            const mixed = t.mixed_additions || 0;
            const accepted = t.ai_accepted || 0;
            const total = t.total_ai_additions || ai;
            const source = t.source === 'report'
                ? '<span class="badge human" style="margin-left:0.5rem">报告</span>'
                : t.source === 'report+metrics'
                    ? '<span class="badge human" style="margin-left:0.5rem">报告+指标</span>'
                    : '<span class="badge ai" style="margin-left:0.5rem">指标</span>';
            return `<tr>
                <td><strong>${escapeHtml(t.tool_model)}</strong>${source}</td>
                <td>${fmt(ai)}</td>
                <td>${fmt(mixed)}</td>
                <td>${fmt(accepted)}</td>
                <td>${fmt(total)}</td>
            </tr>`;
        }).join('');
        renderPaginationControls('tools');
    } catch(e) {
        console.error(e);
        document.getElementById('tools-table').innerHTML =
            '<tr><td colspan="5" style="color:var(--danger)">加载工具数据失败</td></tr>';
        renderPaginationControls('tools');
    }
}

// --- Users Management ---
async function loadUsers() {
    setTableLoading('users-table', 5);
    try {
        const d = await fetchPaginatedJson('users', '/api/admin/users/list', '加载用户列表失败');
        const users = pageItems(d, 'users');
        if (users.length === 0) {
            document.getElementById('users-table').innerHTML =
                '<tr><td colspan="5"><div class="empty-state"><div class="empty-icon">👤</div><p>暂无用户，点击上方按钮创建</p></div></td></tr>';
            renderPaginationControls('users');
            return;
        }
        document.getElementById('users-table').innerHTML = users.map(u => {
            const apiKeys = u.api_keys || [];
            const keyCount = apiKeys.length;
            const keyBadges = apiKeys.slice(0, 3).map(k =>
                `<span class="badge ai" style="margin-right:0.25rem">${escapeHtml(k.key_prefix)}...</span>`
            ).join('');
            const moreKeys = keyCount > 3 ? `<span style="color:var(--text-muted);font-size:0.75rem">+${keyCount-3}</span>` : '';
            const created = u.created_at ? new Date(u.created_at).toLocaleDateString('zh-CN') : '—';
            const displayName = escapeHtml(u.name || '—');
            const displayEmail = escapeHtml(u.email || '');
            const actionName = jsString(u.name || u.email || '');
            return `<tr>
                <td><strong>${displayName}</strong></td>
                <td>${displayEmail}</td>
                <td>${keyCount > 0 ? keyBadges + moreKeys : '<span style="color:var(--text-muted)">无密钥</span>'}</td>
                <td>${created}</td>
                <td>
                    <button class="btn btn-sm" onclick="showCreateApiKeyForUser('${u.id}', ${actionName})">🔑 创建密钥</button>
                    <button class="btn btn-sm btn-danger" onclick="deleteUser('${u.id}', ${actionName})">删除</button>
                </td>
            </tr>`;
        }).join('');
        renderPaginationControls('users');
    } catch(e) {
        console.error(e);
        document.getElementById('users-table').innerHTML =
            '<tr><td colspan="5" style="color:var(--danger)">加载用户列表失败</td></tr>';
        renderPaginationControls('users');
    }
}

function showCreateUserModal() {
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
                <label class="form-label">组织</label>
                <select id="create-user-org" class="form-input" disabled>
                    <option value="">加载组织中...</option>
                </select>
            </div>
            <div class="form-group">
                <label class="form-label">部门</label>
                <select id="create-user-dept" class="form-input" disabled>
                    <option value="">请先选择组织</option>
                </select>
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
    document.getElementById('create-user-org').addEventListener('change', event => {
        loadCreateUserDepartments(event.currentTarget.value);
    });
    populateCreateUserOrganizations();
}

async function populateCreateUserOrganizations() {
    const orgSelect = document.getElementById('create-user-org');
    const deptSelect = document.getElementById('create-user-dept');
    try {
        const orgs = await loadAdminOrganizations();
        if (orgs.length === 0) {
            orgSelect.innerHTML = '<option value="">暂无可选组织</option>';
            deptSelect.innerHTML = '<option value="">暂无可选部门</option>';
            showToast('请先创建组织', 'error');
            return;
        }

        orgSelect.innerHTML = '<option value="">请选择组织</option>' + orgs.map(org => {
            const label = `${escapeHtml(org.name || org.slug)}${org.slug ? ' (' + escapeHtml(org.slug) + ')' : ''}`;
            return `<option value="${org.id}">${label}</option>`;
        }).join('');
        orgSelect.disabled = false;

        if (orgs.length === 1) {
            orgSelect.value = orgs[0].id;
            await loadCreateUserDepartments(orgs[0].id);
        }
    } catch(e) {
        orgSelect.innerHTML = '<option value="">组织加载失败</option>';
        deptSelect.innerHTML = '<option value="">请先选择组织</option>';
        showToast(e.message || '加载组织列表失败', 'error');
    }
}

async function loadCreateUserDepartments(orgId) {
    const deptSelect = document.getElementById('create-user-dept');
    deptSelect.disabled = true;
    if (!orgId) {
        deptSelect.innerHTML = '<option value="">请先选择组织</option>';
        return;
    }

    deptSelect.innerHTML = '<option value="">加载部门中...</option>';
    try {
        const departments = await fetchAllPaginated(
            `/api/admin/departments?org_id=${encodeURIComponent(orgId)}`,
            'departments'
        );
        if (departments.length === 0) {
            deptSelect.innerHTML = '<option value="">该组织暂无部门</option>';
            return;
        }

        deptSelect.innerHTML = '<option value="">请选择部门</option>' + departments.map(dept => {
            const label = escapeHtml(dept.name || dept.slug || '未命名部门');
            return `<option value="${dept.id}">${label}</option>`;
        }).join('');
        deptSelect.disabled = false;

        if (departments.length === 1) {
            deptSelect.value = departments[0].id;
        }
    } catch(e) {
        deptSelect.innerHTML = '<option value="">部门加载失败</option>';
        showToast(e.message || '加载部门列表失败', 'error');
    }
}

async function createUser() {
    const name = document.getElementById('create-user-name').value.trim();
    const emailVal = document.getElementById('create-user-email').value.trim();
    const orgId = document.getElementById('create-user-org').value;
    const departmentId = document.getElementById('create-user-dept').value;
    const genNonce = document.getElementById('create-user-nonce').checked;

    if (!name || !emailVal || !orgId || !departmentId) {
        showToast('请填写用户名、邮箱、组织和部门', 'error');
        return;
    }

    try {
        const r = await fetch('/api/admin/users', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                name,
                email: emailVal,
                org_id: orgId,
                department_id: departmentId,
                generate_nonce: genNonce
            })
        });
        const d = await r.json();
        if (r.ok) {
            let msg = `用户 ${name} 创建成功！`;
            if (d.install_nonce) msg += `\\n安装令牌: ${d.install_nonce}`;
            showToast(msg, 'success');
            closeModal();
            resetTablePage('users');
            loadUsers();
        } else {
            showToast(`创建失败: ${d.error || '未知错误'}`, 'error');
        }
    } catch(e) {
        showToast('创建用户时发生错误', 'error');
    }
}

async function deleteUser(userId, userName) {
    if (!confirm(`确定要删除用户「${userName}」吗？此操作不可撤销。`)) return;
    try {
        const r = await fetch(`/api/admin/users/${userId}`, { method: 'DELETE' });
        if (r.ok) {
            showToast(`用户「${userName}」已删除`, 'success');
            resetTablePage('users');
            loadUsers();
        } else {
            const d = await r.json();
            showToast(`删除失败: ${d.error || '未知错误'}`, 'error');
        }
    } catch(e) {
        showToast('删除用户时发生错误', 'error');
    }
}

// --- Departments Management ---
let adminOrganizationsCache = null;

async function loadAdminOrganizations() {
    if (adminOrganizationsCache) return adminOrganizationsCache;
    adminOrganizationsCache = await fetchAllPaginated(
        '/api/admin/organizations/list?include_personal=false',
        'organizations'
    );
    return adminOrganizationsCache;
}

async function loadDepartments() {
    setTableLoading('departments-table', 5);
    try {
        const d = await fetchPaginatedJson('departments', '/api/v1/aggregate/departments', '加载部门列表失败');
        const departments = pageItems(d, 'departments');
        if (departments.length === 0) {
            document.getElementById('departments-table').innerHTML =
                '<tr><td colspan="5"><div class="empty-state"><div class="empty-icon">🏷️</div><p>暂无部门数据</p></div></td></tr>';
            renderPaginationControls('departments');
            return;
        }

        document.getElementById('departments-table').innerHTML = departments.map(dept => {
            const departmentName = escapeHtml(dept.department || '—');
            const orgName = escapeHtml(dept.organization || '—');
            const total = dept.w_total || 0;
            const ai = dept.w_ai || 0;
            const pct = total > 0 ? (ai / total) * 100 : 0;
            return `<tr>
                <td><strong>${orgName}</strong></td>
                <td><strong>${departmentName}</strong></td>
                <td>${fmt(dept.total_commits || 0)}</td>
                <td>${pctBar(pct)} <span style="font-size:0.8rem">${pct.toFixed(1)}%</span></td>
                <td>${fmt(total)}</td>
            </tr>`;
        }).join('');
        renderPaginationControls('departments');
    } catch(e) {
        console.error(e);
        document.getElementById('departments-table').innerHTML =
            '<tr><td colspan="5" style="color:var(--danger)">加载部门列表失败</td></tr>';
        renderPaginationControls('departments');
    }
}

async function showCreateDepartmentModal() {
    let orgs = [];
    try {
        orgs = await loadAdminOrganizations();
    } catch(e) {
        showToast(e.message || '加载组织列表失败', 'error');
        return;
    }
    if (orgs.length === 0) {
        showToast('请先创建组织', 'error');
        return;
    }
    const orgOptions = orgs.map(org => {
        const label = `${escapeHtml(org.name || org.slug)}${org.slug ? ' (' + escapeHtml(org.slug) + ')' : ''}`;
        return `<option value="${org.id}">${label}</option>`;
    }).join('');

    document.getElementById('modal-container').innerHTML = `
    <div class="modal-overlay" onclick="if(event.target===this)closeModal()">
        <div class="modal">
            <div class="modal-title">新增部门</div>
            <div class="form-group">
                <label class="form-label">所属组织</label>
                <select id="create-dept-org" class="form-input">${orgOptions}</select>
            </div>
            <div class="form-group">
                <label class="form-label">部门名称</label>
                <input type="text" id="create-dept-name" class="form-input" placeholder="例如：技术中心" />
            </div>
            <div class="form-actions">
                <button class="btn" onclick="closeModal()">取消</button>
                <button class="btn btn-primary" onclick="createDepartment()">新增</button>
            </div>
        </div>
    </div>`;
}

async function createDepartment() {
    const org_id = document.getElementById('create-dept-org').value;
    const name = document.getElementById('create-dept-name').value.trim();

    if (!org_id || !name) {
        showToast('请填写组织和部门名称', 'error');
        return;
    }

    try {
        const r = await fetch('/api/admin/departments', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ org_id, name })
        });
        const d = await r.json();
        if (r.ok) {
            showToast(`部门「${name}」已新增`, 'success');
            closeModal();
            resetTablePage('departments');
            loadDepartments();
        } else {
            showToast(`新增失败: ${d.error || '未知错误'}`, 'error');
        }
    } catch(e) {
        showToast('新增部门时发生错误', 'error');
    }
}

// --- API Key Management ---
async function loadApiKeys() {
    setTableLoading('apikeys-table', 7);
    try {
        const d = await fetchPaginatedJson('apikeys', '/api/admin/api-keys', '加载密钥列表失败');
        const keys = pageItems(d, 'api_keys');
        if (keys.length === 0) {
            document.getElementById('apikeys-table').innerHTML =
                '<tr><td colspan="7"><div class="empty-state"><div class="empty-icon">🔑</div><p>暂无 API 密钥，点击上方按钮创建</p></div></td></tr>';
            renderPaginationControls('apikeys');
            return;
        }
        document.getElementById('apikeys-table').innerHTML = keys.map(k => {
            const created = k.created_at ? new Date(k.created_at).toLocaleDateString('zh-CN') : '—';
            const expires = k.expires_at ? new Date(k.expires_at).toLocaleDateString('zh-CN') : '永不过期';
            const lastUsed = k.last_used_at ? new Date(k.last_used_at).toLocaleString('zh-CN') : '从未使用';
            const scopes = (k.scopes || []).map(s => `<span class="badge role" style="margin:0.1rem">${escapeHtml(s)}</span>`).join(' ');
            const keyName = escapeHtml(k.name || '未命名');
            const actionName = jsString(k.name || k.key_prefix || '');
            return `<tr>
                <td><strong>${keyName}</strong></td>
                <td><code style="color:var(--accent);font-size:0.8rem">${escapeHtml(k.key_prefix)}...</code></td>
                <td>${scopes}</td>
                <td>${created}</td>
                <td>${expires}</td>
                <td style="font-size:0.8rem">${lastUsed}</td>
                <td><button class="btn btn-sm btn-danger" onclick="revokeApiKey('${k.id}', ${actionName})">撤销</button></td>
            </tr>`;
        }).join('');
        renderPaginationControls('apikeys');
    } catch(e) {
        console.error(e);
        document.getElementById('apikeys-table').innerHTML =
            '<tr><td colspan="7" style="color:var(--danger)">加载密钥列表失败</td></tr>';
        renderPaginationControls('apikeys');
    }
}

function showCreateApiKeyModal() {
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
}

function showCreateApiKeyForUser(userId, userName) {
    const safeUserName = escapeHtml(userName);
    document.getElementById('modal-container').innerHTML = `
    <div class="modal-overlay" onclick="if(event.target===this)closeModal()">
        <div class="modal">
            <div class="modal-title">为用户「${safeUserName}」创建 API 密钥</div>
            <input type="hidden" id="create-key-user-id" value="${userId}" />
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
}

async function createApiKey() {
    const name = document.getElementById('create-key-name').value.trim();
    if (!name) { showToast('请填写密钥名称', 'error'); return; }

    const scopes = Array.from(document.querySelectorAll('.key-scope:checked')).map(cb => cb.value);
    if (scopes.length === 0) { showToast('请至少选择一个权限范围', 'error'); return; }

    try {
        const r = await fetch('/api/admin/api-keys', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name, scopes })
        });
        const d = await r.json();
        if (r.ok) {
            document.getElementById('new-key-result').style.display = 'block';
            document.getElementById('new-key-value').innerHTML =
                `<button class="copy-btn" onclick="copyKey()">复制</button>${d.key}`;
            document.getElementById('create-key-btn').style.display = 'none';
            showToast('API 密钥创建成功', 'success');
            resetTablePage('apikeys');
            if (currentSection === 'apikeys') loadApiKeys();
        } else {
            showToast(`创建失败: ${d.error || '未知错误'}`, 'error');
        }
    } catch(e) {
        showToast('创建密钥时发生错误', 'error');
    }
}

async function createApiKeyForUser() {
    const name = document.getElementById('create-key-name').value.trim();
    const userId = document.getElementById('create-key-user-id').value;
    if (!name) { showToast('请填写密钥名称', 'error'); return; }

    const scopes = Array.from(document.querySelectorAll('.key-scope:checked')).map(cb => cb.value);
    if (scopes.length === 0) { showToast('请至少选择一个权限范围', 'error'); return; }

    try {
        const r = await fetch('/api/admin/api-keys', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name, scopes, user_id: userId })
        });
        const d = await r.json();
        if (r.ok) {
            document.getElementById('new-key-result').style.display = 'block';
            document.getElementById('new-key-value').innerHTML =
                `<button class="copy-btn" onclick="copyKey()">复制</button>${d.key}`;
            document.getElementById('create-key-btn').style.display = 'none';
            showToast('API 密钥创建成功', 'success');
            resetTablePage('users');
            resetTablePage('apikeys');
            if (currentSection === 'users') loadUsers();
            if (currentSection === 'apikeys') loadApiKeys();
        } else {
            showToast(`创建失败: ${d.error || '未知错误'}`, 'error');
        }
    } catch(e) {
        showToast('创建密钥时发生错误', 'error');
    }
}

function copyKey() {
    const keyEl = document.getElementById('new-key-value');
    const text = keyEl.textContent.replace('复制', '').trim();
    navigator.clipboard.writeText(text).then(() => showToast('已复制到剪贴板', 'success'));
}

async function revokeApiKey(keyId, keyName) {
    if (!confirm(`确定要撤销密钥「${keyName}」吗？撤销后此密钥将立即失效。`)) return;
    try {
        const r = await fetch(`/api/admin/api-keys/${keyId}`, { method: 'DELETE' });
        if (r.ok) {
            showToast(`密钥「${keyName}」已撤销`, 'success');
            resetTablePage('apikeys');
            loadApiKeys();
        } else {
            showToast('撤销失败', 'error');
        }
    } catch(e) {
        showToast('撤销密钥时发生错误', 'error');
    }
}

function closeModal() {
    document.getElementById('modal-container').innerHTML = '';
}

// --- Init ---
loadOverview();
if (!isAdmin) loadClientStatus();
updateRefreshTime();
startAutoRefresh();
