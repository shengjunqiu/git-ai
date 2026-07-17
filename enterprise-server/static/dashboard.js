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
function escapeAttribute(value) {
    return String(value ?? '')
        .replace(/&/g, '&amp;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#39;')
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
const DASHBOARD_DEFAULT_SECTION = 'overview';
const DASHBOARD_SECTIONS = [
    'overview',
    'trends',
    'organizations',
    'departments',
    'developers',
    'projects',
    'tools',
    'users',
    'apikeys',
    'releases',
    'files',
    'help',
];
const ADMIN_ONLY_DASHBOARD_SECTIONS = ['organizations', 'users', 'apikeys', 'releases', 'files'];
let currentSection = 'overview';
let departmentTreeRows = [];
let activeDepartmentParentId = null;
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
    const orgNavItem = document.getElementById('org-nav-item');
    if (orgNavItem) orgNavItem.style.display = 'none';
    const orgSection = document.getElementById('section-organizations');
    if (orgSection) orgSection.style.display = 'none';
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
function canAccessDashboardSection(id) {
    return DASHBOARD_SECTIONS.includes(id)
        && (isAdmin || !ADMIN_ONLY_DASHBOARD_SECTIONS.includes(id));
}

function dashboardSectionFromLocation() {
    const requestedSection = new URL(window.location.href).searchParams.get('section');
    return canAccessDashboardSection(requestedSection)
        ? requestedSection
        : DASHBOARD_DEFAULT_SECTION;
}

function updateDashboardSectionUrl(id, replace = false) {
    const url = new URL(window.location.href);
    url.hash = '';
    if (id === DASHBOARD_DEFAULT_SECTION) {
        url.searchParams.delete('section');
    } else {
        url.searchParams.set('section', id);
    }
    const nextUrl = `${url.pathname}${url.search}${url.hash}`;
    window.history[replace ? 'replaceState' : 'pushState']({ section: id }, '', nextUrl);
}

function activateDashboardSection(id, { updateUrl = false, replaceUrl = false } = {}) {
    const nextSection = canAccessDashboardSection(id) ? id : DASHBOARD_DEFAULT_SECTION;
    currentSection = nextSection;
    document.querySelectorAll('.section').forEach(s => s.classList.remove('active'));
    document.querySelectorAll('.nav-item').forEach(n => n.classList.remove('active'));
    document.getElementById('section-' + nextSection).classList.add('active');
    const activeNavItem = Array.from(document.querySelectorAll('.nav-item'))
        .find(item => item.dataset.section === nextSection);
    if (activeNavItem) activeNavItem.classList.add('active');
    if (updateUrl) updateDashboardSectionUrl(nextSection, replaceUrl);
    loadSection(nextSection);
}

function showSection(event, id) {
    event.preventDefault();
    if (!canAccessDashboardSection(id)) return false;
    activateDashboardSection(id, { updateUrl: true });
    return false;
}

window.addEventListener('popstate', () => {
    activateDashboardSection(dashboardSectionFromLocation());
});

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
        case 'releases': loadReleaseManagement(); break;
        case 'files': loadManagedFiles(); break;
        case 'help': break;
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

async function copyHelpCommand(button) {
    const code = button.parentElement.querySelector('code');
    if (!code) return;

    try {
        await copyHelpText(code.textContent.trim());
        const originalLabel = button.textContent;
        button.textContent = '已复制';
        setTimeout(() => { button.textContent = originalLabel; }, 1600);
        showToast('命令已复制', 'success');
    } catch (error) {
        showToast('复制失败，请手动选择命令', 'error');
    }
}

async function copyHelpText(text) {
    if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(text);
        return;
    }

    const textarea = document.createElement('textarea');
    textarea.value = text;
    textarea.setAttribute('readonly', '');
    textarea.style.position = 'fixed';
    textarea.style.opacity = '0';
    document.body.appendChild(textarea);
    textarea.select();
    const copied = document.execCommand('copy');
    textarea.remove();
    if (!copied) throw new Error('Copy command was rejected');
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
let developerSortBy = 'ai_lines';
let developerSortOrder = 'desc';

function changeDeveloperSorting() {
    developerSortBy = document.getElementById('developer-sort-by')?.value || 'ai_lines';
    developerSortOrder = document.getElementById('developer-sort-order')?.value || 'desc';
    resetTablePage('developers');
    loadDevs();
}

async function loadDevs() {
    setTableLoading('dev-table', 8);
    try {
        const developerUrl = `/api/v1/aggregate/developers?sort_by=${encodeURIComponent(developerSortBy)}&sort_order=${encodeURIComponent(developerSortOrder)}`;
        const d = await fetchPaginatedJson('developers', developerUrl, '加载开发者数据失败');
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
            const unassignedBadge = p.is_unassigned
                ? ' <span class="badge unassigned" title="这些提交来自未配置 Git remote 的仓库，暂时无法确定所属项目">未关联</span>'
                : '';
            return `<tr>
                <td title="${displayUrl}"><strong>${displayName}</strong>${unassignedBadge}</td>
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
const selectedGitTrackingUserIds = new Set();
let visibleGitTrackingUserIds = [];

async function loadUsers() {
    selectedGitTrackingUserIds.clear();
    visibleGitTrackingUserIds = [];
    updateGitTrackingBulkSelection();
    setTableLoading('users-table', 7);
    try {
        const d = await fetchPaginatedJson('users', '/api/admin/users/list', '加载用户列表失败');
        const users = pageItems(d, 'users');
        if (users.length === 0) {
            document.getElementById('users-table').innerHTML =
                '<tr><td colspan="7"><div class="empty-state"><div class="empty-icon">👤</div><p>暂无用户，点击上方按钮创建</p></div></td></tr>';
            renderPaginationControls('users');
            return;
        }
        visibleGitTrackingUserIds = users
            .filter(user => user.git_tracking_upload_enabled !== true)
            .map(user => user.id);
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
            const uploadEnabled = u.git_tracking_upload_enabled === true;
            const uploadStatus = uploadEnabled
                ? '<span class="badge active">已授权</span>'
                : '<span class="badge revoked">未授权</span>';
            return `<tr>
                <td class="selection-column"><input class="git-tracking-user-checkbox" type="checkbox" value="${u.id}" aria-label="选择${displayName}" onchange="toggleGitTrackingUser('${u.id}', this.checked)" ${uploadEnabled ? 'disabled' : ''} /></td>
                <td><strong>${displayName}</strong></td>
                <td>${displayEmail}</td>
                <td>${uploadStatus}</td>
                <td>${keyCount > 0 ? keyBadges + moreKeys : '<span style="color:var(--text-muted)">无密钥</span>'}</td>
                <td>${created}</td>
                <td>
                    <button class="btn btn-sm ${uploadEnabled ? 'btn-danger' : 'btn-primary'}" onclick="setGitTrackingUploadAuthorization('${u.id}', ${actionName}, ${!uploadEnabled}, this)">${uploadEnabled ? '撤销上传' : '授权上传'}</button>
                    <button class="btn btn-sm" onclick="showCreateApiKeyForUser('${u.id}', ${actionName})">🔑 创建密钥</button>
                    <button class="btn btn-sm btn-danger" onclick="deleteUser('${u.id}', ${actionName})">删除</button>
                </td>
            </tr>`;
        }).join('');
        updateGitTrackingBulkSelection();
        renderPaginationControls('users');
    } catch(e) {
        console.error(e);
        document.getElementById('users-table').innerHTML =
            '<tr><td colspan="7" style="color:var(--danger)">加载用户列表失败</td></tr>';
        renderPaginationControls('users');
    }
}

function toggleGitTrackingUser(userId, selected) {
    if (selected) selectedGitTrackingUserIds.add(userId);
    else selectedGitTrackingUserIds.delete(userId);
    updateGitTrackingBulkSelection();
}

function toggleAllGitTrackingUsers(selected) {
    visibleGitTrackingUserIds.forEach(userId => {
        if (selected) selectedGitTrackingUserIds.add(userId);
        else selectedGitTrackingUserIds.delete(userId);
    });
    document.querySelectorAll('.git-tracking-user-checkbox:not(:disabled)').forEach(checkbox => {
        checkbox.checked = selected;
    });
    updateGitTrackingBulkSelection();
}

function updateGitTrackingBulkSelection() {
    const count = selectedGitTrackingUserIds.size;
    const label = document.getElementById('users-bulk-selection');
    const button = document.getElementById('users-bulk-authorize');
    const selectAll = document.getElementById('users-select-all');
    if (label) label.textContent = count > 0 ? `已选择 ${count} 位未授权用户` : '选择未授权用户后可批量授权';
    if (button) button.disabled = count === 0;
    if (selectAll) {
        const selectedVisibleCount = visibleGitTrackingUserIds.filter(id => selectedGitTrackingUserIds.has(id)).length;
        selectAll.disabled = visibleGitTrackingUserIds.length === 0;
        selectAll.checked = visibleGitTrackingUserIds.length > 0 && selectedVisibleCount === visibleGitTrackingUserIds.length;
        selectAll.indeterminate = selectedVisibleCount > 0 && selectedVisibleCount < visibleGitTrackingUserIds.length;
    }
}

async function bulkAuthorizeGitTrackingUpload(button) {
    const userIds = Array.from(selectedGitTrackingUserIds);
    if (userIds.length === 0) return;
    if (!confirm(`确定为选中的 ${userIds.length} 位用户授权 Git 追踪上传吗？\n授权后，这些用户可以向平台上传 Git 追踪信息。`)) return;

    if (button) button.disabled = true;
    try {
        const response = await fetch('/api/admin/users/git-tracking-upload/authorize', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ user_ids: userIds })
        });
        const result = await response.json();
        if (!response.ok) {
            showToast(`批量授权失败: ${result.error || '未知错误'}`, 'error');
            return;
        }
        showToast(`已为 ${result.authorized_count || userIds.length} 位用户授权 Git 追踪上传`, 'success');
        await loadUsers();
    } catch(e) {
        showToast('批量授权时发生错误', 'error');
    } finally {
        if (button && button.isConnected) button.disabled = selectedGitTrackingUserIds.size === 0;
    }
}

async function setGitTrackingUploadAuthorization(userId, userName, authorized, button) {
    const actionLabel = authorized ? '授权' : '撤销授权';
    const consequence = authorized
        ? '授权后，该开发者可以向平台上传 Git 追踪信息。'
        : '撤销后，该开发者新的 Git 追踪信息上传会被立即拒绝。';
    if (!confirm(`确定要${actionLabel}开发者「${userName}」吗？\n${consequence}`)) return;

    if (button) button.disabled = true;
    try {
        const r = await fetch(`/api/admin/users/${userId}/git-tracking-upload`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ authorized })
        });
        const d = await r.json();
        if (!r.ok) {
            showToast(`${actionLabel}失败: ${d.error || '未知错误'}`, 'error');
            return;
        }

        showToast(`已${actionLabel}开发者「${userName}」的 Git 追踪上传权限`, 'success');
        await loadUsers();
    } catch(e) {
        showToast(`${actionLabel}时发生错误`, 'error');
    } finally {
        if (button && button.isConnected) button.disabled = false;
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
const DEPARTMENT_CODE_PREFIX_ORDER = ['F', 'A', 'C', 'S'];

function departmentCodePrefixRank(code) {
    const prefix = String(code || '').trim().charAt(0).toUpperCase();
    const rank = DEPARTMENT_CODE_PREFIX_ORDER.indexOf(prefix);
    return rank === -1 ? DEPARTMENT_CODE_PREFIX_ORDER.length : rank;
}

async function loadAdminOrganizations() {
    if (adminOrganizationsCache) return adminOrganizationsCache;
    adminOrganizationsCache = await fetchAllPaginated(
        '/api/admin/organizations/list?include_personal=false',
        'organizations'
    );
    return adminOrganizationsCache;
}

async function loadDepartments() {
    setTableLoading('departments-table', 6);
    try {
        departmentTreeRows = await fetchAllPaginated(
            '/api/v1/aggregate/departments',
            'departments'
        );
        if (departmentTreeRows.length === 0) {
            renderDepartmentBreadcrumb();
            document.getElementById('departments-table').innerHTML =
                `<tr><td colspan="6"><div class="empty-state"><div class="empty-icon">🏷️</div><p>${isAdmin ? '暂无部门数据' : '当前账号尚未分配部门'}</p></div></td></tr>`;
            return;
        }

        if (activeDepartmentParentId && !departmentTreeRows.some(dept => dept.id === activeDepartmentParentId)) {
            activeDepartmentParentId = null;
        }
        renderDepartmentLevel();
    } catch(e) {
        console.error(e);
        document.getElementById('departments-table').innerHTML =
            '<tr><td colspan="6" style="color:var(--danger)">加载部门列表失败</td></tr>';
    }
}

function openDepartmentLevel(parentId) {
    if (!isAdmin) return;
    activeDepartmentParentId = parentId || null;
    renderDepartmentLevel();
}

function backDepartmentLevel() {
    if (!isAdmin) return;
    if (!activeDepartmentParentId) return;
    const current = departmentTreeRows.find(dept => dept.id === activeDepartmentParentId);
    activeDepartmentParentId = current?.parent_id || null;
    renderDepartmentLevel();
}

function renderDepartmentBreadcrumb() {
    const breadcrumb = document.getElementById('departments-breadcrumb');
    const backButton = document.getElementById('departments-back');
    if (!breadcrumb || !backButton) return;

    if (!isAdmin) {
        breadcrumb.innerHTML = '<strong>我的部门</strong>';
        backButton.style.display = 'none';
        return;
    }

    const byId = new Map(departmentTreeRows.map(dept => [dept.id, dept]));
    const trail = [];
    const visited = new Set();
    let current = activeDepartmentParentId ? byId.get(activeDepartmentParentId) : null;
    while (current && !visited.has(current.id)) {
        visited.add(current.id);
        trail.push(current);
        current = current.parent_id ? byId.get(current.parent_id) : null;
    }
    trail.reverse();

    const parts = [activeDepartmentParentId
        ? '<button class="btn btn-sm" onclick="openDepartmentLevel(null)">全部部门</button>'
        : '<strong>全部部门</strong>'];
    trail.forEach((dept, index) => {
        parts.push('<span style="color:var(--text-muted)">/</span>');
        if (index === trail.length - 1) {
            parts.push(`<strong>${escapeHtml(dept.department || '—')}</strong>`);
        } else {
            parts.push(`<button class="btn btn-sm" onclick="openDepartmentLevel('${dept.id}')">${escapeHtml(dept.department || '—')}</button>`);
        }
    });
    breadcrumb.innerHTML = parts.join(' ');

    backButton.style.display = activeDepartmentParentId ? '' : 'none';
}

function renderDepartmentLevel() {
    renderDepartmentBreadcrumb();
    const departments = (isAdmin
        ? departmentTreeRows.filter(dept => (dept.parent_id || null) === activeDepartmentParentId)
        : departmentTreeRows.slice())
        .sort((left, right) => {
            const prefixDifference = departmentCodePrefixRank(left.code) - departmentCodePrefixRank(right.code);
            if (prefixDifference !== 0) return prefixDifference;
            const codeDifference = String(left.code || '').localeCompare(
                String(right.code || ''),
                undefined,
                { numeric: true, sensitivity: 'base' }
            );
            if (codeDifference !== 0) return codeDifference;
            return String(left.id || '').localeCompare(String(right.id || ''));
        });

    if (departments.length === 0) {
        document.getElementById('departments-table').innerHTML =
            '<tr><td colspan="6"><div class="empty-state"><div class="empty-icon">🏷️</div><p>当前层级暂无下级部门</p></div></td></tr>';
        return;
    }

    document.getElementById('departments-table').innerHTML = departments.map(dept => {
            const departmentName = escapeHtml(dept.department || '—');
            const departmentCode = escapeHtml(dept.code || '—');
            const orgName = escapeHtml(dept.organization || '—');
            const nodeIcon = isAdmin && dept.has_children ? '›' : '•';
            const rowAction = isAdmin && dept.has_children
                ? ` onclick="openDepartmentLevel('${dept.id}')" style="cursor:pointer"`
                : '';
            const total = dept.w_total || 0;
            const pct = departmentAiPercentage(dept) * 100;
            return `<tr${rowAction}>
                <td><strong>${orgName}</strong></td>
                <td><span class="department-code">${departmentCode}</span></td>
                <td>
                    <div style="display:flex;align-items:center;gap:0.45rem">
                        <span style="color:var(--text-muted);width:0.9rem">${nodeIcon}</span>
                        <strong>${departmentName}</strong>
                    </div>
                </td>
                <td>${fmt(dept.total_commits || 0)}</td>
                <td>${pctBar(pct)} <span style="font-size:0.8rem">${pct.toFixed(1)}%</span></td>
                <td>${fmt(total)}</td>
            </tr>`;
    }).join('');
}

function departmentAiPercentage(department) {
    const total = Number(department.w_total) || 0;
    const ai = Number(department.w_ai) || 0;
    return total > 0 ? ai / total : 0;
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
                <select id="create-dept-org" class="form-input" onchange="loadCreateDepartmentParents()">${orgOptions}</select>
            </div>
            <div class="form-group">
                <label class="form-label">部门名称</label>
                <input type="text" id="create-dept-name" class="form-input" placeholder="例如：技术中心" />
            </div>
            <div class="form-group">
                <label class="form-label">部门编码（可选）</label>
                <input type="text" id="create-dept-code" class="form-input" placeholder="例如：F01289；留空自动生成" />
            </div>
            <div class="form-group">
                <label class="form-label">上级部门（可选）</label>
                <select id="create-dept-parent" class="form-input">
                    <option value="">无（根部门）</option>
                </select>
            </div>
            <div class="form-actions">
                <button class="btn" onclick="closeModal()">取消</button>
                <button class="btn btn-primary" onclick="createDepartment()">新增</button>
            </div>
        </div>
    </div>`;
    await loadCreateDepartmentParents();
}

async function loadCreateDepartmentParents() {
    const orgId = document.getElementById('create-dept-org')?.value;
    const parentSelect = document.getElementById('create-dept-parent');
    if (!orgId || !parentSelect) return;

    parentSelect.disabled = true;
    parentSelect.innerHTML = '<option value="">加载上级部门中...</option>';
    try {
        const departments = await fetchAllPaginated(
            `/api/admin/departments?org_id=${encodeURIComponent(orgId)}`,
            'departments'
        );
        parentSelect.innerHTML = '<option value="">无（根部门）</option>' + departments.map(dept => {
            const label = `${escapeHtml(dept.code || '—')} · ${escapeHtml(dept.name || '—')}`;
            return `<option value="${dept.id}">${label}</option>`;
        }).join('');
    } catch (e) {
        parentSelect.innerHTML = '<option value="">上级部门加载失败</option>';
        showToast(e.message || '加载上级部门失败', 'error');
    } finally {
        parentSelect.disabled = false;
    }
}

async function createDepartment() {
    const org_id = document.getElementById('create-dept-org').value;
    const name = document.getElementById('create-dept-name').value.trim();
    const code = document.getElementById('create-dept-code').value.trim() || null;
    const parent_id = document.getElementById('create-dept-parent').value || null;

    if (!org_id || !name) {
        showToast('请填写组织和部门名称', 'error');
        return;
    }

    try {
        const r = await fetch('/api/admin/departments', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ org_id, name, code, parent_id })
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

// --- CLI Release Management ---
const REQUIRED_RELEASE_FILES = [
    'git-ai-linux-x64',
    'git-ai-linux-arm64',
    'git-ai-windows-x64.exe',
    'git-ai-windows-arm64.exe',
    'git-ai-macos-x64',
    'git-ai-macos-arm64',
];

function formatBytes(value) {
    const bytes = Number(value || 0);
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

async function readResponseJson(response) {
    try {
        return await response.json();
    } catch (_) {
        return {};
    }
}

function renderSelectedReleaseFiles() {
    const input = document.getElementById('release-files');
    const target = document.getElementById('release-selected-files');
    if (!input || !target) return;
    const selected = new Map(Array.from(input.files || []).map(file => [file.name, file]));
    target.innerHTML = REQUIRED_RELEASE_FILES.map(filename => {
        const file = selected.get(filename);
        return file
            ? `<span class="selected-file-ok">✓ ${escapeHtml(filename)}（${formatBytes(file.size)}）</span>`
            : `<span class="selected-file-missing">缺少 ${escapeHtml(filename)}</span>`;
    }).join('<br>');
}

async function loadReleaseManagement() {
    const table = document.getElementById('release-table');
    if (!table) return;
    table.innerHTML = '<tr><td colspan="5" style="color:var(--text-muted)">加载中...</td></tr>';
    try {
        const [metadataResponse, assetsResponse] = await Promise.all([
            fetch('/worker/releases'),
            fetch('/api/admin/releases/assets'),
        ]);
        const metadata = await readResponseJson(metadataResponse);
        const assetData = await readResponseJson(assetsResponse);
        if (!metadataResponse.ok || !assetsResponse.ok) {
            throw new Error(metadata.error || assetData.error || '加载发布数据失败');
        }

        const channels = metadata.channels || {};
        const latest = channels.latest || null;
        const versionGroups = new Map();
        (assetData.assets || []).forEach(asset => {
            if (!versionGroups.has(asset.version)) versionGroups.set(asset.version, []);
            versionGroups.get(asset.version).push(asset);
        });
        Object.entries(channels).forEach(([channel, info]) => {
            if (channel === info.version && !versionGroups.has(info.version)) {
                versionGroups.set(info.version, []);
            }
        });

        const stats = document.getElementById('release-channel-stats');
        stats.innerHTML = `
            <div class="stat-card"><div class="stat-label">LATEST</div><div class="stat-value total">${escapeHtml(latest?.version || '未发布')}</div><div class="stat-detail">${latest ? '客户端自动更新目标' : '尚未设置 latest 渠道'}</div></div>
            <div class="stat-card"><div class="stat-label">版本数量</div><div class="stat-value total">${versionGroups.size}</div><div class="stat-detail">包含草稿和已发布版本</div></div>
            <div class="stat-card"><div class="stat-label">发布文件</div><div class="stat-value total">${(assetData.assets || []).length}</div><div class="stat-detail">跨平台二进制、脚本与校验文件</div></div>`;

        const versions = Array.from(versionGroups.keys()).sort((a, b) =>
            b.localeCompare(a, undefined, { numeric: true, sensitivity: 'base' }));
        table.innerHTML = versions.map(version => {
            const assets = versionGroups.get(version) || [];
            const versionChannel = channels[version];
            const checksum = versionChannel?.checksum || assets.find(asset => asset.filename === 'SHA256SUMS')?.sha256 || '';
            const isLatest = latest?.version === version;
            const isPublished = Boolean(versionChannel);
            const assetNames = assets
                .slice()
                .sort((a, b) => a.filename.localeCompare(b.filename))
                .map(asset => `${escapeHtml(asset.filename)} (${formatBytes(asset.size_bytes)})`)
                .join('<br>');
            const actions = [];
            if (isPublished && !isLatest) {
                actions.push(`<button class="btn btn-sm btn-primary" onclick="promoteCliRelease(${jsString(version)}, ${jsString(checksum)})">设为 latest</button>`);
            }
            if (isPublished) {
                actions.push(`<button class="btn btn-sm" onclick="copyPublishedUrl(${jsString(`/worker/releases/${version}/download/install.sh`)})">复制安装链接</button>`);
            }
            return `<tr>
                <td><strong>${escapeHtml(version)}</strong></td>
                <td>${isLatest ? '<span class="badge active">latest</span>' : isPublished ? '<span class="badge ai">已发布</span>' : '<span class="badge revoked">草稿</span>'}</td>
                <td><div class="asset-list">${assetNames || '暂无资产记录'}</div></td>
                <td><span class="checksum" title="${escapeHtml(checksum)}">${escapeHtml(checksum || '—')}</span></td>
                <td><div class="action-group">${actions.join('') || '—'}</div></td>
            </tr>`;
        }).join('') || '<tr><td colspan="5"><div class="empty-state"><div class="empty-icon">🚀</div><p>尚未上传 CLI 版本</p></div></td></tr>';
    } catch (error) {
        table.innerHTML = `<tr><td colspan="5" style="color:var(--danger)">${escapeHtml(error.message || '加载发布数据失败')}</td></tr>`;
    }
}

async function publishCliRelease(button) {
    const version = document.getElementById('release-version').value.trim();
    const input = document.getElementById('release-files');
    const status = document.getElementById('release-publish-status');
    const selectedFiles = Array.from(input.files || []);
    const selectedNames = new Set(selectedFiles.map(file => file.name));
    const missing = REQUIRED_RELEASE_FILES.filter(filename => !selectedNames.has(filename));
    const unexpected = selectedFiles.filter(file => !REQUIRED_RELEASE_FILES.includes(file.name)).map(file => file.name);
    if (!version) {
        showToast('请填写版本号', 'error');
        return;
    }
    if (missing.length || unexpected.length || selectedFiles.length !== REQUIRED_RELEASE_FILES.length) {
        showToast(`发布文件不完整${missing.length ? `，缺少：${missing.join('、')}` : ''}${unexpected.length ? `，多余：${unexpected.join('、')}` : ''}`, 'error');
        renderSelectedReleaseFiles();
        return;
    }

    const data = new FormData();
    data.append('version', version);
    data.append('promote_to_latest', document.getElementById('release-promote-latest').checked ? 'true' : 'false');
    selectedFiles.forEach(file => data.append('files', file, file.name));
    button.disabled = true;
    status.className = 'publish-status';
    status.textContent = '正在上传并校验完整发布包，请不要关闭页面...';
    try {
        const response = await fetch('/api/admin/releases/publish', { method: 'POST', body: data });
        const result = await readResponseJson(response);
        if (!response.ok) throw new Error(result.error || '发布失败');
        status.className = 'publish-status success';
        status.textContent = `版本 ${result.version} 发布成功${result.latest_updated ? '，latest 已更新' : ''}`;
        showToast(`CLI ${result.version} 发布成功`, 'success');
        input.value = '';
        renderSelectedReleaseFiles();
        await loadReleaseManagement();
    } catch (error) {
        status.className = 'publish-status error';
        status.textContent = error.message || '发布失败';
        showToast(status.textContent, 'error');
    } finally {
        button.disabled = false;
    }
}

async function promoteCliRelease(version, checksum) {
    if (!confirm(`确定将 latest 切换到 CLI ${version} 吗？\n客户端下一次检查更新时将看到这个版本。`)) return;
    try {
        const response = await fetch('/api/admin/releases/channel', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ channel: 'latest', version, checksum }),
        });
        const result = await readResponseJson(response);
        if (!response.ok) throw new Error(result.error || '切换 latest 失败');
        showToast(`latest 已切换到 ${version}`, 'success');
        await loadReleaseManagement();
    } catch (error) {
        showToast(error.message || '切换 latest 失败', 'error');
    }
}

// --- Managed File Center ---
async function loadManagedFiles() {
    const table = document.getElementById('managed-files-table');
    if (!table) return;
    table.innerHTML = '<tr><td colspan="5" style="color:var(--text-muted)">加载中...</td></tr>';
    try {
        const response = await fetch('/api/admin/files');
        const result = await readResponseJson(response);
        if (!response.ok) throw new Error(result.error || '加载文件列表失败');
        const files = result.files || [];
        table.innerHTML = files.map(file => {
            const versions = (file.versions || []).slice().sort((a, b) =>
                b.version.localeCompare(a.version, undefined, { numeric: true, sensitivity: 'base' }));
            const versionList = versions.map(version => {
                const state = file.current_version === version.version
                    ? '<span class="badge active">当前</span>'
                    : version.published_at
                        ? '<span class="badge ai">已发布</span>'
                        : '<span class="badge revoked">草稿</span>';
                return `<span class="version-chip">${escapeHtml(version.version)} · ${formatBytes(version.size_bytes)} ${state}</span>`;
            }).join('');
            const fixedLinkActions = versions
                .filter(version => version.published_at)
                .map(version => `<button class="btn btn-sm" onclick="copyPublishedUrl(${jsString(`/files/${file.slug}/${version.version}/download`)})">复制 ${escapeHtml(version.version)} 固定链接</button>`)
                .join('');
            const versionActions = versions
                .filter(version => version.version !== file.current_version)
                .map(version => `<button class="btn btn-sm" onclick="publishManagedFileVersion(${jsString(file.slug)}, ${jsString(version.version)})">发布 ${escapeHtml(version.version)}</button><button class="btn btn-sm btn-danger" onclick="deleteManagedFileVersion(${jsString(file.slug)}, ${jsString(version.version)})">删除 ${escapeHtml(version.version)}</button>`)
                .join('');
            return `<tr>
                <td><strong>${escapeHtml(file.name)}</strong><br><code style="color:var(--accent);font-size:0.72rem">${escapeHtml(file.slug)}</code><br><span style="color:var(--text-muted);font-size:0.7rem">${escapeHtml(file.description || '')}</span></td>
                <td>${file.current_version ? `<strong>${escapeHtml(file.current_version)}</strong>` : '<span class="badge revoked">未发布</span>'}</td>
                <td><div class="version-list">${versionList || '暂无版本'}</div></td>
                <td>${file.is_public ? '<span class="badge active">公开</span>' : '<span class="badge role">登录后下载</span>'}</td>
                <td><div class="action-group">
                    ${file.current_version ? `<button class="btn btn-sm btn-primary" onclick="copyPublishedUrl(${jsString(file.latest_download_url)})">复制下载链接</button>` : ''}
                    <button class="btn btn-sm" onclick="showEditManagedFileModal(${jsString(file.slug)}, ${jsString(file.name)}, ${jsString(file.description || '')}, ${file.is_public})">设置</button>
                    ${fixedLinkActions}
                    ${versionActions}
                </div></td>
            </tr>`;
        }).join('') || '<tr><td colspan="5"><div class="empty-state"><div class="empty-icon">📦</div><p>尚未上传普通文件</p></div></td></tr>';
    } catch (error) {
        table.innerHTML = `<tr><td colspan="5" style="color:var(--danger)">${escapeHtml(error.message || '加载文件列表失败')}</td></tr>`;
    }
}

async function uploadManagedFile(button) {
    const name = document.getElementById('managed-file-name').value.trim();
    const slug = document.getElementById('managed-file-slug').value.trim();
    const version = document.getElementById('managed-file-version').value.trim();
    const description = document.getElementById('managed-file-description').value.trim();
    const input = document.getElementById('managed-file-upload');
    const file = input.files?.[0];
    const status = document.getElementById('managed-file-upload-status');
    if (!name || !slug || !version || !file) {
        showToast('请填写名称、文件标识、版本号并选择文件', 'error');
        return;
    }

    const data = new FormData();
    data.append('name', name);
    data.append('slug', slug);
    data.append('version', version);
    data.append('description', description);
    data.append('is_public', document.getElementById('managed-file-public').checked ? 'true' : 'false');
    data.append('file', file, file.name);
    button.disabled = true;
    status.className = 'publish-status';
    status.textContent = `正在上传 ${file.name}（${formatBytes(file.size)}）...`;
    try {
        const response = await fetch('/api/admin/files/upload', { method: 'POST', body: data });
        const result = await readResponseJson(response);
        if (!response.ok) throw new Error(result.error || '上传失败');
        if (document.getElementById('managed-file-publish-now').checked) {
            const publishResponse = await fetch(`/api/admin/files/${encodeURIComponent(result.slug)}/publish`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ version: result.version }),
            });
            const publishResult = await readResponseJson(publishResponse);
            if (!publishResponse.ok) throw new Error(publishResult.error || '文件已上传，但发布失败');
        }
        status.className = 'publish-status success';
        status.textContent = `文件 ${result.filename} ${document.getElementById('managed-file-publish-now').checked ? '上传并发布' : '上传为草稿'}成功`;
        showToast(status.textContent, 'success');
        input.value = '';
        await loadManagedFiles();
    } catch (error) {
        status.className = 'publish-status error';
        status.textContent = error.message || '上传失败';
        showToast(status.textContent, 'error');
    } finally {
        button.disabled = false;
    }
}

