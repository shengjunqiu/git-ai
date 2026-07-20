const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

function moduleUrl(source) {
    return `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`;
}

const apiSource = fs.readFileSync(
    path.join(__dirname, 'dashboard', 'api.js'),
    'utf8',
);
const renderSource = fs.readFileSync(
    path.join(__dirname, 'dashboard', 'render.js'),
    'utf8',
);
const apiUrl = moduleUrl(apiSource);
const paginationSource = fs.readFileSync(
    path.join(__dirname, 'dashboard', 'pagination.js'),
    'utf8',
)
    .replace("'./api.js'", JSON.stringify(apiUrl))
    .replace("'./render.js'", JSON.stringify(moduleUrl(renderSource)));
const paginationModulePromise = import(moduleUrl(paginationSource));
const apiModulePromise = import(apiUrl);

function createElement() {
    let innerHTML = '';
    return {
        get innerHTML() {
            return innerHTML;
        },
        set innerHTML(value) {
            innerHTML = value;
        },
        cloneNode() {
            return createElement();
        },
    };
}

async function createHarness(apiRequest, options = {}) {
    const { createDashboardPagination } = await paginationModulePromise;
    const reloads = [];
    const elements = new Map();
    const document = {
        getElementById(id) {
            if (!elements.has(id)) elements.set(id, createElement());
            return elements.get(id);
        },
    };
    const pagination = createDashboardPagination({
        apiRequest,
        document,
        reloadTable: async key => {
            reloads.push(key);
        },
        ...options,
    });
    return { document, elements, pagination, reloads };
}

test('pagination starts on page one and adds a bounded limit', async () => {
    const { pagination } = await createHarness(async () => ({}));

    assert.deepEqual(pagination.getTablePageSnapshot('projects'), {
        page: 1,
        cursor: null,
        nextCursor: null,
        hasMore: false,
        loading: false,
    });
    assert.equal(
        pagination.addPaginationParams('/api/projects?range=30d', 'projects'),
        '/api/projects?range=30d&limit=25',
    );
    assert.deepEqual(
        pagination.pageItems({ projects: Array.from({ length: 30 }, (_, id) => id) }, 'projects'),
        Array.from({ length: 25 }, (_, id) => id),
    );
});

test('next and previous transitions maintain the cursor stack', async () => {
    const responses = [
        { projects: [1], pagination: { next_cursor: 'cursor-2', has_more: true } },
        { projects: [2], pagination: { next_cursor: 'cursor-3', has_more: true } },
        { projects: [2], pagination: { next_cursor: 'cursor-new-3', has_more: true } },
    ];
    const urls = [];
    const { pagination, reloads } = await createHarness(async url => {
        urls.push(url);
        return responses.shift();
    });

    await pagination.fetchPaginatedJson('projects', '/api/projects', '加载失败');
    await pagination.goToTablePage('projects', 'next');
    assert.equal(
        pagination.addPaginationParams('/api/projects', 'projects'),
        '/api/projects?limit=25&cursor=cursor-2',
    );
    await pagination.fetchPaginatedJson('projects', '/api/projects', '加载失败');
    await pagination.goToTablePage('projects', 'next');
    await pagination.goToTablePage('projects', 'prev');
    await pagination.fetchPaginatedJson('projects', '/api/projects', '加载失败');
    await pagination.goToTablePage('projects', 'next');

    assert.deepEqual(reloads, ['projects', 'projects', 'projects', 'projects']);
    assert.deepEqual(urls, [
        '/api/projects?limit=25',
        '/api/projects?limit=25&cursor=cursor-2',
        '/api/projects?limit=25&cursor=cursor-2',
    ]);
    assert.equal(
        pagination.getTablePageSnapshot('projects').cursor,
        'cursor-new-3',
    );
    pagination.resetTablePage('projects');
    assert.deepEqual(pagination.getTablePageSnapshot('projects'), {
        page: 1,
        cursor: null,
        nextCursor: null,
        hasMore: false,
        loading: false,
    });
});

test('loading and boundary guards do not trigger table reloads', async () => {
    let resolveRequest;
    const request = new Promise(resolve => {
        resolveRequest = resolve;
    });
    const { pagination, reloads } = await createHarness(() => request);

    await pagination.goToTablePage('projects', 'prev');
    const pending = pagination.fetchPaginatedJson('projects', '/api/projects', '加载失败');
    assert.equal(pagination.getTablePageSnapshot('projects').loading, true);
    await pagination.goToTablePage('projects', 'next');
    resolveRequest({ pagination: { next_cursor: null, has_more: true } });
    await pending;
    await pagination.goToTablePage('projects', 'next');

    assert.deepEqual(reloads, []);
    assert.equal(pagination.getTablePageSnapshot('projects').loading, false);
});

test('pagination renderer reflects page, loading, and next-page availability', async () => {
    let resolveRequest;
    const request = new Promise(resolve => {
        resolveRequest = resolve;
    });
    const { elements, pagination } = await createHarness(() => request);

    pagination.renderPaginationControls('projects');
    const container = elements.get('proj-pagination');
    assert.match(container.innerHTML, /第 1 页/);
    assert.equal((container.innerHTML.match(/disabled/g) || []).length, 2);

    const pending = pagination.fetchPaginatedJson('projects', '/api/projects', '加载失败');
    pagination.renderPaginationControls('projects');
    assert.equal((container.innerHTML.match(/disabled/g) || []).length, 2);
    resolveRequest({ pagination: { next_cursor: 'cursor-2', has_more: true } });
    await pending;
    pagination.renderPaginationControls('projects');
    assert.equal((container.innerHTML.match(/disabled/g) || []).length, 1);
});

test('pagination preserves typed API errors and wraps unexpected failures', async () => {
    const { InvalidResponseError, NetworkError } = await apiModulePromise;
    const typedError = new NetworkError('离线');
    const typed = await createHarness(async () => {
        throw typedError;
    });
    await assert.rejects(
        typed.pagination.fetchPaginatedJson('projects', '/api/projects', '加载失败'),
        error => error === typedError,
    );

    const cause = new TypeError('invalid payload');
    const unexpected = await createHarness(async () => {
        throw cause;
    });
    await assert.rejects(
        unexpected.pagination.fetchPaginatedJson('projects', '/api/projects', '加载失败'),
        error => error instanceof InvalidResponseError
            && error.message === '加载失败'
            && error.cause === cause,
    );
    assert.equal(
        unexpected.pagination.getTablePageSnapshot('projects').loading,
        false,
    );
});
