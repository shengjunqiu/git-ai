const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const dashboardSource = fs.readFileSync(
    path.join(__dirname, 'dashboard.js'),
    'utf8',
);
const renderSource = fs.readFileSync(
    path.join(__dirname, 'dashboard', 'render.js'),
    'utf8',
);
const refreshSource = fs.readFileSync(
    path.join(__dirname, 'dashboard', 'refresh.js'),
    'utf8',
);
const renderModulePromise = import(
    `data:text/javascript;base64,${Buffer.from(renderSource).toString('base64')}`
);
const refreshModulePromise = import(
    `data:text/javascript;base64,${Buffer.from(refreshSource).toString('base64')}`
);

function sourceBetween(startMarker, endMarker) {
    const start = dashboardSource.indexOf(startMarker);
    const end = dashboardSource.indexOf(endMarker, start);
    assert.notEqual(start, -1, `missing source marker: ${startMarker}`);
    assert.notEqual(end, -1, `missing source marker: ${endMarker}`);
    return dashboardSource.slice(start, end);
}

async function createHarness() {
    const [renderModule, refreshModule] = await Promise.all([
        renderModulePromise,
        refreshModulePromise,
    ]);
    const elements = new Map();
    const context = vm.createContext({
        AbortController,
        JSON,
        Map,
        Promise,
        Set,
        WeakMap,
        RefreshMode: refreshModule.RefreshMode,
        isSilentRefresh: refreshModule.isSilentRefresh,
        refreshCollisionAction: refreshModule.refreshCollisionAction,
        replaceHtmlIfChanged: renderModule.replaceHtmlIfChanged,
        document: {
            getElementById(id) {
                return elements.get(id) || null;
            },
        },
    });
    const tableSource = sourceBetween(
        'function setTableLoading',
        'const OPTION_PAGE_LIMIT',
    );
    const chartSource = sourceBetween(
        'const chartDataSignatures = new WeakMap();',
        'function createOverviewTrendChart',
    );
    vm.runInContext(`${tableSource}
${chartSource}
globalThis.refreshTestExports = {
    RefreshMode,
    isSilentRefresh,
    refreshCollisionAction,
    replaceHtmlIfChanged,
    setTableLoading,
    rememberChartData,
    updateChartDataIfChanged,
};`, context);
    return {
        ...context.refreshTestExports,
        elements,
    };
}

async function createRefreshHarness(overrides = {}) {
    const { createDashboardRefresh } = await refreshModulePromise;
    let section = 'projects';
    let hidden = false;
    const errors = [];
    const statuses = [];
    const successes = [];
    const refresh = createDashboardRefresh({
        currentSection: () => section,
        isPageHidden: () => hidden,
        loadSectionData: async () => {},
        afterSectionSuccess: async () => {},
        onSectionSuccess: id => successes.push(id),
        onSectionError: (id, error, options) => errors.push({ id, error, options }),
        onStatusChange: status => statuses.push(status),
        ...overrides,
    });
    return {
        errors,
        refresh,
        setHidden(value) {
            hidden = value;
        },
        setSection(value) {
            section = value;
        },
        statuses,
        successes,
    };
}

function createHtmlElement(initialHtml = '') {
    let innerHTML = initialHtml;
    let writes = 0;
    return {
        get innerHTML() {
            return innerHTML;
        },
        set innerHTML(value) {
            writes += 1;
            innerHTML = value;
        },
        cloneNode() {
            return createHtmlElement();
        },
        writes() {
            return writes;
        },
    };
}

test('AUTO skips loading while INITIAL and MANUAL retain loading feedback', async () => {
    const harness = await createHarness();
    const autoElement = createHtmlElement('<tr><td>已有数据</td></tr>');
    harness.elements.set('auto-table', autoElement);
    harness.setTableLoading('auto-table', 2, { mode: harness.RefreshMode.AUTO });
    assert.equal(autoElement.writes(), 0);
    assert.equal(autoElement.innerHTML, '<tr><td>已有数据</td></tr>');

    for (const mode of [harness.RefreshMode.INITIAL, harness.RefreshMode.MANUAL]) {
        const element = createHtmlElement('<tr><td>已有数据</td></tr>');
        harness.elements.set(`${mode}-table`, element);
        harness.setTableLoading(`${mode}-table`, 2, { mode });
        assert.equal(element.writes(), 1);
        assert.match(element.innerHTML, /加载中/);
    }
});

test('HTML replacement writes once only when normalized content changes', async () => {
    const harness = await createHarness();
    const element = createHtmlElement('<tr><td>稳定内容</td></tr>');

    assert.equal(
        harness.replaceHtmlIfChanged(element, '<tr><td>稳定内容</td></tr>'),
        false,
    );
    assert.equal(element.writes(), 0);

    assert.equal(
        harness.replaceHtmlIfChanged(element, '<tr><td>新内容</td></tr>'),
        true,
    );
    assert.equal(
        harness.replaceHtmlIfChanged(element, '<tr><td>新内容</td></tr>'),
        false,
    );
    assert.equal(element.writes(), 1);
});

test('refresh collision policy skips AUTO and queues one MANUAL refresh', async () => {
    const harness = await createHarness();

    assert.equal(
        harness.refreshCollisionAction(harness.RefreshMode.AUTO, true),
        'skip',
    );
    assert.equal(
        harness.refreshCollisionAction(harness.RefreshMode.MANUAL, true),
        'queue',
    );
    assert.equal(
        harness.refreshCollisionAction(harness.RefreshMode.INITIAL, true),
        'replace',
    );
    assert.equal(
        harness.refreshCollisionAction(harness.RefreshMode.AUTO, false),
        'start',
    );
});

