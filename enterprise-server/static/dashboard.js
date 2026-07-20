class ApiRequestError extends Error {
    constructor(message, { status = null, requestId = null, cause = null } = {}) {
        super(message, cause ? { cause } : undefined);
        this.name = this.constructor.name;
        this.status = status;
        this.requestId = requestId;
    }
}
class AuthExpiredError extends ApiRequestError {}
class PermissionDeniedError extends ApiRequestError {}
class HttpError extends ApiRequestError {}
class InvalidResponseError extends ApiRequestError {}
class NetworkError extends ApiRequestError {}
class TimeoutError extends ApiRequestError {}
class AbortError extends ApiRequestError {}

const API_DEFAULT_TIMEOUT_MS = 15000;
const API_GET_RETRIES = 1;
const API_RETRYABLE_STATUSES = new Set([429, 502, 503, 504]);

function requestIdFromResponse(response) {
    return response.headers?.get?.('x-request-id') || null;
}

function safeResponseMessage(data, status, fallback) {
    if (status >= 500) return fallback;
    const value = typeof data?.error === 'string'
        ? data.error
        : typeof data?.message === 'string'
            ? data.message
            : '';
    const message = value.replace(/\s+/g, ' ').trim();
    return message && message.length <= 300 ? message : fallback;
}

function authReturnTo() {
    return `${window.location.pathname}${window.location.search}${window.location.hash}`;
}

function redirectToLogin() {
    const loginUrl = `/auth/login?return_to=${encodeURIComponent(authReturnTo())}`;
    window.location.assign(loginUrl);
}

function waitForRetry(delayMs, signal) {
    return new Promise((resolve, reject) => {
        if (signal?.aborted) {
            reject(new AbortError('请求已取消'));
            return;
        }
        const onAbort = () => {
            clearTimeout(timer);
            reject(new AbortError('请求已取消'));
        };
        const timer = setTimeout(() => {
            signal?.removeEventListener('abort', onAbort);
            resolve();
        }, delayMs);
        signal?.addEventListener('abort', onAbort, { once: true });
    });
}

function createRequestSignal(externalSignal, timeoutMs) {
    const controller = new AbortController();
    let timedOut = false;
    const onExternalAbort = () => controller.abort(externalSignal.reason);
    if (externalSignal?.aborted) onExternalAbort();
    else externalSignal?.addEventListener('abort', onExternalAbort, { once: true });
    const timeout = Number.isFinite(timeoutMs) && timeoutMs > 0
        ? setTimeout(() => {
            timedOut = true;
            controller.abort();
        }, timeoutMs)
        : null;
    return {
        signal: controller.signal,
        timedOut: () => timedOut,
        cleanup: () => {
            if (timeout) clearTimeout(timeout);
            externalSignal?.removeEventListener('abort', onExternalAbort);
        },
    };
}

async function parseApiResponse(response) {
    const requestId = requestIdFromResponse(response);
    const contentType = response.headers?.get?.('content-type') || '';
    const body = await response.text();
    let data = null;
    if (body) {
        if (!contentType.toLowerCase().includes('application/json')) {
            if (response.ok) {
                throw new InvalidResponseError('服务器返回了非 JSON 响应', {
                    status: response.status,
                    requestId,
                });
            }
        } else {
            try {
                data = JSON.parse(body);
            } catch (cause) {
                if (response.ok) {
                    throw new InvalidResponseError('服务器返回了无效 JSON', {
                        status: response.status,
                        requestId,
                        cause,
                    });
                }
            }
        }
    }

    if (response.ok) {
        if (!body && response.status !== 204) {
            throw new InvalidResponseError('服务器返回了空响应', {
                status: response.status,
                requestId,
            });
        }
        return data;
    }
    if (response.status === 401) {
        throw new AuthExpiredError('登录已过期，请重新登录', {
            status: response.status,
            requestId,
        });
    }
    if (response.status === 403) {
        throw new PermissionDeniedError(
            safeResponseMessage(data, response.status, '没有执行此操作的权限'),
            { status: response.status, requestId },
        );
    }
    throw new HttpError(
        safeResponseMessage(data, response.status, `请求失败（HTTP ${response.status}）`),
        { status: response.status, requestId },
    );
}

async function apiRequest(url, options = {}) {
    const method = String(options.method || 'GET').toUpperCase();
    const retries = options.retries ?? (method === 'GET' ? API_GET_RETRIES : 0);
    const headers = new Headers(options.headers || {});
    if (!headers.has('Accept')) headers.set('Accept', 'application/json');

    for (let attempt = 0; attempt <= retries; attempt += 1) {
        const requestSignal = createRequestSignal(
            options.signal,
            options.timeoutMs ?? API_DEFAULT_TIMEOUT_MS,
        );
        try {
            const response = await fetch(url, {
                ...options,
                method,
                headers,
                signal: requestSignal.signal,
            });
            if (
                method === 'GET'
                && API_RETRYABLE_STATUSES.has(response.status)
                && attempt < retries
            ) {
                requestSignal.cleanup();
                await response.body?.cancel?.();
                await waitForRetry(250 * (2 ** attempt), options.signal);
                continue;
            }
            const data = await parseApiResponse(response);
            requestSignal.cleanup();
            return data;
        } catch (error) {
            const didTimeout = requestSignal.timedOut();
            requestSignal.cleanup();
            let typedError = error;
            if (options.signal?.aborted) {
                typedError = new AbortError('请求已取消', { cause: error });
            } else if (didTimeout) {
                typedError = new TimeoutError('请求超时，请稍后重试', { cause: error });
            } else if (error instanceof TypeError || error?.name === 'TypeError') {
                typedError = new NetworkError('网络连接失败，请检查网络后重试', {
                    cause: error,
                });
            }

            const retryableTransportError =
                typedError instanceof NetworkError || typedError instanceof TimeoutError;
            if (method === 'GET' && retryableTransportError && attempt < retries) {
                await waitForRetry(250 * (2 ** attempt), options.signal);
                continue;
            }
            if (typedError instanceof AuthExpiredError) redirectToLogin();
            throw typedError;
        }
    }
}

const fmt = n => typeof n === 'number' ? n.toLocaleString() : '0';
function finiteNumber(value, fallback = 0) {
    const number = Number(value);
    return Number.isFinite(number) ? number : fallback;
}
function clampPercent(value) {
    return Math.min(100, Math.max(0, finiteNumber(value)));
}
const pctBar = (pct) => `<div class="bar"><div class="bar-fill" style="width:${clampPercent(pct)}%"></div></div>`;
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
const RefreshMode = Object.freeze({
    INITIAL: 'initial',
    MANUAL: 'manual',
    AUTO: 'auto',
});
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
const sectionRefreshes = new Map();
const queuedManualRefreshes = new Map();
let lastRefreshAttemptAt = null;
let lastRefreshSuccessAt = null;
const successfulSections = new Set();
let departmentTreeRows = [];
let currentDepartmentLevelRows = [];
const departmentLevelCache = new Map();
const DEPARTMENT_LEVEL_CACHE_MS = 30000;
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

function isSilentRefresh(options) {
    return options?.mode === RefreshMode.AUTO;
}

function currentSectionRequestSignal() {
    return sectionRefreshes.get(currentSection)?.controller.signal;
}

function refreshCollisionAction(mode, hasInFlight) {
    if (!hasInFlight) return 'start';
    if (mode === RefreshMode.AUTO) return 'skip';
    if (mode === RefreshMode.MANUAL) return 'queue';
    return 'replace';
}

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

async function fetchPaginatedJson(key, url, errorMessage, signal) {
    const state = getTablePageState(key);
    state.loading = true;
    try {
        const d = await apiRequest(addPaginationParams(url, key), { signal });
        const pagination = d.pagination || {};
        state.nextCursor = pagination.next_cursor || null;
        state.hasMore = Boolean(pagination.has_more);
        return d;
    } catch (error) {
        if (!(error instanceof ApiRequestError)) {
            throw new InvalidResponseError(errorMessage, { cause: error });
        }
        throw error;
    } finally {
        state.loading = false;
    }
}

function pageItems(data, field) {
    return (data[field] || []).slice(0, TABLE_PAGE_SIZE);
}

function replaceHtmlIfChanged(element, nextHtml) {
    if (!element) return false;
    let comparableHtml = nextHtml;
    if (typeof element.cloneNode === 'function') {
        const comparisonElement = element.cloneNode(false);
        comparisonElement.innerHTML = nextHtml;
        comparableHtml = comparisonElement.innerHTML;
    }
    if (element.innerHTML === comparableHtml) return false;
    element.innerHTML = nextHtml;
    return true;
}

function setTableLoading(tbodyId, colspan, options) {
    if (isSilentRefresh(options)) return;
    replaceHtmlIfChanged(
        document.getElementById(tbodyId),
        `<tr><td colspan="${colspan}" style="color:var(--text-muted)">加载中...</td></tr>`,
    );
}

function renderPaginationControls(key) {
    const containerId = tablePagerContainers[key];
    const container = containerId ? document.getElementById(containerId) : null;
    if (!container) return;
    const state = getTablePageState(key);
    replaceHtmlIfChanged(container, `
        <button class="btn btn-sm" onclick="goToTablePage('${key}', 'prev')" ${state.page <= 1 || state.loading ? 'disabled' : ''}>上一页</button>
        <span class="pagination-status">第 ${state.page} 页</span>
        <button class="btn btn-sm" onclick="goToTablePage('${key}', 'next')" ${!state.hasMore || state.loading ? 'disabled' : ''}>下一页</button>
    `);
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
    if (!tablePagerContainers[key]) return Promise.resolve();
    return loadSection(currentSection, { mode: RefreshMode.MANUAL });
}

const OPTION_PAGE_LIMIT = 100;
const OPTION_SEARCH_DELAY_MS = 250;
const optionSearchTimers = new Map();
const optionRequestControllers = new Map();

function boundedOptionUrl(url, query = '') {
    const params = new URLSearchParams({ limit: String(OPTION_PAGE_LIMIT) });
    const normalizedQuery = String(query || '').trim();
    if (normalizedQuery) params.set('q', normalizedQuery);
    return `${url}${url.includes('?') ? '&' : '?'}${params.toString()}`;
}

async function fetchBoundedOptions(url, field, query, signal) {
    const data = await apiRequest(boundedOptionUrl(url, query), { signal });
    const items = data[field] || [];
    return {
        items: items.slice(0, OPTION_PAGE_LIMIT),
        hasMore: Boolean(data.pagination?.has_more) || items.length > OPTION_PAGE_LIMIT,
    };
}

function optionResultMessage(label, count, hasMore, query = '') {
    const normalizedQuery = String(query || '').trim();
    if (count === 0) {
        return normalizedQuery ? `未找到匹配${label}` : `暂无可选${label}`;
    }
    if (hasMore) {
        return `结果超过 ${OPTION_PAGE_LIMIT} 个，仅显示前 ${OPTION_PAGE_LIMIT} 个，请继续输入关键词缩小范围`;
    }
    return normalizedQuery ? `找到 ${count} 个${label}` : `已加载 ${count} 个${label}`;
}

function setOptionStatus(id, message, state = '') {
    const element = document.getElementById(id);
    if (!element) return;
    element.textContent = message;
    element.className = `option-search-status${state ? ` ${state}` : ''}`;
}

function beginOptionRequest(key) {
    optionRequestControllers.get(key)?.abort();
    const controller = new AbortController();
    optionRequestControllers.set(key, controller);
    return controller;
}

