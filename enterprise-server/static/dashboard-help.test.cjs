const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const dashboardSource = fs.readFileSync(
    path.join(__dirname, 'dashboard.js'),
    'utf8',
);

function lazyHelpSource() {
    const startMarker = '// --- Lazy help content ---';
    const endMarker = '// --- Time range helper ---';
    const start = dashboardSource.indexOf(startMarker);
    const end = dashboardSource.indexOf(endMarker, start);
    assert.notEqual(start, -1, `missing source marker: ${startMarker}`);
    assert.notEqual(end, -1, `missing source marker: ${endMarker}`);
    return dashboardSource.slice(start, end);
}

function createHarness(response = { html: '<article id="help-install">帮助内容</article>' }) {
    let requests = 0;
    let writes = 0;
    let scrolls = 0;
    const attributes = new Map([['aria-busy', 'true']]);
    const helpContent = {
        dataset: {},
        innerHTML: '<p>正在加载...</p>',
        removeAttribute(name) {
            attributes.delete(name);
        },
        setAttribute(name, value) {
            attributes.set(name, value);
        },
    };
    const helpTarget = {
        scrollIntoView() {
            scrolls += 1;
        },
    };
    const window = { location: { hash: '' } };
    const context = vm.createContext({
        InvalidResponseError: class InvalidResponseError extends Error {
            constructor(message) {
                super(message);
                this.name = 'InvalidResponseError';
            }
        },
        apiRequest: async () => {
            requests += 1;
            return response;
        },
        document: {
            getElementById(id) {
                if (id === 'help-content') return helpContent;
                if (id === 'help-install') return helpTarget;
                return null;
            },
        },
        isSilentRefresh: options => options?.mode === 'auto',
        replaceHtmlIfChanged(element, nextHtml) {
            if (element.innerHTML === nextHtml) return false;
            writes += 1;
            element.innerHTML = nextHtml;
            return true;
        },
        requestAnimationFrame(callback) {
            callback();
        },
        window,
    });

    vm.runInContext(`${lazyHelpSource()}
globalThis.helpTestExports = { loadHelp };`, context);

    return {
        attributes,
        helpContent,
        loadHelp: context.helpTestExports.loadHelp,
        requests: () => requests,
        scrolls: () => scrolls,
        window,
        writes: () => writes,
    };
}

test('help content is fetched and committed only once', async () => {
    const harness = createHarness();

    await harness.loadHelp({ mode: 'initial' });
    await harness.loadHelp({ mode: 'manual' });

    assert.equal(harness.requests(), 1);
    assert.equal(harness.writes(), 1);
    assert.equal(harness.helpContent.dataset.loaded, 'true');
    assert.equal(harness.attributes.has('aria-busy'), false);
    assert.match(harness.helpContent.innerHTML, /帮助内容/);
});

test('invalid help payload keeps the lazy container retryable', async () => {
    const harness = createHarness({ html: '' });

    await assert.rejects(
        harness.loadHelp({ mode: 'initial' }),
        error => error.name === 'InvalidResponseError',
    );

    assert.equal(harness.helpContent.dataset.loaded, undefined);
    assert.equal(harness.attributes.get('aria-busy'), 'true');
});

test('help deep links scroll after loading but not during background refresh', async () => {
    const harness = createHarness();
    harness.window.location.hash = '#help-install';

    await harness.loadHelp({ mode: 'initial' });
    await harness.loadHelp({ mode: 'auto' });
    assert.equal(harness.scrolls(), 1);

    await harness.loadHelp({ mode: 'manual' });
    assert.equal(harness.scrolls(), 2);
});
