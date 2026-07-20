import {
    ApiRequestError,
    InvalidResponseError,
} from './api.js';
import {
    escapeAttribute,
    replaceHtmlIfChanged,
} from './render.js';

export const TABLE_PAGE_SIZE = 25;

const TABLE_PAGER_CONTAINERS = Object.freeze({
    organizations: 'org-pagination',
    departments: 'departments-pagination',
    developers: 'dev-pagination',
    projects: 'proj-pagination',
    tools: 'tools-pagination',
    users: 'users-pagination',
    apikeys: 'apikeys-pagination',
});

function initialPageState() {
    return {
        page: 1,
        cursors: [null],
        nextCursor: null,
        hasMore: false,
        loading: false,
    };
}

export function createDashboardPagination({
    apiRequest,
    document,
    reloadTable,
    pageSize = TABLE_PAGE_SIZE,
    pagerContainers = TABLE_PAGER_CONTAINERS,
}) {
    const tablePageState = new Map();

    function getTablePageState(key) {
        if (!tablePageState.has(key)) {
            tablePageState.set(key, initialPageState());
        }
        return tablePageState.get(key);
    }

    function getTablePageSnapshot(key) {
        const state = getTablePageState(key);
        return Object.freeze({
            page: state.page,
            cursor: state.cursors[state.page - 1] || null,
            nextCursor: state.nextCursor,
            hasMore: state.hasMore,
            loading: state.loading,
        });
    }

    function resetTablePage(key) {
        tablePageState.set(key, initialPageState());
    }

    function addPaginationParams(url, key) {
        const state = getTablePageState(key);
        const cursor = state.cursors[state.page - 1];
        const params = new URLSearchParams({ limit: String(pageSize) });
        if (cursor) params.set('cursor', cursor);
        return `${url}${url.includes('?') ? '&' : '?'}${params.toString()}`;
    }

    async function fetchPaginatedJson(key, url, errorMessage, signal) {
        const state = getTablePageState(key);
        state.loading = true;
        try {
            const data = await apiRequest(addPaginationParams(url, key), { signal });
            const pagination = data.pagination || {};
            state.nextCursor = pagination.next_cursor || null;
            state.hasMore = Boolean(pagination.has_more);
            return data;
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
        return (data[field] || []).slice(0, pageSize);
    }

    function renderPaginationControls(key) {
        const containerId = pagerContainers[key];
        const container = containerId ? document.getElementById(containerId) : null;
        if (!container) return;
        const state = getTablePageState(key);
        replaceHtmlIfChanged(container, `
            <button class="btn btn-sm" data-action="table-page" data-table-key="${escapeAttribute(key)}" data-page-direction="prev" ${state.page <= 1 || state.loading ? 'disabled' : ''}>上一页</button>
            <span class="pagination-status">第 ${state.page} 页</span>
            <button class="btn btn-sm" data-action="table-page" data-table-key="${escapeAttribute(key)}" data-page-direction="next" ${!state.hasMore || state.loading ? 'disabled' : ''}>下一页</button>
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
        if (!pagerContainers[key]) return;
        await reloadTable(key);
    }

    return Object.freeze({
        addPaginationParams,
        fetchPaginatedJson,
        getTablePageSnapshot,
        goToTablePage,
        pageItems,
        renderPaginationControls,
        resetTablePage,
    });
}