async function publishManagedFileVersion(slug, version) {
    if (!confirm(`确定将 ${slug} 的当前版本切换到 ${version} 吗？`)) return;
    try {
        const response = await fetch(`/api/admin/files/${encodeURIComponent(slug)}/publish`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ version }),
        });
        const result = await readResponseJson(response);
        if (!response.ok) throw new Error(result.error || '发布失败');
        showToast(`${slug} 已发布 ${version}`, 'success');
        await loadManagedFiles();
    } catch (error) {
        showToast(error.message || '发布失败', 'error');
    }
}

async function deleteManagedFileVersion(slug, version) {
    if (!confirm(`确定删除 ${slug} ${version} 吗？此操作无法撤销。`)) return;
    try {
        const response = await fetch(`/api/admin/files/${encodeURIComponent(slug)}/versions/${encodeURIComponent(version)}`, { method: 'DELETE' });
        const result = await readResponseJson(response);
        if (!response.ok) throw new Error(result.error || '删除失败');
        showToast(`${slug} ${version} 已删除`, 'success');
        await loadManagedFiles();
    } catch (error) {
        showToast(error.message || '删除失败', 'error');
    }
}

function showEditManagedFileModal(slug, name, description, isPublic) {
    document.getElementById('modal-container').innerHTML = `
    <div class="modal-overlay" onclick="if(event.target===this)closeModal()">
        <div class="modal">
            <div class="modal-title">文件设置</div>
            <div class="form-group"><label class="form-label">文件标识</label><input id="edit-file-slug" class="form-input" value="${escapeAttribute(slug)}" disabled /></div>
            <div class="form-group"><label class="form-label">显示名称</label><input id="edit-file-name" class="form-input" value="${escapeAttribute(name)}" /></div>
            <div class="form-group"><label class="form-label">说明</label><input id="edit-file-description" class="form-input" value="${escapeAttribute(description)}" /></div>
            <label class="checkbox-label"><input id="edit-file-public" type="checkbox" ${isPublic ? 'checked' : ''} /> 公开下载（无需登录）</label>
            <div class="form-actions"><button class="btn" onclick="closeModal()">取消</button><button class="btn btn-primary" onclick="saveManagedFileSettings()">保存</button></div>
        </div>
    </div>`;
}

