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
const stateSource = fs.readFileSync(
    path.join(__dirname, 'dashboard', 'state.js'),
    'utf8',
);
const renderModulePromise = import(
    `data:text/javascript;base64,${Buffer.from(renderSource).toString('base64')}`
);
const stateModulePromise = import(
    `data:text/javascript;base64,${Buffer.from(stateSource).toString('base64')}`
);

function sourceBetween(startMarker, endMarker) {
    const start = dashboardSource.indexOf(startMarker);
    const end = dashboardSource.indexOf(endMarker, start);
    assert.notEqual(start, -1, `missing source marker: ${startMarker}`);
    assert.notEqual(end, -1, `missing source marker: ${endMarker}`);
    return dashboardSource.slice(start, end);
}

async function createHarness() {
    const [renderModule, stateModule] = await Promise.all([
        renderModulePromise,
        stateModulePromise,
    ]);
    const elements = new Map();
    const context = vm.createContext({
        AbortController,
        JSON,
        Map,
        Promise,
        Set,
        WeakMap,
        RefreshMode: stateModule.RefreshMode,
        replaceHtmlIfChanged: renderModule.replaceHtmlIfChanged,
        document: {
            getElementById(id) {
                return elements.get(id) || null;
            },
        },
    });
    const refreshSource = sourceBetween(
        'function isSilentRefresh',
        'function getTablePageState',
    );
    const tableSource = sourceBetween(
        'function setTableLoading',
        'function renderPaginationControls',
    );
    const chartSource = sourceBetween(
        'const chartDataSignatures = new WeakMap();',
        'function createOverviewTrendChart',
    );
    vm.runInContext(`${refreshSource}
${tableSource}
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