function finishOptionRequest(key, controller) {
    if (optionRequestControllers.get(key) === controller) {
        optionRequestControllers.delete(key);
    }
}

function scheduleOptionSearch(key, callback) {
    cancelOptionRequest(key);
    optionSearchTimers.set(key, setTimeout(() => {
        optionSearchTimers.delete(key);
        callback();
    }, OPTION_SEARCH_DELAY_MS));
}

function cancelOptionRequest(key) {
    clearTimeout(optionSearchTimers.get(key));
    optionSearchTimers.delete(key);
    const controller = optionRequestControllers.get(key);
    controller?.abort();
    if (optionRequestControllers.get(key) === controller) {
        optionRequestControllers.delete(key);
    }
}

function cancelOptionRequests() {
    optionSearchTimers.forEach(timer => clearTimeout(timer));
    optionSearchTimers.clear();
    optionRequestControllers.forEach(controller => controller.abort());
    optionRequestControllers.clear();
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

const mobileNavigationMediaQuery = window.matchMedia('(max-width: 768px)');
const mobileMenuButton = document.getElementById('mobile-menu-button');
const sidebarCloseButton = document.getElementById('sidebar-close-button');
const dashboardSidebar = document.getElementById('dashboard-sidebar');
const sidebarBackdrop = document.getElementById('sidebar-backdrop');
const dashboardMain = document.getElementById('dashboard-main');
const mobileTopbar = document.querySelector('.mobile-topbar');
let mobileNavigationReturnFocus = null;

function mobileNavigationFocusableElements() {
    return Array.from(
        dashboardSidebar.querySelectorAll(
            'a[href], button:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
    ).filter(element => {
        const style = window.getComputedStyle(element);
        return style.display !== 'none' && style.visibility !== 'hidden';
    });
}

function isMobileNavigationOpen() {
    return mobileNavigationMediaQuery.matches && dashboardSidebar.classList.contains('open');
}

function openMobileNavigation(trigger = mobileMenuButton) {
    if (!mobileNavigationMediaQuery.matches || isMobileNavigationOpen()) return;
    mobileNavigationReturnFocus = trigger;
    dashboardSidebar.inert = false;
    dashboardSidebar.setAttribute('aria-hidden', 'false');
    dashboardSidebar.classList.add('open');
    sidebarBackdrop.classList.add('open');
    document.body.classList.add('mobile-nav-open');
    dashboardMain.inert = true;
    mobileTopbar.inert = true;
    mobileMenuButton.setAttribute('aria-expanded', 'true');
    mobileMenuButton.setAttribute('aria-label', '导航菜单已打开');
    requestAnimationFrame(() => {
        const focusTarget = dashboardSidebar.querySelector('.nav-item.active')
            || mobileNavigationFocusableElements()[0];
        focusTarget?.focus();
    });
}

function closeMobileNavigation({ restoreFocus = true } = {}) {
    const wasOpen = dashboardSidebar.classList.contains('open');
    dashboardSidebar.classList.remove('open');
    sidebarBackdrop.classList.remove('open');
    document.body.classList.remove('mobile-nav-open');
    dashboardMain.inert = false;
    mobileTopbar.inert = false;
    mobileMenuButton.setAttribute('aria-expanded', 'false');
    mobileMenuButton.setAttribute('aria-label', '打开导航菜单');

    if (wasOpen && restoreFocus) {
        const focusTarget = mobileNavigationReturnFocus?.isConnected
            ? mobileNavigationReturnFocus
            : mobileMenuButton;
        focusTarget.focus({ preventScroll: true });
    }

    if (mobileNavigationMediaQuery.matches) {
        dashboardSidebar.inert = true;
        dashboardSidebar.setAttribute('aria-hidden', 'true');
    } else {
        dashboardSidebar.inert = false;
        dashboardSidebar.removeAttribute('aria-hidden');
    }
}

function handleMobileNavigationKeydown(event) {
    if (!isMobileNavigationOpen()) return;
    if (event.key === 'Escape') {
        event.preventDefault();
        closeMobileNavigation();
        return;
    }
    if (event.key !== 'Tab') return;
    const focusable = mobileNavigationFocusableElements();
    if (focusable.length === 0) {
        event.preventDefault();
        dashboardSidebar.focus();
        return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
    }
}

function syncMobileNavigation() {
    closeMobileNavigation({ restoreFocus: false });
}

function initializeMobileNavigation() {
    mobileMenuButton.addEventListener('click', () => openMobileNavigation());
    sidebarCloseButton.addEventListener('click', () => closeMobileNavigation());
    sidebarBackdrop.addEventListener('click', () => closeMobileNavigation());
    document.addEventListener('keydown', handleMobileNavigationKeydown);
    if (mobileNavigationMediaQuery.addEventListener) {
        mobileNavigationMediaQuery.addEventListener('change', syncMobileNavigation);
    } else {
        mobileNavigationMediaQuery.addListener(syncMobileNavigation);
    }
    syncMobileNavigation();
}

function startAutoRefresh() {
    stopAutoRefresh();
    refreshInterval = setInterval(
        () => {
            if (!document.hidden) refreshCurrentSection({ mode: RefreshMode.AUTO });
        },
        AUTO_REFRESH_MS,
    );
}
function stopAutoRefresh() {
    if (refreshInterval) { clearInterval(refreshInterval); refreshInterval = null; }
}
function formatRefreshTime(value) {
    if (!value) return '—';
    return value.toLocaleTimeString('zh-CN', { hour12: false });
}

function updateRefreshTime({ stale = false } = {}) {
    const status = document.getElementById('last-refresh');
    status.textContent = `最后成功: ${formatRefreshTime(lastRefreshSuccessAt)} · 最后尝试: ${formatRefreshTime(lastRefreshAttemptAt)}`;
    status.title = stale ? '后台刷新失败，当前显示的数据可能已过期' : '';
    document.querySelector('.refresh-dot')?.classList.toggle('stale', stale);
}

async function refreshCurrentSection({ mode = RefreshMode.MANUAL } = {}) {
    const sectionId = currentSection;
    return loadSection(sectionId, { mode });
}

function handleDashboardVisibilityChange() {
    if (!document.hidden) {
        refreshCurrentSection({ mode: RefreshMode.AUTO });
    }
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
    loadSection(nextSection, { mode: RefreshMode.INITIAL });
}

function showSection(event, id) {
    event.preventDefault();
    if (!canAccessDashboardSection(id)) return false;
    activateDashboardSection(id, { updateUrl: true });
    closeMobileNavigation();
    return false;
}

window.addEventListener('popstate', () => {
    activateDashboardSection(dashboardSectionFromLocation());
    closeMobileNavigation({ restoreFocus: false });
});

function clearSectionError(id) {
    document.querySelector(`#section-${id} > .section-request-error`)?.remove();
}

function showSectionLoadError(id, error, { background = false } = {}) {
    if (error instanceof AbortError) return;
    const requestId = error.requestId ? ` 请求 ID：${error.requestId}` : '';
    console.error('Dashboard request failed', {
        section: id,
        name: error.name,
        status: error.status,
        requestId: error.requestId,
        error,
    });
    if (error instanceof AuthExpiredError) return;
    if (background && successfulSections.has(id)) {
        if (currentSection === id) {
            updateRefreshTime({ stale: true });
            showToast(`后台刷新失败，当前数据可能已过期。${error.message}${requestId}`, 'error');
        }
        return;
    }

    clearSectionError(id);
    const section = document.getElementById(`section-${id}`);
    const banner = document.createElement('div');
    banner.className = 'section-request-error';
    const message = document.createElement('span');
    message.textContent = `${error.message || '栏目加载失败'}${requestId}`;
    const retry = document.createElement('button');
    retry.type = 'button';
    retry.className = 'btn btn-sm';
    retry.textContent = '重试';
    retry.addEventListener('click', () => loadSection(id, { mode: RefreshMode.MANUAL }));
    banner.append(message, retry);
    section.prepend(banner);
}

function loadSection(id, { mode = RefreshMode.MANUAL } = {}) {
    const existingRefresh = sectionRefreshes.get(id);
    const collisionAction = refreshCollisionAction(mode, Boolean(existingRefresh));
    if (collisionAction === 'skip') return Promise.resolve(false);
    if (collisionAction === 'queue') {
        const existingQueuedRefresh = queuedManualRefreshes.get(id);
        if (existingQueuedRefresh) return existingQueuedRefresh;

        // A manual refresh wins over an in-flight AUTO by running once after it settles.
        // Repeated clicks share this queued promise instead of adding more requests.
        const queuedRefresh = existingRefresh.promise.then(() => {
            queuedManualRefreshes.delete(id);
            return loadSection(id, { mode: RefreshMode.MANUAL });
        });
        queuedManualRefreshes.set(id, queuedRefresh);
        return queuedRefresh;
    }
    if (collisionAction === 'replace') existingRefresh.controller.abort();

    const controller = new AbortController();
    const refresh = { controller, promise: null };
    refresh.promise = performSectionLoad(id, { mode, controller })
        .finally(() => {
            if (sectionRefreshes.get(id) === refresh) {
                sectionRefreshes.delete(id);
            }
        });
    sectionRefreshes.set(id, refresh);
    return refresh.promise;
}

async function performSectionLoad(id, { mode, controller }) {
    const background = isSilentRefresh({ mode }) && successfulSections.has(id);
    if (currentSection === id) {
        lastRefreshAttemptAt = new Date();
        updateRefreshTime({ stale: false });
    }
    const loaders = {
        overview: loadOverview,
        trends: loadTrends,
        organizations: loadOrgs,
        developers: loadDevs,
        projects: loadProjects,
        tools: loadTools,
        users: loadUsers,
        departments: loadDepartments,
        apikeys: loadApiKeys,
        releases: loadReleaseManagement,
        files: loadManagedFiles,
        help: loadHelp,
    };
    try {
        await loaders[id]({ mode, signal: controller.signal });
        if (controller.signal.aborted) return false;
        successfulSections.add(id);
        clearSectionError(id);
        if (!isAdmin && currentSection === id) {
            await loadClientStatus({ mode, signal: controller.signal, sectionId: id });
        }
        if (currentSection === id) {
            lastRefreshSuccessAt = new Date();
            updateRefreshTime({ stale: false });
        }
        return true;
    } catch (error) {
        showSectionLoadError(id, error, { background });
        return false;
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

// --- Lazy help content ---
let helpContentLoaded = false;

function scrollToHelpHash() {
    const targetId = window.location.hash.slice(1);
    if (!targetId.startsWith('help-')) return;
    requestAnimationFrame(() => document.getElementById(targetId)?.scrollIntoView());
}

async function loadHelp({ signal, mode }) {
    const container = document.getElementById('help-content');
    if (!container) {
        throw new InvalidResponseError('帮助内容容器不存在');
    }
    if (helpContentLoaded || container.dataset.loaded === 'true') {
        if (!isSilentRefresh({ mode })) scrollToHelpHash();
        return;
    }

    container.setAttribute('aria-busy', 'true');
    const data = await apiRequest('/api/v1/dashboard/help', { signal });
    if (typeof data?.html !== 'string' || !data.html.trim()) {
        throw new InvalidResponseError('服务器返回了无效的帮助内容');
    }

    replaceHtmlIfChanged(container, data.html);
    container.dataset.loaded = 'true';
    container.removeAttribute('aria-busy');
    helpContentLoaded = true;
    if (!isSilentRefresh({ mode })) scrollToHelpHash();
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
let trendChartType = null;
let agentComparisonChart = null;
const chartDataSignatures = new WeakMap();
let developerGitInfo = new Map();

function setDisplayIfChanged(element, display) {
    if (!element || element.style.display === display) return false;
    element.style.display = display;
    return true;
}

function setTextIfChanged(element, text) {
    const nextText = String(text);
    if (!element || element.textContent === nextText) return false;
    element.textContent = nextText;
    return true;
}

function setClassNameIfChanged(element, className) {
    if (!element || element.className === className) return false;
    element.className = className;
    return true;
}

function setTitleIfChanged(element, title) {
    if (!element || element.title === title) return false;
    element.title = title;
    return true;
}

function chartDataSignature(labels, datasets) {
    return JSON.stringify({ labels, datasets });
}

function rememberChartData(chart, labels, datasets) {
    chartDataSignatures.set(chart, chartDataSignature(labels, datasets));
}

function updateChartDataIfChanged(chart, labels, datasets, options) {
    const nextSignature = chartDataSignature(labels, datasets);
    if (chartDataSignatures.get(chart) === nextSignature) return false;

    chart.data.labels = [...labels];
    chart.data.datasets = datasets.map(dataset => ({
        ...dataset,
        data: [...(dataset.data || [])],
    }));
    chartDataSignatures.set(chart, nextSignature);
    if (isSilentRefresh(options)) chart.update('none');
    else chart.update();
    return true;
}

function createOverviewTrendChart(canvas, labels, datasets) {
    const chart = new Chart(canvas.getContext('2d'), {
        type: 'line',
        data: { labels, datasets },
        options: {
            responsive: true, maintainAspectRatio: false,
            plugins: { legend: { labels: { color: '#94a3b8' } } },
            scales: {
                x: { ticks: { color: '#64748b', maxRotation: 45 }, grid: { color: '#1e293b' } },
                y: { ticks: { color: '#64748b' }, grid: { color: '#1e293b' } },
            }
        }
    });
    rememberChartData(chart, labels, datasets);
    return chart;
}

function createTrendChart(canvas, type, labels, datasets) {
    const chart = new Chart(canvas.getContext('2d'), {
        type,
        data: { labels, datasets },
        options: {
            responsive: true, maintainAspectRatio: false,
            plugins: { legend: { labels: { color: '#94a3b8' } } },
            scales: {
                x: { ticks: { color: '#64748b', maxRotation: 45 }, grid: { color: '#1e293b' } },
                y: { ticks: { color: '#64748b' }, grid: { color: '#1e293b' } },
            }
        }
    });
    rememberChartData(chart, labels, datasets);
    return chart;
}

function createAgentComparisonChart(canvas, labels, datasets) {
    const chart = new Chart(canvas.getContext('2d'), {
        type: 'bar',
        data: { labels, datasets },
        options: {
            responsive: true, maintainAspectRatio: false, indexAxis: 'y',
            plugins: { legend: { labels: { color: '#94a3b8' } } },
            scales: {
                x: { ticks: { color: '#64748b' }, grid: { color: '#1e293b' } },
                y: { ticks: { color: '#94a3b8' }, grid: { color: '#1e293b' } },
            }
        }
    });
    rememberChartData(chart, labels, datasets);
    return chart;
}

// --- Overview ---
async function loadOverview({ signal, mode }) {
    const rangeLabel = getTimeRangeLabel();
    setTextIfChanged(
        document.getElementById('overview-trend-title'),
        `AI 代码趋势（${rangeLabel}）`,
    );

    const [summaryResult, developersResult, trendResult] = await Promise.allSettled([
        apiRequest(withTimeRange('/api/v1/aggregate/summary'), { signal }),
        apiRequest(
            withTimeRange('/api/v1/aggregate/developers?limit=5'),
            { signal },
        ),
        apiRequest(
            withTimeRange('/api/v1/aggregate/trends?metric=ai_lines&granularity=day'),
            { signal },
        ),
    ]);

    if (signal?.aborted) throw new AbortError('请求已取消');

    if (summaryResult.status === 'fulfilled') renderOverviewSummary(summaryResult.value);
    if (developersResult.status === 'fulfilled') renderOverviewDevelopers(developersResult.value);
    if (trendResult.status === 'fulfilled') renderOverviewTrend(trendResult.value, { mode });

    const failedResult = [summaryResult, developersResult, trendResult]
        .find(result => result.status === 'rejected');
    if (failedResult) throw failedResult.reason;
}

function renderOverviewSummary(data) {
    setTextIfChanged(document.getElementById('s-commits'), fmt(data.total_commits));
    setTextIfChanged(document.getElementById('s-ai-lines'), fmt(data.total_ai_lines));
    setTextIfChanged(document.getElementById('s-human-lines'), fmt(data.total_human_lines));
    setTextIfChanged(
        document.getElementById('s-ai-pct'),
        `${clampPercent(data.pct_ai_lines).toFixed(1)}%`,
    );
    if (isAdmin) {
        setTextIfChanged(document.getElementById('s-devs'), fmt(data.total_developers));
    }
    setTextIfChanged(document.getElementById('s-projects'), fmt(data.total_projects));
}

function renderOverviewDevelopers(data) {
    const top = [...(data.developers || [])]
        .sort((a, b) => (b.ai_added_lines || 0) - (a.ai_added_lines || 0))
        .slice(0, 5);
    const maxLines = top.length ? Math.max(...top.map(x => x.total_added_lines || 0)) : 1;
    const nextHtml = top.map(dev => {
        const total = dev.total_added_lines || 0;
        const ai = dev.ai_added_lines || 0;
        const human = dev.human_added_lines || 0;
        const aiW = clampPercent(maxLines > 0 ? (ai / maxLines * 100) : 0);
        const humanW = clampPercent(maxLines > 0 ? (human / maxLines * 100) : 0);
        const displayName = escapeHtml(dev.name || dev.email || '未知');
        const displayEmail = escapeHtml(dev.email || '');
        return `<div class="chart-bar">
            <div class="chart-label" title="${displayName} ${displayEmail}">${displayName}</div>
            <div class="chart-track"><div class="chart-fill"><div class="ai-part" style="width:${aiW}%"></div><div class="human-part" style="width:${humanW}%"></div></div></div>
            <div class="chart-value">${fmt(total)} <span class="badge ai">${clampPercent(ai / (total || 1) * 100).toFixed(0)}% AI</span></div>
        </div>`;
    }).join('') || '<div class="empty-state"><div class="empty-icon">📭</div><p>暂无开发者数据</p></div>';
    replaceHtmlIfChanged(document.getElementById('top-developers'), nextHtml);
}

async function loadClientStatus({ signal, mode, sectionId = currentSection }) {
    if (isAdmin) return;
    const cardEl = document.getElementById('sidebar-gitai');
    const statusEl = document.getElementById('sidebar-gitai-status');
    const detailEl = document.getElementById('sidebar-gitai-detail');
    const dotEl = document.getElementById('sidebar-gitai-dot');
    const overviewStatusEl = document.getElementById('s-gitai-status');
    const overviewDetailEl = document.getElementById('s-gitai-detail');
    if ((!cardEl || !statusEl || !detailEl || !dotEl) && !overviewStatusEl) return;
    try {
        const d = await apiRequest('/api/v1/client/status', { signal });
        if (currentSection !== sectionId) return;
        if (!d.detected) {
            if (overviewStatusEl) {
                setTextIfChanged(overviewStatusEl, '未检测到');
                setClassNameIfChanged(overviewStatusEl, 'stat-value human');
            }
            if (overviewDetailEl) {
                setTextIfChanged(overviewDetailEl, 'CLI 登录后会显示同步信息');
                setTitleIfChanged(overviewDetailEl, 'CLI 登录后会显示同步信息');
            }
            if (statusEl && cardEl && dotEl && detailEl) {
                setTextIfChanged(statusEl, 'git-ai 未检测到');
                setClassNameIfChanged(cardEl, 'sidebar-gitai');
                setClassNameIfChanged(dotEl, 'sidebar-gitai-dot');
                setTextIfChanged(detailEl, 'CLI 登录后会显示状态');
                setTitleIfChanged(detailEl, '');
            }
            return;
        }

        const loggedIn = d.status === 'logged_in';
        const statusLabel = d.status_label || (loggedIn ? '已登录' : '已登出');
        if (overviewStatusEl) {
            setTextIfChanged(overviewStatusEl, statusLabel);
            setClassNameIfChanged(overviewStatusEl, loggedIn ? 'stat-value ai' : 'stat-value human');
        }
        if (statusEl && cardEl && dotEl) {
            setTextIfChanged(statusEl, `git-ai ${statusLabel}`);
            setClassNameIfChanged(cardEl, loggedIn ? 'sidebar-gitai online' : 'sidebar-gitai offline');
            setClassNameIfChanged(dotEl, loggedIn ? 'sidebar-gitai-dot online' : 'sidebar-gitai-dot offline');
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
            setTextIfChanged(overviewDetailEl, syncDetail);
            setTitleIfChanged(overviewDetailEl, syncDetail);
        }
        setTextIfChanged(detailEl, syncDetail);
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
        setTitleIfChanged(detailEl, titleParts.join(' · '));
    } catch(error) {
        if (error instanceof AbortError || error instanceof AuthExpiredError) throw error;
        console.error('Client status request failed', error);
        if (currentSection !== sectionId) return;
        if (isSilentRefresh({ mode })) return;
        if (overviewStatusEl) {
            setTextIfChanged(overviewStatusEl, '检测失败');
            setClassNameIfChanged(overviewStatusEl, 'stat-value pct');
        }
        if (overviewDetailEl) {
            setTextIfChanged(overviewDetailEl, '无法读取同步信息');
            setTitleIfChanged(overviewDetailEl, '无法读取同步信息');
        }
        if (statusEl && cardEl && dotEl && detailEl) {
            setTextIfChanged(statusEl, 'git-ai 检测失败');
            setClassNameIfChanged(cardEl, 'sidebar-gitai error');
            setClassNameIfChanged(dotEl, 'sidebar-gitai-dot error');
            setTextIfChanged(detailEl, '无法读取状态');
            setTitleIfChanged(detailEl, '');
        }
    }
}

function renderOverviewTrend(result, { mode }) {
    const data = result.data || [];
    const canvas = document.getElementById('overview-trend-chart');
    const empty = document.getElementById('overview-trend-empty');
    if (data.length === 0) {
        setDisplayIfChanged(canvas, 'none');
        setDisplayIfChanged(empty, 'block');
        return;
    }
    setDisplayIfChanged(canvas, 'block');
    setDisplayIfChanged(empty, 'none');

    const labels = data.map(point => point.period);
    const aiValues = data.map(point => point.ai_lines);
    const humanValues = data.map(point => point.human_lines);
    const datasets = [
        { label: 'AI 代码行', data: aiValues, borderColor: '#818cf8', backgroundColor: 'rgba(129,140,248,0.1)', fill: true, tension: 0.3 },
        { label: '非 AI 代码行', data: humanValues, borderColor: '#34d399', backgroundColor: 'rgba(52,211,153,0.1)', fill: true, tension: 0.3 },
    ];

    if (!overviewTrendChart) {
        overviewTrendChart = createOverviewTrendChart(canvas, labels, datasets);
    } else {
        updateChartDataIfChanged(overviewTrendChart, labels, datasets, { mode });
    }
}

// --- Trends ---
async function loadTrends({ signal, mode }) {
    const metric = document.getElementById('trend-metric').value;
    const granularity = document.getElementById('trend-granularity').value;

    const metricLabels = { ai_ratio: 'AI 占比', ai_lines: 'AI 代码行数', human_lines: '非 AI 代码行数', commits: '提交数' };
    const granLabels = { day: '按天', week: '按周', month: '按月' };
    document.getElementById('trend-chart-title').textContent =
        `${metricLabels[metric]}趋势（${granLabels[granularity]}）`;

        const d = await apiRequest(
            `/api/v1/aggregate/trends?metric=${encodeURIComponent(metric)}&granularity=${encodeURIComponent(granularity)}`,
            { signal },
        );
        const data = d.data || [];
        const canvas = document.getElementById('trend-chart');
        const empty = document.getElementById('trend-chart-empty');

        if (data.length === 0) {
            setDisplayIfChanged(canvas, 'none');
            setDisplayIfChanged(empty, 'block');
        } else {
            setDisplayIfChanged(canvas, 'block');
            setDisplayIfChanged(empty, 'none');

            const labels = data.map(p => p.period);
            const values = data.map(p => p.value);
            const isSinglePoint = data.length === 1;
            const nextChartType = isSinglePoint ? 'bar' : 'line';
            const datasets = [{
                label: metricLabels[metric],
                data: values,
                borderColor: '#818cf8',
                backgroundColor: isSinglePoint ? 'rgba(129,140,248,0.7)' : 'rgba(129,140,248,0.15)',
                fill: !isSinglePoint, tension: 0.3, pointRadius: 4,
                borderWidth: isSinglePoint ? 1 : 2,
            }];

            if (!trendChart || trendChartType !== nextChartType) {
                if (trendChart) trendChart.destroy();
                trendChart = createTrendChart(canvas, nextChartType, labels, datasets);
                trendChartType = nextChartType;
            } else {
                updateChartDataIfChanged(trendChart, labels, datasets, { mode });
            }
        }

    // Agent comparison chart
        const comparisonData = await apiRequest('/api/v1/aggregate/agent-comparison', { signal });
        const comps = (comparisonData.comparisons || []).slice(0, 10);
        const comparisonCanvas = document.getElementById('agent-comparison-chart');
        const comparisonEmpty = document.getElementById('agent-comparison-empty');
        if (comps.length === 0) {
            setDisplayIfChanged(comparisonCanvas, 'none');
            setDisplayIfChanged(comparisonEmpty, 'block');
        } else {
            setDisplayIfChanged(comparisonCanvas, 'block');
            setDisplayIfChanged(comparisonEmpty, 'none');
            const labels = comps.map(c => c.tool_model);
            const aiData = comps.map(c => c.ai_additions || 0);
            const datasets = [{
                label: 'AI 代码行数',
                data: aiData,
                backgroundColor: 'rgba(129,140,248,0.7)',
                borderColor: '#818cf8',
                borderWidth: 1,
            }];

            if (!agentComparisonChart) {
                agentComparisonChart = createAgentComparisonChart(comparisonCanvas, labels, datasets);
            } else {
                updateChartDataIfChanged(agentComparisonChart, labels, datasets, { mode });
            }
        }
}

// --- Organizations ---
async function loadOrgs({ signal, mode }) {
    setTableLoading('org-table', 5, { mode });
    try {
        const d = await fetchPaginatedJson('organizations', '/api/v1/aggregate/organizations', '加载组织数据失败', signal);
        const nextHtml = pageItems(d, 'organizations').map(o => {
            return `<tr>
                <td><strong>${escapeHtml(o.organization)}</strong><br><span style="color:var(--text-muted);font-size:0.75rem">${escapeHtml(o.org_slug || '')}</span></td>
                <td>${fmt(o.total_commits)}</td>
                <td>${fmt(o.w_ai)}</td>
                <td>${fmt(o.w_human)}</td>
                <td>${pctBar(o.pct_ai)} <span style="font-size:0.8rem">${clampPercent(o.pct_ai).toFixed(1)}%</span></td>
            </tr>`;
        }).join('') || '<tr><td colspan="5" style="color:var(--text-muted)">暂无组织数据</td></tr>';
        replaceHtmlIfChanged(document.getElementById('org-table'), nextHtml);
        renderPaginationControls('organizations');
    } catch(e) {
        renderPaginationControls('organizations');
        throw e;
    }
}

// --- Developers ---
let developerSortBy = 'ai_lines';
let developerSortOrder = 'desc';

function changeDeveloperSorting() {
    developerSortBy = document.getElementById('developer-sort-by')?.value || 'ai_lines';
    developerSortOrder = document.getElementById('developer-sort-order')?.value || 'desc';
    resetTablePage('developers');
    loadSection('developers', { mode: RefreshMode.MANUAL });
}

async function loadDevs({ signal, mode }) {
    setTableLoading('dev-table', 8, { mode });
    try {
        const developerUrl = `/api/v1/aggregate/developers?sort_by=${encodeURIComponent(developerSortBy)}&sort_order=${encodeURIComponent(developerSortOrder)}`;
        const d = await fetchPaginatedJson('developers', developerUrl, '加载开发者数据失败', signal);
        const developers = pageItems(d, 'developers');
        const nextDeveloperGitInfo = new Map();
        if (developers.length === 0) {
            developerGitInfo = nextDeveloperGitInfo;
            replaceHtmlIfChanged(
                document.getElementById('dev-table'),
                '<tr><td colspan="8" style="color:var(--text-muted)">暂无开发者数据</td></tr>',
            );
            renderPaginationControls('developers');
            return;
        }

        const nextHtml = developers.map(dev => {
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
            nextDeveloperGitInfo.set(devId, {
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
                <td>${pctBar(dev.pct_ai)} <span style="font-size:0.8rem">${clampPercent(dev.pct_ai).toFixed(1)}%</span></td>
                <td><button class="btn btn-sm" onclick="showDeveloperGitInfo(${actionDevId})">Git 信息</button></td>
            </tr>`;
        }).join('');
        developerGitInfo = nextDeveloperGitInfo;
        replaceHtmlIfChanged(document.getElementById('dev-table'), nextHtml);
        renderPaginationControls('developers');
    } catch(e) {
        renderPaginationControls('developers');
        throw e;
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
async function loadProjects({ signal, mode }) {
    setTableLoading('proj-table', 6, { mode });
    try {
        const d = await fetchPaginatedJson('projects', '/api/v1/aggregate/projects', '加载项目数据失败', signal);
        const nextHtml = pageItems(d, 'projects').map(p => {
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
                <td>${pctBar(p.pct_ai)} <span style="font-size:0.8rem">${clampPercent(p.pct_ai).toFixed(1)}%</span></td>
            </tr>`;
        }).join('') || '<tr><td colspan="6" style="color:var(--text-muted)">暂无项目数据</td></tr>';
        replaceHtmlIfChanged(document.getElementById('proj-table'), nextHtml);
        renderPaginationControls('projects');
    } catch(e) {
        renderPaginationControls('projects');
        throw e;
    }
}

// --- Tools ---
async function loadTools({ signal, mode }) {
    setTableLoading('tools-table', 5, { mode });
    try {
        const d = await fetchPaginatedJson('tools', '/api/v1/aggregate/tools', '加载工具数据失败', signal);
        const tools = pageItems(d, 'tools');
        if (tools.length === 0) {
            replaceHtmlIfChanged(
                document.getElementById('tools-table'),
                '<tr><td colspan="5" style="color:var(--text-muted)">暂无工具使用数据，数据将在报告上传或指标事件后显示</td></tr>',
            );
            renderPaginationControls('tools');
            return;
        }
        const nextHtml = tools.map(t => {
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
        replaceHtmlIfChanged(document.getElementById('tools-table'), nextHtml);
        renderPaginationControls('tools');
    } catch(e) {
        renderPaginationControls('tools');
        throw e;
    }
}

// --- Users Management ---
const selectedGitTrackingUserIds = new Set();
let visibleGitTrackingUserIds = [];

async function loadUsers({ signal, mode }) {
    setTableLoading('users-table', 7, { mode });
    try {
        const d = await fetchPaginatedJson('users', '/api/admin/users/list', '加载用户列表失败', signal);
        const users = pageItems(d, 'users');
        const previousVisibleUserIds = new Set(visibleGitTrackingUserIds);
        const nextVisibleUserIds = users
            .filter(user => user.git_tracking_upload_enabled !== true)
            .map(user => user.id);
        const nextVisibleUserIdSet = new Set(nextVisibleUserIds);
        if (isSilentRefresh({ mode })) {
            selectedGitTrackingUserIds.forEach(userId => {
                if (previousVisibleUserIds.has(userId) && !nextVisibleUserIdSet.has(userId)) {
                    selectedGitTrackingUserIds.delete(userId);
                }
            });
        } else {
            selectedGitTrackingUserIds.clear();
        }
        visibleGitTrackingUserIds = nextVisibleUserIds;

        if (users.length === 0) {
            replaceHtmlIfChanged(
                document.getElementById('users-table'),
                '<tr><td colspan="7"><div class="empty-state"><div class="empty-icon">👤</div><p>暂无用户，点击上方按钮创建</p></div></td></tr>',
            );
            updateGitTrackingBulkSelection();
            renderPaginationControls('users');
            return;
        }
        const nextHtml = users.map(u => {
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
            const actionUserId = jsString(u.id);
            const userIdAttribute = escapeAttribute(u.id);
            const uploadEnabled = u.git_tracking_upload_enabled === true;
            const uploadStatus = uploadEnabled
                ? '<span class="badge active">已授权</span>'
                : '<span class="badge revoked">未授权</span>';
            const selected = !uploadEnabled && selectedGitTrackingUserIds.has(u.id);
            return `<tr>
                <td class="selection-column"><input class="git-tracking-user-checkbox" type="checkbox" value="${userIdAttribute}" aria-label="选择${displayName}" onchange="toggleGitTrackingUser(${actionUserId}, this.checked)" ${uploadEnabled ? 'disabled' : ''} ${selected ? 'checked' : ''} /></td>
                <td><strong>${displayName}</strong></td>
                <td>${displayEmail}</td>
                <td>${uploadStatus}</td>
                <td>${keyCount > 0 ? keyBadges + moreKeys : '<span style="color:var(--text-muted)">无密钥</span>'}</td>
                <td>${created}</td>
                <td>
                    <button class="btn btn-sm ${uploadEnabled ? 'btn-danger' : 'btn-primary'}" onclick="setGitTrackingUploadAuthorization(${actionUserId}, ${actionName}, ${!uploadEnabled}, this)">${uploadEnabled ? '撤销上传' : '授权上传'}</button>
                    <button class="btn btn-sm" onclick="showCreateApiKeyForUser(${actionUserId}, ${actionName})">🔑 创建密钥</button>
                    <button class="btn btn-sm btn-danger" onclick="deleteUser(${actionUserId}, ${actionName})">删除</button>
                </td>
            </tr>`;
        }).join('');
        replaceHtmlIfChanged(document.getElementById('users-table'), nextHtml);
        updateGitTrackingBulkSelection();
        renderPaginationControls('users');
    } catch(e) {
        renderPaginationControls('users');
        throw e;
    }
}

function toggleGitTrackingUser(userId, selected) {
    if (selected) selectedGitTrackingUserIds.add(userId);
    else selectedGitTrackingUserIds.delete(userId);
    const checkbox = Array.from(document.querySelectorAll('.git-tracking-user-checkbox'))
        .find(element => element.value === userId);
    checkbox?.toggleAttribute('checked', selected);
    updateGitTrackingBulkSelection();
}

function toggleAllGitTrackingUsers(selected) {
    visibleGitTrackingUserIds.forEach(userId => {
        if (selected) selectedGitTrackingUserIds.add(userId);
        else selectedGitTrackingUserIds.delete(userId);
    });
    document.querySelectorAll('.git-tracking-user-checkbox:not(:disabled)').forEach(checkbox => {
        checkbox.checked = selected;
        checkbox.toggleAttribute('checked', selected);
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
        const result = await apiRequest('/api/admin/users/git-tracking-upload/authorize', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ user_ids: userIds })
        });
        showToast(`已为 ${result.authorized_count || userIds.length} 位用户授权 Git 追踪上传`, 'success');
        await loadSection('users');
    } catch(e) {
        showToast(`批量授权失败：${e.message}`, 'error');
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
        await apiRequest(`/api/admin/users/${encodeURIComponent(userId)}/git-tracking-upload`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ authorized })
        });

        showToast(`已${actionLabel}开发者「${userName}」的 Git 追踪上传权限`, 'success');
        await loadSection('users');
    } catch(e) {
        showToast(`${actionLabel}失败：${e.message}`, 'error');
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
                <input type="search" id="create-user-org-search" class="form-input option-search-input" placeholder="按组织名称或标识搜索" />
                <select id="create-user-org" class="form-input" disabled>
                    <option value="">加载组织中...</option>
                </select>
                <div id="create-user-org-status" class="option-search-status loading" role="status" aria-live="polite">正在加载组织...</div>
            </div>
            <div class="form-group">
                <label class="form-label">部门</label>
                <input type="search" id="create-user-dept-search" class="form-input option-search-input" placeholder="按部门名称或编码搜索" disabled />
                <select id="create-user-dept" class="form-input" disabled>
                    <option value="">请先选择组织</option>
                </select>
                <div id="create-user-dept-status" class="option-search-status" role="status" aria-live="polite">请先选择组织</div>
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
        const departmentSearch = document.getElementById('create-user-dept-search');
        departmentSearch.value = '';
        departmentSearch.disabled = !event.currentTarget.value;
        loadCreateUserDepartments(event.currentTarget.value);
    });
    document.getElementById('create-user-org-search').addEventListener('input', event => {
        const query = event.currentTarget.value;
        setOptionStatus('create-user-org-status', '正在搜索组织...', 'loading');
        scheduleOptionSearch(
            'create-user-org',
            () => populateCreateUserOrganizations(query),
        );
    });
    document.getElementById('create-user-dept-search').addEventListener('input', event => {
        const query = event.currentTarget.value;
        setOptionStatus('create-user-dept-status', '正在搜索部门...', 'loading');
        scheduleOptionSearch(
            'create-user-dept',
            () => loadCreateUserDepartments(
                document.getElementById('create-user-org')?.value,
                query,
            ),
        );
    });
    populateCreateUserOrganizations();
}

async function populateCreateUserOrganizations(query = '') {
    const orgSelect = document.getElementById('create-user-org');
    const deptSelect = document.getElementById('create-user-dept');
    const departmentSearch = document.getElementById('create-user-dept-search');
    if (!orgSelect || !deptSelect) return;
    const previousOrgId = orgSelect.value;
    const controller = beginOptionRequest('create-user-org');
    orgSelect.disabled = true;
    setOptionStatus('create-user-org-status', '正在加载组织...', 'loading');
    try {
        const normalizedQuery = String(query || '').trim();
        const result = normalizedQuery
            ? await fetchBoundedOptions(
                '/api/admin/organizations/list?include_personal=false',
                'organizations',
                normalizedQuery,
                controller.signal,
            )
            : await loadAdminOrganizations(controller.signal);
        if (optionRequestControllers.get('create-user-org') !== controller) return;
        const orgs = result.items;
        if (orgs.length === 0) {
            cancelOptionRequest('create-user-dept');
            orgSelect.innerHTML = '<option value="">暂无可选组织</option>';
            deptSelect.innerHTML = '<option value="">暂无可选部门</option>';
            deptSelect.disabled = true;
            departmentSearch.disabled = true;
            setOptionStatus(
                'create-user-org-status',
                optionResultMessage('组织', 0, false, normalizedQuery),
                'empty',
            );
            setOptionStatus('create-user-dept-status', '请先选择组织');
            if (!normalizedQuery) showToast('请先创建组织', 'error');
            return;
        }

        orgSelect.innerHTML = '<option value="">请选择组织</option>' + orgs.map(org => {
            const label = `${escapeHtml(org.name || org.slug)}${org.slug ? ' (' + escapeHtml(org.slug) + ')' : ''}`;
            return `<option value="${escapeAttribute(org.id)}">${label}</option>`;
        }).join('');
        orgSelect.disabled = false;
        setOptionStatus(
            'create-user-org-status',
            optionResultMessage('组织', orgs.length, result.hasMore, normalizedQuery),
            result.hasMore ? 'limited' : 'success',
        );

        if (previousOrgId && orgs.some(org => org.id === previousOrgId)) {
            orgSelect.value = previousOrgId;
            departmentSearch.disabled = false;
        } else if (!normalizedQuery && orgs.length === 1 && !result.hasMore) {
            orgSelect.value = orgs[0].id;
            departmentSearch.disabled = false;
            await loadCreateUserDepartments(orgs[0].id);
        } else {
            cancelOptionRequest('create-user-dept');
            deptSelect.innerHTML = '<option value="">请先选择组织</option>';
            deptSelect.disabled = true;
            departmentSearch.disabled = true;
            setOptionStatus('create-user-dept-status', '请先选择组织');
        }
    } catch(e) {
        if (e instanceof AbortError) return;
        cancelOptionRequest('create-user-dept');
        orgSelect.innerHTML = '<option value="">组织加载失败</option>';
        deptSelect.innerHTML = '<option value="">请先选择组织</option>';
        deptSelect.disabled = true;
        departmentSearch.disabled = true;
        setOptionStatus('create-user-org-status', '组织加载失败，请重试', 'error');
        setOptionStatus('create-user-dept-status', '请先选择组织');
        showToast(e.message || '加载组织列表失败', 'error');
    } finally {
        finishOptionRequest('create-user-org', controller);
    }
}

async function loadCreateUserDepartments(orgId, query = '') {
    const deptSelect = document.getElementById('create-user-dept');
    const departmentSearch = document.getElementById('create-user-dept-search');
    if (!deptSelect || !departmentSearch) return;
    const controller = beginOptionRequest('create-user-dept');
    deptSelect.disabled = true;
    if (!orgId) {
        deptSelect.innerHTML = '<option value="">请先选择组织</option>';
        departmentSearch.disabled = true;
        setOptionStatus('create-user-dept-status', '请先选择组织');
        finishOptionRequest('create-user-dept', controller);
        return;
    }

    departmentSearch.disabled = false;
    deptSelect.innerHTML = '<option value="">加载部门中...</option>';
    setOptionStatus('create-user-dept-status', '正在加载部门...', 'loading');
    try {
        const normalizedQuery = String(query || '').trim();
        const result = await fetchBoundedOptions(
            `/api/admin/departments?org_id=${encodeURIComponent(orgId)}`,
            'departments',
            normalizedQuery,
            controller.signal,
        );
        if (optionRequestControllers.get('create-user-dept') !== controller) return;
        const departments = result.items;
        if (departments.length === 0) {
            deptSelect.innerHTML = '<option value="">该组织暂无部门</option>';
            setOptionStatus(
                'create-user-dept-status',
                optionResultMessage('部门', 0, false, normalizedQuery),
                'empty',
            );
            return;
        }

        deptSelect.innerHTML = '<option value="">请选择部门</option>' + departments.map(dept => {
            const label = escapeHtml(dept.name || dept.slug || '未命名部门');
            return `<option value="${escapeAttribute(dept.id)}">${label}</option>`;
        }).join('');
        deptSelect.disabled = false;
        setOptionStatus(
            'create-user-dept-status',
            optionResultMessage('部门', departments.length, result.hasMore, normalizedQuery),
            result.hasMore ? 'limited' : 'success',
        );

        if (departments.length === 1 && !result.hasMore) {
            deptSelect.value = departments[0].id;
        }
    } catch(e) {
        if (e instanceof AbortError) return;
        deptSelect.innerHTML = '<option value="">部门加载失败</option>';
        setOptionStatus('create-user-dept-status', '部门加载失败，请重试', 'error');
        showToast(e.message || '加载部门列表失败', 'error');
    } finally {
        finishOptionRequest('create-user-dept', controller);
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
        const d = await apiRequest('/api/admin/users', {
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
        let msg = `用户 ${name} 创建成功！`;
        if (d.install_nonce) msg += `\\n安装令牌: ${d.install_nonce}`;
        showToast(msg, 'success');
        closeModal();
        resetTablePage('users');
        loadSection('users');
    } catch(e) {
        showToast(`创建失败：${e.message}`, 'error');
    }
}

async function deleteUser(userId, userName) {
    if (!confirm(`确定要删除用户「${userName}」吗？此操作不可撤销。`)) return;
    try {
        await apiRequest(`/api/admin/users/${encodeURIComponent(userId)}`, { method: 'DELETE' });
        showToast(`用户「${userName}」已删除`, 'success');
        resetTablePage('users');
        loadSection('users');
    } catch(e) {
        showToast(`删除失败：${e.message}`, 'error');
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

async function loadAdminOrganizations(signal = currentSectionRequestSignal()) {
    if (adminOrganizationsCache) return adminOrganizationsCache;
    adminOrganizationsCache = await fetchBoundedOptions(
        '/api/admin/organizations/list?include_personal=false',
        'organizations',
        '',
        signal,
    );
    return adminOrganizationsCache;
}

async function loadDepartments({ signal, mode }) {
    setTableLoading('departments-table', 6, { mode });
    const departmentUrl = activeDepartmentParentId && isAdmin
        ? `/api/v1/aggregate/departments?parent_id=${encodeURIComponent(activeDepartmentParentId)}`
        : '/api/v1/aggregate/departments';
    const data = await fetchPaginatedJson(
        'departments',
        departmentUrl,
        '加载部门数据失败',
        signal,
    );
    if (isAdmin && activeDepartmentParentId && data.parent_exists === false) {
        activeDepartmentParentId = null;
        resetTablePage('departments');
        return loadDepartments({ signal, mode });
    }

    const nextDepartmentLevelRows = pageItems(data, 'departments');
    mergeDepartmentNodes(nextDepartmentLevelRows);
    currentDepartmentLevelRows = nextDepartmentLevelRows;
    departmentLevelCache.set(departmentLevelCacheKey(), {
        expiresAt: Date.now() + DEPARTMENT_LEVEL_CACHE_MS,
        rows: nextDepartmentLevelRows,
    });
    if (currentDepartmentLevelRows.length === 0) {
        renderDepartmentBreadcrumb();
        replaceHtmlIfChanged(
            document.getElementById('departments-table'),
            `<tr><td colspan="6"><div class="empty-state"><div class="empty-icon">🏷️</div><p>${activeDepartmentParentId ? '当前层级暂无下级部门' : (isAdmin ? '暂无部门数据' : '当前账号尚未分配部门')}</p></div></td></tr>`,
        );
        renderPaginationControls('departments');
        return;
    }

    renderDepartmentLevel();
    renderPaginationControls('departments');
}

function departmentLevelCacheKey() {
    const state = getTablePageState('departments');
    const cursor = state.cursors[state.page - 1] || '';
    return `${activeDepartmentParentId || 'root'}:${state.page}:${cursor}`;
}

function mergeDepartmentNodes(rows) {
    const byId = new Map(departmentTreeRows.map(department => [department.id, department]));
    rows.forEach(department => byId.set(department.id, department));
    departmentTreeRows = Array.from(byId.values());
}

function renderCachedDepartmentLevel() {
    const cached = departmentLevelCache.get(departmentLevelCacheKey());
    if (!cached || cached.expiresAt <= Date.now()) return false;
    currentDepartmentLevelRows = cached.rows;
    mergeDepartmentNodes(cached.rows);
    renderDepartmentLevel();
    renderPaginationControls('departments');
    return true;
}

function openDepartmentLevel(parentId) {
    if (!isAdmin) return;
    activeDepartmentParentId = parentId || null;
    resetTablePage('departments');
    renderCachedDepartmentLevel();
    loadSection('departments', { mode: RefreshMode.INITIAL });
}

function backDepartmentLevel() {
    if (!isAdmin) return;
    if (!activeDepartmentParentId) return;
    const current = departmentTreeRows.find(dept => dept.id === activeDepartmentParentId);
    activeDepartmentParentId = current?.parent_id || null;
    resetTablePage('departments');
    renderCachedDepartmentLevel();
    loadSection('departments', { mode: RefreshMode.INITIAL });
}

function renderDepartmentBreadcrumb() {
    const breadcrumb = document.getElementById('departments-breadcrumb');
    const backButton = document.getElementById('departments-back');
    if (!breadcrumb || !backButton) return;

    if (!isAdmin) {
        replaceHtmlIfChanged(breadcrumb, '<strong>我的部门</strong>');
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
            parts.push(`<button class="btn btn-sm" onclick="openDepartmentLevel(${jsString(dept.id)})">${escapeHtml(dept.department || '—')}</button>`);
        }
    });
    replaceHtmlIfChanged(breadcrumb, parts.join(' '));

    backButton.style.display = activeDepartmentParentId ? '' : 'none';
}

function renderDepartmentLevel() {
    renderDepartmentBreadcrumb();
    const departments = currentDepartmentLevelRows
        .slice()
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
        replaceHtmlIfChanged(
            document.getElementById('departments-table'),
            '<tr><td colspan="6"><div class="empty-state"><div class="empty-icon">🏷️</div><p>当前层级暂无下级部门</p></div></td></tr>',
        );
        return;
    }

    const nextHtml = departments.map(dept => {
            const departmentName = escapeHtml(dept.department || '—');
            const departmentCode = escapeHtml(dept.code || '—');
            const orgName = escapeHtml(dept.organization || '—');
            const nodeIcon = isAdmin && dept.has_children ? '›' : '•';
            const rowAction = isAdmin && dept.has_children
                ? ` onclick="openDepartmentLevel(${jsString(dept.id)})" style="cursor:pointer"`
                : '';
            const total = dept.w_total || 0;
            const pct = clampPercent(departmentAiPercentage(dept) * 100);
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
    replaceHtmlIfChanged(document.getElementById('departments-table'), nextHtml);
}

function departmentAiPercentage(department) {
    const total = Number(department.w_total) || 0;
    const ai = Number(department.w_ai) || 0;
    return total > 0 ? ai / total : 0;
}

async function showCreateDepartmentModal() {
    document.getElementById('modal-container').innerHTML = `
    <div class="modal-overlay" onclick="if(event.target===this)closeModal()">
        <div class="modal">
            <div class="modal-title">新增部门</div>
            <div class="form-group">
                <label class="form-label">所属组织</label>
                <input type="search" id="create-dept-org-search" class="form-input option-search-input" placeholder="按组织名称或标识搜索" />
                <select id="create-dept-org" class="form-input" disabled>
                    <option value="">加载组织中...</option>
                </select>
                <div id="create-dept-org-status" class="option-search-status loading" role="status" aria-live="polite">正在加载组织...</div>
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
                <input type="search" id="create-dept-parent-search" class="form-input option-search-input" placeholder="按部门名称或编码搜索" disabled />
                <select id="create-dept-parent" class="form-input" disabled>
                    <option value="">请先选择组织</option>
                </select>
                <div id="create-dept-parent-status" class="option-search-status" role="status" aria-live="polite">请先选择组织</div>
            </div>
            <div class="form-actions">
                <button class="btn" onclick="closeModal()">取消</button>
                <button class="btn btn-primary" onclick="createDepartment()">新增</button>
            </div>
        </div>
    </div>`;
    document.getElementById('create-dept-org').addEventListener('change', event => {
        const parentSearch = document.getElementById('create-dept-parent-search');
        parentSearch.value = '';
        parentSearch.disabled = !event.currentTarget.value;
        loadCreateDepartmentParents(event.currentTarget.value);
    });
    document.getElementById('create-dept-org-search').addEventListener('input', event => {
        const query = event.currentTarget.value;
        setOptionStatus('create-dept-org-status', '正在搜索组织...', 'loading');
        scheduleOptionSearch(
            'create-dept-org',
            () => populateCreateDepartmentOrganizations(query),
        );
    });
    document.getElementById('create-dept-parent-search').addEventListener('input', event => {
        const query = event.currentTarget.value;
        setOptionStatus('create-dept-parent-status', '正在搜索部门...', 'loading');
        scheduleOptionSearch(
            'create-dept-parent',
            () => loadCreateDepartmentParents(
                document.getElementById('create-dept-org')?.value,
                query,
            ),
        );
    });
    await populateCreateDepartmentOrganizations();
}

async function populateCreateDepartmentOrganizations(query = '') {
    const orgSelect = document.getElementById('create-dept-org');
    const parentSelect = document.getElementById('create-dept-parent');
    const parentSearch = document.getElementById('create-dept-parent-search');
    if (!orgSelect || !parentSelect || !parentSearch) return;
    const previousOrgId = orgSelect.value;
    const controller = beginOptionRequest('create-dept-org');
    orgSelect.disabled = true;
    setOptionStatus('create-dept-org-status', '正在加载组织...', 'loading');
    try {
        const normalizedQuery = String(query || '').trim();
        const result = normalizedQuery
            ? await fetchBoundedOptions(
                '/api/admin/organizations/list?include_personal=false',
                'organizations',
                normalizedQuery,
                controller.signal,
            )
            : await loadAdminOrganizations(controller.signal);
        if (optionRequestControllers.get('create-dept-org') !== controller) return;
        const orgs = result.items;
        if (orgs.length === 0) {
            cancelOptionRequest('create-dept-parent');
            orgSelect.innerHTML = '<option value="">暂无可选组织</option>';
            parentSelect.innerHTML = '<option value="">请先选择组织</option>';
            parentSelect.disabled = true;
            parentSearch.disabled = true;
            setOptionStatus(
                'create-dept-org-status',
                optionResultMessage('组织', 0, false, normalizedQuery),
                'empty',
            );
            setOptionStatus('create-dept-parent-status', '请先选择组织');
            if (!normalizedQuery) showToast('请先创建组织', 'error');
            return;
        }

        orgSelect.innerHTML = orgs.map(org => {
            const label = `${escapeHtml(org.name || org.slug)}${org.slug ? ' (' + escapeHtml(org.slug) + ')' : ''}`;
            return `<option value="${escapeAttribute(org.id)}">${label}</option>`;
        }).join('');
        orgSelect.disabled = false;
        setOptionStatus(
            'create-dept-org-status',
            optionResultMessage('组织', orgs.length, result.hasMore, normalizedQuery),
            result.hasMore ? 'limited' : 'success',
        );

        orgSelect.value = orgs.some(org => org.id === previousOrgId)
            ? previousOrgId
            : orgs[0].id;
        parentSearch.disabled = false;
        parentSearch.value = '';
        await loadCreateDepartmentParents(orgSelect.value);
    } catch(e) {
        if (e instanceof AbortError) return;
        cancelOptionRequest('create-dept-parent');
        orgSelect.innerHTML = '<option value="">组织加载失败</option>';
        parentSelect.innerHTML = '<option value="">请先选择组织</option>';
        parentSelect.disabled = true;
        parentSearch.disabled = true;
        setOptionStatus('create-dept-org-status', '组织加载失败，请重试', 'error');
        setOptionStatus('create-dept-parent-status', '请先选择组织');
        showToast(e.message || '加载组织列表失败', 'error');
    } finally {
        finishOptionRequest('create-dept-org', controller);
    }
}

async function loadCreateDepartmentParents(orgId, query = '') {
    const parentSelect = document.getElementById('create-dept-parent');
    const parentSearch = document.getElementById('create-dept-parent-search');
    if (!parentSelect || !parentSearch) return;
    const controller = beginOptionRequest('create-dept-parent');
    let loaded = false;
    if (!orgId) {
        parentSelect.disabled = true;
        parentSelect.innerHTML = '<option value="">请先选择组织</option>';
        parentSearch.disabled = true;
        setOptionStatus('create-dept-parent-status', '请先选择组织');
        finishOptionRequest('create-dept-parent', controller);
        return;
    }

    parentSearch.disabled = false;
    parentSelect.disabled = true;
    parentSelect.innerHTML = '<option value="">加载上级部门中...</option>';
    setOptionStatus('create-dept-parent-status', '正在加载部门...', 'loading');
    try {
        const normalizedQuery = String(query || '').trim();
        const result = await fetchBoundedOptions(
            `/api/admin/departments?org_id=${encodeURIComponent(orgId)}`,
            'departments',
            normalizedQuery,
            controller.signal,
        );
        if (optionRequestControllers.get('create-dept-parent') !== controller) return;
        const departments = result.items;
        parentSelect.innerHTML = '<option value="">无（根部门）</option>' + departments.map(dept => {
            const label = `${escapeHtml(dept.code || '—')} · ${escapeHtml(dept.name || '—')}`;
            return `<option value="${escapeAttribute(dept.id)}">${label}</option>`;
        }).join('');
        setOptionStatus(
            'create-dept-parent-status',
            optionResultMessage('部门', departments.length, result.hasMore, normalizedQuery),
            departments.length === 0
                ? 'empty'
                : result.hasMore ? 'limited' : 'success',
        );
        loaded = true;
    } catch (e) {
        if (e instanceof AbortError) return;
        parentSelect.innerHTML = '<option value="">上级部门加载失败</option>';
        setOptionStatus('create-dept-parent-status', '部门加载失败，请重试', 'error');
        showToast(e.message || '加载上级部门失败', 'error');
    } finally {
        if (optionRequestControllers.get('create-dept-parent') === controller) {
            parentSelect.disabled = !loaded;
        }
        finishOptionRequest('create-dept-parent', controller);
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
        await apiRequest('/api/admin/departments', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ org_id, name, code, parent_id })
        });
        showToast(`部门「${name}」已新增`, 'success');
        closeModal();
        resetTablePage('departments');
        loadSection('departments');
    } catch(e) {
        showToast(`新增失败：${e.message}`, 'error');
    }
}

// --- API Key Management ---
async function loadApiKeys({ signal, mode }) {
    setTableLoading('apikeys-table', 7, { mode });
    try {
        const d = await fetchPaginatedJson('apikeys', '/api/admin/api-keys', '加载密钥列表失败', signal);
        const keys = pageItems(d, 'api_keys');
        if (keys.length === 0) {
            replaceHtmlIfChanged(
                document.getElementById('apikeys-table'),
                '<tr><td colspan="7"><div class="empty-state"><div class="empty-icon">🔑</div><p>暂无 API 密钥，点击上方按钮创建</p></div></td></tr>',
            );
            renderPaginationControls('apikeys');
            return;
        }
        const nextHtml = keys.map(k => {
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
                <td><button class="btn btn-sm btn-danger" onclick="revokeApiKey(${jsString(k.id)}, ${actionName})">撤销</button></td>
            </tr>`;
        }).join('');
        replaceHtmlIfChanged(document.getElementById('apikeys-table'), nextHtml);
        renderPaginationControls('apikeys');
    } catch(e) {
        renderPaginationControls('apikeys');
        throw e;
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
            <input type="hidden" id="create-key-user-id" value="${escapeAttribute(userId)}" />
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
        const d = await apiRequest('/api/admin/api-keys', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name, scopes })
        });
        document.getElementById('new-key-result').style.display = 'block';
        renderApiKeyValue(d.key);
        document.getElementById('create-key-btn').style.display = 'none';
        showToast('API 密钥创建成功', 'success');
        resetTablePage('apikeys');
        if (currentSection === 'apikeys') loadSection('apikeys');
    } catch(e) {
        showToast(`创建失败：${e.message}`, 'error');
    }
}

async function createApiKeyForUser() {
    const name = document.getElementById('create-key-name').value.trim();
    const userId = document.getElementById('create-key-user-id').value;
    if (!name) { showToast('请填写密钥名称', 'error'); return; }

    const scopes = Array.from(document.querySelectorAll('.key-scope:checked')).map(cb => cb.value);
    if (scopes.length === 0) { showToast('请至少选择一个权限范围', 'error'); return; }

    try {
        const d = await apiRequest('/api/admin/api-keys', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name, scopes, user_id: userId })
        });
        document.getElementById('new-key-result').style.display = 'block';
        renderApiKeyValue(d.key);
        document.getElementById('create-key-btn').style.display = 'none';
        showToast('API 密钥创建成功', 'success');
        resetTablePage('users');
        resetTablePage('apikeys');
        if (['users', 'apikeys'].includes(currentSection)) loadSection(currentSection);
    } catch(e) {
        showToast(`创建失败：${e.message}`, 'error');
    }
}

function renderApiKeyValue(value) {
    const keyEl = document.getElementById('new-key-value');
    const secret = String(value ?? '');
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'copy-btn';
    button.textContent = '复制';
    button.addEventListener('click', copyKey);
    const text = document.createElement('span');
    text.textContent = secret;

    keyEl.dataset.secret = secret;
    keyEl.replaceChildren(button, text);
}

function copyKey() {
    const keyEl = document.getElementById('new-key-value');
    navigator.clipboard.writeText(keyEl.dataset.secret || '')
        .then(() => showToast('已复制到剪贴板', 'success'));
}

async function revokeApiKey(keyId, keyName) {
    if (!confirm(`确定要撤销密钥「${keyName}」吗？撤销后此密钥将立即失效。`)) return;
    try {
        await apiRequest(`/api/admin/api-keys/${encodeURIComponent(keyId)}`, { method: 'DELETE' });
        showToast(`密钥「${keyName}」已撤销`, 'success');
        resetTablePage('apikeys');
        loadSection('apikeys');
    } catch(e) {
        showToast(`撤销失败：${e.message}`, 'error');
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
const BYTES_PER_MIB = 1024 * 1024;
const DEFAULT_RELEASE_FILE_MAX_BYTES = 100 * BYTES_PER_MIB;
const DEFAULT_RELEASE_TOTAL_MAX_BYTES = 500 * BYTES_PER_MIB;
const DEFAULT_MANAGED_FILE_MAX_BYTES = 500 * BYTES_PER_MIB;
const UPLOAD_REQUEST_TIMEOUT_MS = 15 * 60 * 1000;
const activeUploads = new Map();

function formatBytes(value) {
    const bytes = Number(value || 0);
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

function inputUploadLimit(input, datasetKey, fallback) {
    const value = Number(input?.dataset?.[datasetKey]);
    return Number.isFinite(value) && value > 0 ? value : fallback;
}

function utf8ByteLength(value) {
    return new TextEncoder().encode(String(value ?? '')).length;
}

function isValidUploadVersion(value) {
    const version = String(value ?? '').trim();
    return version !== '.'
        && version !== '..'
        && utf8ByteLength(version) <= 80
        && /^[A-Za-z0-9._+-]+$/.test(version);
}

function isSafeUploadFilename(value) {
    const filename = String(value ?? '').trim();
    return Boolean(filename)
        && filename !== '.'
        && filename !== '..'
        && utf8ByteLength(filename) <= 240
        && !filename.includes('/')
        && !filename.includes('\\')
        && !filename.includes('\0');
}

function managedFileExtension(filename) {
    const normalized = String(filename ?? '').trim().toLowerCase();
    if (normalized.endsWith('.tar.gz')) return '.tar.gz';
    const separator = normalized.lastIndexOf('.');
    return separator > 0 && separator < normalized.length - 1
        ? normalized.slice(separator)
        : '无扩展名';
}

function analyzeReleaseFiles(
    files,
    {
        maxFileBytes = DEFAULT_RELEASE_FILE_MAX_BYTES,
        maxTotalBytes = DEFAULT_RELEASE_TOTAL_MAX_BYTES,
    } = {},
) {
    const selectedFiles = Array.from(files || []);
    const filesByName = new Map();
    const nameCounts = new Map();
    selectedFiles.forEach(file => {
        if (!filesByName.has(file.name)) filesByName.set(file.name, file);
        nameCounts.set(file.name, (nameCounts.get(file.name) || 0) + 1);
    });
    const missing = REQUIRED_RELEASE_FILES.filter(filename => !filesByName.has(filename));
    const unexpected = selectedFiles
        .filter(file => !REQUIRED_RELEASE_FILES.includes(file.name))
        .map(file => file.name);
    const duplicates = Array.from(nameCounts.entries())
        .filter(([, count]) => count > 1)
        .map(([filename]) => filename);
    const empty = selectedFiles.filter(file => file.size === 0).map(file => file.name);
    const oversized = selectedFiles
        .filter(file => file.size > maxFileBytes)
        .map(file => file.name);
    const totalBytes = selectedFiles.reduce((total, file) => total + Number(file.size || 0), 0);
    const valid = selectedFiles.length === REQUIRED_RELEASE_FILES.length
        && missing.length === 0
        && unexpected.length === 0
        && duplicates.length === 0
        && empty.length === 0
        && oversized.length === 0
        && totalBytes <= maxTotalBytes;

    return {
        duplicates,
        empty,
        files: selectedFiles,
        filesByName,
        maxFileBytes,
        maxTotalBytes,
        missing,
        oversized,
        totalBytes,
        unexpected,
        valid,
    };
}

function releaseSelectionError(analysis) {
    if (analysis.valid) return '';
    const issues = [];
    if (analysis.files.length !== REQUIRED_RELEASE_FILES.length) {
        issues.push(`必须选择 ${REQUIRED_RELEASE_FILES.length} 个文件`);
    }
    if (analysis.missing.length) issues.push(`缺少：${analysis.missing.join('、')}`);
    if (analysis.unexpected.length) issues.push(`文件名不正确：${analysis.unexpected.join('、')}`);
    if (analysis.duplicates.length) issues.push(`文件名重复：${analysis.duplicates.join('、')}`);
    if (analysis.empty.length) issues.push(`空文件：${analysis.empty.join('、')}`);
    if (analysis.oversized.length) {
        issues.push(
            `单文件超过 ${formatBytes(analysis.maxFileBytes)}：${analysis.oversized.join('、')}`,
        );
    }
    if (analysis.totalBytes > analysis.maxTotalBytes) {
        issues.push(`总大小超过 ${formatBytes(analysis.maxTotalBytes)}`);
    }
    return issues.join('；');
}

function analyzeManagedFiles(
    files,
    { maxFileBytes = DEFAULT_MANAGED_FILE_MAX_BYTES } = {},
) {
    const selectedFiles = Array.from(files || []);
    const file = selectedFiles[0] || null;
    const invalidName = Boolean(file && !isSafeUploadFilename(file.name));
    const empty = Boolean(file && file.size === 0);
    const oversized = Boolean(file && file.size > maxFileBytes);
    return {
        empty,
        extension: file ? managedFileExtension(file.name) : '',
        file,
        files: selectedFiles,
        invalidName,
        maxFileBytes,
        oversized,
        valid: selectedFiles.length === 1 && !invalidName && !empty && !oversized,
    };
}

function managedSelectionError(analysis) {
    if (analysis.valid) return '';
    if (analysis.files.length !== 1) return '每次必须且只能选择一个文件';
    if (analysis.invalidName) return '文件名无效或超过 240 字节';
    if (analysis.empty) return '不能上传空文件';
    if (analysis.oversized) {
        return `文件超过 ${formatBytes(analysis.maxFileBytes)} 上限`;
    }
    return '文件无效';
}

function beginUpload(key, button, busyLabel) {
    if (activeUploads.has(key)) return false;
    activeUploads.set(key, {
        button,
        label: button?.textContent || '',
    });
    if (button) {
        button.disabled = true;
        button.textContent = busyLabel;
    }
    return true;
}

function finishUpload(key) {
    const operation = activeUploads.get(key);
    if (!operation) return;
    if (operation.button) {
        operation.button.disabled = false;
        operation.button.textContent = operation.label;
    }
    activeUploads.delete(key);
}

function warnBeforeLeavingDuringUpload(event) {
    if (activeUploads.size === 0) return;
    event.preventDefault();
    event.returnValue = '';
}

function renderSelectedReleaseFiles() {
    const input = document.getElementById('release-files');
    const target = document.getElementById('release-selected-files');
    if (!input || !target) return;
    const analysis = analyzeReleaseFiles(input.files, {
        maxFileBytes: inputUploadLimit(
            input,
            'maxFileBytes',
            DEFAULT_RELEASE_FILE_MAX_BYTES,
        ),
        maxTotalBytes: inputUploadLimit(
            input,
            'maxTotalBytes',
            DEFAULT_RELEASE_TOTAL_MAX_BYTES,
        ),
    });
    const fileRows = REQUIRED_RELEASE_FILES.map(filename => {
        const file = analysis.filesByName.get(filename);
        return file
            ? `<span class="selected-file-ok">✓ ${escapeHtml(filename)}（${formatBytes(file.size)}）</span>`
            : `<span class="selected-file-missing">缺少 ${escapeHtml(filename)}</span>`;
    }).join('<br>');
    const unexpectedRows = analysis.unexpected
        .map(filename => `<span class="selected-file-missing">文件名不正确：${escapeHtml(filename)}</span>`)
        .join('<br>');
    const summaryClass = analysis.valid ? 'selected-file-ok' : 'selected-file-missing';
    target.innerHTML = `${fileRows}${unexpectedRows ? `<br>${unexpectedRows}` : ''}
        <div class="selected-file-summary ${summaryClass}">
            已选择 ${analysis.files.length}/${REQUIRED_RELEASE_FILES.length} 个文件 ·
            总计 ${formatBytes(analysis.totalBytes)} / ${formatBytes(analysis.maxTotalBytes)}
            ${analysis.valid ? ' · 可以上传' : ` · ${escapeHtml(releaseSelectionError(analysis))}`}
        </div>`;
}

function renderSelectedManagedFile() {
    const input = document.getElementById('managed-file-upload');
    const target = document.getElementById('managed-file-selected');
    if (!input || !target) return;
    const analysis = analyzeManagedFiles(input.files, {
        maxFileBytes: inputUploadLimit(
            input,
            'maxFileBytes',
            DEFAULT_MANAGED_FILE_MAX_BYTES,
        ),
    });
    if (!analysis.file) {
        target.textContent = '尚未选择文件';
        return;
    }
    const stateClass = analysis.valid ? 'selected-file-ok' : 'selected-file-missing';
    target.innerHTML = `<span class="${stateClass}">
        ${analysis.valid ? '✓' : '⚠'} ${escapeHtml(analysis.file.name)}
        · ${formatBytes(analysis.file.size)}
        · 扩展名 ${escapeHtml(analysis.extension)}
        · 上限 ${formatBytes(analysis.maxFileBytes)}
        ${analysis.valid ? '' : ` · ${escapeHtml(managedSelectionError(analysis))}`}
    </span>`;
}

async function loadReleaseManagement({ signal, mode }) {
    const table = document.getElementById('release-table');
    if (!table) return;
    if (!isSilentRefresh({ mode })) {
        replaceHtmlIfChanged(
            table,
            '<tr><td colspan="5" style="color:var(--text-muted)">加载中...</td></tr>',
        );
    }
        const [metadata, assetData] = await Promise.all([
            apiRequest('/worker/releases', { signal }),
            apiRequest('/api/admin/releases/assets', { signal }),
        ]);

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
        const nextStatsHtml = `
            <div class="stat-card"><div class="stat-label">LATEST</div><div class="stat-value total">${escapeHtml(latest?.version || '未发布')}</div><div class="stat-detail">${latest ? '客户端自动更新目标' : '尚未设置 latest 渠道'}</div></div>
            <div class="stat-card"><div class="stat-label">版本数量</div><div class="stat-value total">${versionGroups.size}</div><div class="stat-detail">包含草稿和已发布版本</div></div>
            <div class="stat-card"><div class="stat-label">发布文件</div><div class="stat-value total">${(assetData.assets || []).length}</div><div class="stat-detail">跨平台二进制、脚本与校验文件</div></div>`;
        replaceHtmlIfChanged(stats, nextStatsHtml);

        const versions = Array.from(versionGroups.keys()).sort((a, b) =>
            b.localeCompare(a, undefined, { numeric: true, sensitivity: 'base' }));
        const nextTableHtml = versions.map(version => {
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
        replaceHtmlIfChanged(table, nextTableHtml);
}

async function publishCliRelease(button) {
    const version = document.getElementById('release-version').value.trim();
    const input = document.getElementById('release-files');
    const status = document.getElementById('release-publish-status');
    const analysis = analyzeReleaseFiles(input.files, {
        maxFileBytes: inputUploadLimit(
            input,
            'maxFileBytes',
            DEFAULT_RELEASE_FILE_MAX_BYTES,
        ),
        maxTotalBytes: inputUploadLimit(
            input,
            'maxTotalBytes',
            DEFAULT_RELEASE_TOTAL_MAX_BYTES,
        ),
    });
    if (!isValidUploadVersion(version)) {
        showToast('版本号只能使用字母、数字、点、短横线、下划线和加号', 'error');
        return;
    }
    const selectionError = releaseSelectionError(analysis);
    if (selectionError) {
        showToast(selectionError, 'error');
        renderSelectedReleaseFiles();
        return;
    }
    if (!beginUpload('release', button, '正在上传...')) {
        showToast('CLI 发布正在进行，请勿重复提交', 'error');
        return;
    }

    const data = new FormData();
    data.append('version', version);
    data.append('promote_to_latest', document.getElementById('release-promote-latest').checked ? 'true' : 'false');
    analysis.files.forEach(file => data.append('files', file, file.name));
    status.className = 'publish-status';
    status.textContent = `正在上传并校验 ${analysis.files.length} 个文件（共 ${formatBytes(analysis.totalBytes)}），请不要关闭页面...`;
    try {
        const result = await apiRequest('/api/admin/releases/publish', {
            method: 'POST',
            body: data,
            timeoutMs: UPLOAD_REQUEST_TIMEOUT_MS,
        });
        status.className = 'publish-status success';
        status.textContent = `版本 ${result.version} 发布成功${result.latest_updated ? '，latest 已更新' : ''}`;
        showToast(`CLI ${result.version} 发布成功`, 'success');
        input.value = '';
        renderSelectedReleaseFiles();
        await loadSection('releases');
    } catch (error) {
        status.className = 'publish-status error';
        status.textContent = error.message || '发布失败';
        showToast(status.textContent, 'error');
    } finally {
        finishUpload('release');
    }
}

async function promoteCliRelease(version, checksum) {
    if (!confirm(`确定将 latest 切换到 CLI ${version} 吗？\n客户端下一次检查更新时将看到这个版本。`)) return;
    try {
        await apiRequest('/api/admin/releases/channel', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ channel: 'latest', version, checksum }),
        });
        showToast(`latest 已切换到 ${version}`, 'success');
        await loadSection('releases');
    } catch (error) {
        showToast(error.message || '切换 latest 失败', 'error');
    }
}

// --- Managed File Center ---
async function loadManagedFiles({ signal, mode }) {
    const table = document.getElementById('managed-files-table');
    if (!table) return;
    if (!isSilentRefresh({ mode })) {
        replaceHtmlIfChanged(
            table,
            '<tr><td colspan="5" style="color:var(--text-muted)">加载中...</td></tr>',
        );
    }
        const result = await apiRequest('/api/admin/files', { signal });
        const files = result.files || [];
        const nextHtml = files.map(file => {
            const isPublic = file.is_public === true;
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
                <td>${isPublic ? '<span class="badge active">公开</span>' : '<span class="badge role">登录后下载</span>'}</td>
                <td><div class="action-group">
                    ${file.current_version ? `<button class="btn btn-sm btn-primary" onclick="copyPublishedUrl(${jsString(file.latest_download_url)})">复制下载链接</button>` : ''}
                    <button class="btn btn-sm" onclick="showEditManagedFileModal(${jsString(file.slug)}, ${jsString(file.name)}, ${jsString(file.description || '')}, ${isPublic})">设置</button>
                    ${fixedLinkActions}
                    ${versionActions}
                </div></td>
            </tr>`;
        }).join('') || '<tr><td colspan="5"><div class="empty-state"><div class="empty-icon">📦</div><p>尚未上传普通文件</p></div></td></tr>';
        replaceHtmlIfChanged(table, nextHtml);
}

async function uploadManagedFile(button) {
    const name = document.getElementById('managed-file-name').value.trim();
    const slug = document.getElementById('managed-file-slug').value.trim();
    const version = document.getElementById('managed-file-version').value.trim();
    const description = document.getElementById('managed-file-description').value.trim();
    const input = document.getElementById('managed-file-upload');
    const status = document.getElementById('managed-file-upload-status');
    const analysis = analyzeManagedFiles(input.files, {
        maxFileBytes: inputUploadLimit(
            input,
            'maxFileBytes',
            DEFAULT_MANAGED_FILE_MAX_BYTES,
        ),
    });
    const file = analysis.file;
    if (!name || !slug || !version || !file) {
        showToast('请填写名称、文件标识、版本号并选择文件', 'error');
        return;
    }
    if (!/^[a-z0-9_-]{1,80}$/.test(slug)) {
        showToast('文件标识只能使用小写字母、数字、短横线和下划线', 'error');
        return;
    }
    if (!isValidUploadVersion(version)) {
        showToast('版本号只能使用字母、数字、点、短横线、下划线和加号', 'error');
        return;
    }
    const selectionError = managedSelectionError(analysis);
    if (selectionError) {
        showToast(selectionError, 'error');
        renderSelectedManagedFile();
        return;
    }
    if (!beginUpload('managed-file', button, '正在上传...')) {
        showToast('文件上传正在进行，请勿重复提交', 'error');
        return;
    }

    const data = new FormData();
    data.append('name', name);
    data.append('slug', slug);
    data.append('version', version);
    data.append('description', description);
    data.append('is_public', document.getElementById('managed-file-public').checked ? 'true' : 'false');
    data.append('file', file, file.name);
    status.className = 'publish-status';
    status.textContent = `正在上传 ${file.name}（${formatBytes(file.size)}，${analysis.extension}）...`;
    try {
        const result = await apiRequest('/api/admin/files/upload', {
            method: 'POST',
            body: data,
            timeoutMs: UPLOAD_REQUEST_TIMEOUT_MS,
        });
        if (document.getElementById('managed-file-publish-now').checked) {
            await apiRequest(`/api/admin/files/${encodeURIComponent(result.slug)}/publish`, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ version: result.version }),
            });
        }
        status.className = 'publish-status success';
        status.textContent = `文件 ${result.filename} ${document.getElementById('managed-file-publish-now').checked ? '上传并发布' : '上传为草稿'}成功`;
        showToast(status.textContent, 'success');
        input.value = '';
        renderSelectedManagedFile();
        await loadSection('files');
    } catch (error) {
        status.className = 'publish-status error';
        status.textContent = error.message || '上传失败';
        showToast(status.textContent, 'error');
    } finally {
        finishUpload('managed-file');
    }
}

async function publishManagedFileVersion(slug, version) {
    if (!confirm(`确定将 ${slug} 的当前版本切换到 ${version} 吗？`)) return;
    try {
        await apiRequest(`/api/admin/files/${encodeURIComponent(slug)}/publish`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ version }),
        });
        showToast(`${slug} 已发布 ${version}`, 'success');
        await loadSection('files');
    } catch (error) {
        showToast(error.message || '发布失败', 'error');
    }
}