async function saveManagedFileSettings() {
    const slug = document.getElementById('edit-file-slug').value;
    const name = document.getElementById('edit-file-name').value.trim();
    const description = document.getElementById('edit-file-description').value.trim();
    const isPublic = document.getElementById('edit-file-public').checked;
    if (!name) {
        showToast('显示名称不能为空', 'error');
        return;
    }
    try {
        const response = await fetch(`/api/admin/files/${encodeURIComponent(slug)}`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name, description, is_public: isPublic }),
        });
        const result = await readResponseJson(response);
        if (!response.ok) throw new Error(result.error || '保存失败');
        closeModal();
        showToast('文件设置已保存', 'success');
        await loadManagedFiles();
    } catch (error) {
        showToast(error.message || '保存失败', 'error');
    }
}

async function copyPublishedUrl(path) {
    const url = new URL(path, window.location.origin).href;
    try {
        await copyHelpText(url);
        showToast('下载链接已复制', 'success');
    } catch (_) {
        showToast(`复制失败：${url}`, 'error');
    }
}

function closeModal() {
    document.getElementById('modal-container').innerHTML = '';
}

// --- Init ---
const requestedInitialSection = new URL(window.location.href).searchParams.get('section');
const initialSection = dashboardSectionFromLocation();
activateDashboardSection(initialSection);
if (requestedInitialSection && requestedInitialSection !== initialSection) {
    updateDashboardSectionUrl(initialSection, true);
}
if (!isAdmin) loadClientStatus();
updateRefreshTime();
startAutoRefresh();
