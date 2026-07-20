const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const dashboardSource = fs.readFileSync(
    path.join(__dirname, 'dashboard.js'),
    'utf8',
);

function createHarness(apiRequestImpl = async () => ({})) {
    const elements = new Map();
    const start = dashboardSource.indexOf('const OPTION_PAGE_LIMIT');
    const end = dashboardSource.indexOf('// Role-based UI', start);
    assert.notEqual(start, -1);
    assert.notEqual(end, -1);

    const context = vm.createContext({
        AbortController,
        Map,
        URLSearchParams,
        apiRequest: apiRequestImpl,
        clearTimeout,
        document: {
            getElementById(id) {
                return elements.get(id) || null;
            },
        },
        setTimeout,
    });
    vm.runInContext(`${dashboardSource.slice(start, end)}
globalThis.optionTestExports = {
    beginOptionRequest,
    boundedOptionUrl,
    cancelOptionRequests,
    fetchBoundedOptions,
    optionResultMessage,
    scheduleOptionSearch,
    setOptionStatus,
};`, context);
    return {
        ...context.optionTestExports,
        elements,
    };
}

test('bounded option requests fetch one page and preserve explicit overflow', async () => {
    const requestedUrls = [];
    const items = Array.from({ length: 101 }, (_, index) => ({ id: index }));
    const harness = createHarness(async url => {
        requestedUrls.push(url);
        return {
            departments: items,
            pagination: { has_more: true, next_cursor: 'unused' },
        };
    });

    const result = await harness.fetchBoundedOptions(
        '/api/admin/departments?org_id=org-1',
        'departments',
        '平台 团队',
    );

    assert.equal(requestedUrls.length, 1);
    assert.equal(
        requestedUrls[0],
        '/api/admin/departments?org_id=org-1&limit=100&q=%E5%B9%B3%E5%8F%B0+%E5%9B%A2%E9%98%9F',
    );
    assert.equal(result.items.length, 100);
    assert.equal(result.hasMore, true);
});

test('option result feedback distinguishes empty, bounded, and successful searches', () => {
    const harness = createHarness();

    assert.equal(harness.optionResultMessage('部门', 0, false, 'missing'), '未找到匹配部门');
    assert.match(
        harness.optionResultMessage('部门', 100, true, '平台'),
        /仅显示前 100 个，请继续输入关键词/,
    );
    assert.equal(harness.optionResultMessage('组织', 2, false, ''), '已加载 2 个组织');

    const status = { textContent: '', className: '' };
    harness.elements.set('status', status);
    harness.setOptionStatus('status', '正在加载部门...', 'loading');
    assert.equal(status.className, 'option-search-status loading');
    harness.setOptionStatus('status', '部门加载失败，请重试', 'error');
    assert.equal(status.className, 'option-search-status error');
});

test('a newly scheduled search aborts the in-flight option request', () => {
    const harness = createHarness();
    const controller = harness.beginOptionRequest('create-user-dept');

    harness.scheduleOptionSearch('create-user-dept', () => {});

    assert.equal(controller.signal.aborted, true);
    harness.cancelOptionRequests();
});