async function deleteManagedFileVersion(slug, version) {
    if (!confirm(`确定删除 ${slug} ${version} 吗？此操作无法撤销。`)) return;
    try {
        await apiRequest(
            `/api/admin/files/${encodeURIComponent(slug)}/versions/${encodeURIComponent(version)}`,
            { method: 'DELETE' },
        );
        showToast(`${slug} ${version} 已删除`, 'success');
        await loadSection('files');
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
        await apiRequest(`/api/admin/files/${encodeURIComponent(slug)}`, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name, description, is_public: isPublic }),
        });
        closeModal();
        showToast('文件设置已保存', 'success');
        await loadSection('files');
    } catch (error) {
        showToast(error.message || '保存失败', 'error');
    }
}

async function copyPublishedUrl(path) {
    try {
        const url = new URL(String(path ?? ''), window.location.origin);
        if (!['http:', 'https:'].includes(url.protocol) || url.origin !== window.location.origin) {
            throw new Error('Only same-origin HTTP(S) download URLs are allowed');
        }
        await copyHelpText(url);
        showToast('下载链接已复制', 'success');
    } catch (_) {
        showToast('复制失败：下载地址不受信任', 'error');
    }
}

function closeModal() {
    cancelOptionRequests();
    document.getElementById('modal-container').innerHTML = '';
}

// --- Init ---
initializeMobileNavigation();
document.addEventListener('visibilitychange', handleDashboardVisibilityChange);
window.addEventListener('beforeunload', warnBeforeLeavingDuringUpload);
const requestedInitialSection = new URL(window.location.href).searchParams.get('section');
const initialSection = dashboardSectionFromLocation();
activateDashboardSection(initialSection);
if (requestedInitialSection && requestedInitialSection !== initialSection) {
    updateDashboardSectionUrl(initialSection, true);
}
updateRefreshTime();
startAutoRefresh();