test('refresh lifecycle preserves attempt, loader, clear, client, and success order', async () => {
    const { RefreshMode } = await refreshModulePromise;
    const events = [];
    const timestamps = [
        new Date('2026-07-20T10:00:00Z'),
        new Date('2026-07-20T10:00:01Z'),
    ];
    const harness = await createRefreshHarness({
        now: () => timestamps.shift(),
        loadSectionData: async (id, { mode, signal }) => {
            events.push(['load', id, mode, signal.aborted]);
        },
        onSectionSuccess: id => events.push(['clear-error', id]),
        afterSectionSuccess: async (id, { mode }) => {
            events.push(['client-status', id, mode]);
        },
        onStatusChange: status => events.push(['status', status]),
    });

    assert.equal(
        await harness.refresh.loadSection('projects', { mode: RefreshMode.INITIAL }),
        true,
    );
    assert.deepEqual(events.map(event => event[0]), [
        'status',
        'load',
        'clear-error',
        'client-status',
        'status',
    ]);
    assert.equal(
        harness.refresh.getRefreshSnapshot().lastRefreshAttemptAt.toISOString(),
        '2026-07-20T10:00:00.000Z',
    );
    assert.equal(
        harness.refresh.getRefreshSnapshot().lastRefreshSuccessAt.toISOString(),
        '2026-07-20T10:00:01.000Z',
    );
});

test('AUTO skips while MANUAL queues exactly one follow-up refresh', async () => {
    const { RefreshMode } = await refreshModulePromise;
    const pendingLoads = [];
    const harness = await createRefreshHarness({
        loadSectionData: () => new Promise(resolve => pendingLoads.push(resolve)),
    });

    const automatic = harness.refresh.loadSection('projects', { mode: RefreshMode.AUTO });
    assert.equal(
        await harness.refresh.loadSection('projects', { mode: RefreshMode.AUTO }),
        false,
    );
    const firstManual = harness.refresh.loadSection('projects', { mode: RefreshMode.MANUAL });
    const secondManual = harness.refresh.loadSection('projects', { mode: RefreshMode.MANUAL });
    assert.equal(firstManual, secondManual);
    assert.equal(pendingLoads.length, 1);

    pendingLoads.shift()();
    await automatic;
    await Promise.resolve();
    assert.equal(pendingLoads.length, 1);
    pendingLoads.shift()();
    assert.equal(await firstManual, true);
});

test('INITIAL aborts and replaces an in-flight section refresh', async () => {
    const { RefreshMode } = await refreshModulePromise;
    const pendingLoads = [];
    const signals = [];
    const harness = await createRefreshHarness({
        loadSectionData: (_id, { signal }) => {
            signals.push(signal);
            return new Promise(resolve => pendingLoads.push(resolve));
        },
    });

    const first = harness.refresh.loadSection('projects', { mode: RefreshMode.AUTO });
    const replacement = harness.refresh.loadSection('projects', {
        mode: RefreshMode.INITIAL,
    });
    assert.equal(signals[0].aborted, true);
    assert.equal(signals[1].aborted, false);

    pendingLoads[0]();
    pendingLoads[1]();
    assert.equal(await first, false);
    assert.equal(await replacement, true);
    assert.deepEqual(harness.successes, ['projects']);
});

test('failed AUTO refresh is background only after a successful load', async () => {
    const { RefreshMode } = await refreshModulePromise;
    const failure = new Error('offline');
    let calls = 0;
    const harness = await createRefreshHarness({
        loadSectionData: async () => {
            calls += 1;
            if (calls > 1) throw failure;
        },
    });

    assert.equal(await harness.refresh.loadSection('projects'), true);
    assert.equal(
        await harness.refresh.loadSection('projects', { mode: RefreshMode.AUTO }),
        false,
    );
    assert.deepEqual(harness.errors, [{
        id: 'projects',
        error: failure,
        options: { background: true },
    }]);
});

test('auto refresh and visibility callbacks only load a visible page', async () => {
    let intervalCallback;
    const cleared = [];
    const loads = [];
    const harness = await createRefreshHarness({
        loadSectionData: async id => loads.push(id),
        setIntervalImpl(callback, delay) {
            intervalCallback = callback;
            assert.equal(delay, 60000);
            return 7;
        },
        clearIntervalImpl: handle => cleared.push(handle),
    });

    harness.refresh.startAutoRefresh();
    harness.setHidden(true);
    intervalCallback();
    harness.refresh.handleVisibilityChange();
    await Promise.resolve();
    assert.deepEqual(loads, []);

    harness.setHidden(false);
    intervalCallback();
    await Promise.resolve();
    assert.deepEqual(loads, ['projects']);
    harness.refresh.stopAutoRefresh();
    assert.deepEqual(cleared, [7]);
});

test('unchanged chart data keeps the instance idle and AUTO updates without animation', async () => {
    const harness = await createHarness();
    const updates = [];
    const chart = {
        data: {
            labels: ['周一'],
            datasets: [{ label: 'AI', data: [10] }],
        },
        update(mode) {
            updates.push(mode);
        },
    };
    const originalChart = chart;
    harness.rememberChartData(chart, chart.data.labels, chart.data.datasets);

    assert.equal(
        harness.updateChartDataIfChanged(
            chart,
            ['周一'],
            [{ label: 'AI', data: [10] }],
            { mode: harness.RefreshMode.AUTO },
        ),
        false,
    );
    assert.equal(chart, originalChart);
    assert.deepEqual(updates, []);

    assert.equal(
        harness.updateChartDataIfChanged(
            chart,
            ['周一', '周二'],
            [{ label: 'AI', data: [10, 12] }],
            { mode: harness.RefreshMode.AUTO },
        ),
        true,
    );
    assert.equal(chart, originalChart);
    assert.deepEqual(updates, ['none']);
});
