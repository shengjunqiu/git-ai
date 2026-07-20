const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const toastSource = fs.readFileSync(
    path.join(__dirname, 'dashboard', 'ui', 'toast.js'),
    'utf8',
);
const toastModulePromise = import(
    `data:text/javascript;base64,${Buffer.from(toastSource).toString('base64')}`
);

function createHarness() {
    let currentToast = null;
    let nextTimer = 1;
    const timers = new Map();
    const clearedTimers = [];
    const document = {
        body: {
            appendChild(element) {
                currentToast = element;
                element.isConnected = true;
            },
        },
        createElement(tagName) {
            const attributes = new Map();
            return {
                attributes,
                className: '',
                isConnected: false,
                tagName,
                textContent: '',
                remove() {
                    this.isConnected = false;
                    if (currentToast === this) currentToast = null;
                },
                setAttribute(name, value) {
                    attributes.set(name, value);
                },
            };
        },
        querySelector(selector) {
            return selector === '.toast' ? currentToast : null;
        },
    };
    return toastModulePromise.then(({ createToast }) => ({
        clearedTimers,
        currentToast: () => currentToast,
        document,
        runTimer(handle) {
            const callback = timers.get(handle);
            timers.delete(handle);
            callback();
        },
        timers,
        toast: createToast({
            document,
            setTimeoutImpl(callback, delay) {
                const handle = nextTimer;
                nextTimer += 1;
                timers.set(handle, callback);
                assert.equal(delay, 3000);
                return handle;
            },
            clearTimeoutImpl(handle) {
                clearedTimers.push(handle);
                timers.delete(handle);
            },
        }),
    }));
}

test('toast renders untrusted messages as text with accessible semantics', async () => {
    const harness = await createHarness();

    harness.toast.showToast('<img src=x onerror=alert(1)>', 'error');

    const element = harness.currentToast();
    assert.equal(element.className, 'toast error');
    assert.equal(element.textContent, '<img src=x onerror=alert(1)>');
    assert.equal(element.attributes.get('role'), 'alert');
    assert.equal(element.attributes.get('aria-live'), 'assertive');
    assert.equal(element.attributes.get('aria-atomic'), 'true');
});

test('success and invalid types use status semantics and a safe class', async () => {
    const harness = await createHarness();

    harness.toast.showToast('完成', 'success');
    assert.equal(harness.currentToast().className, 'toast success');
    assert.equal(harness.currentToast().attributes.get('role'), 'status');
    assert.equal(harness.currentToast().attributes.get('aria-live'), 'polite');

    harness.toast.showToast('未知', 'custom injected');
    assert.equal(harness.currentToast().className, 'toast info');
});

test('a new toast removes the previous toast and cancels its timer', async () => {
    const harness = await createHarness();

    harness.toast.showToast('第一条');
    const first = harness.currentToast();
    harness.toast.showToast('第二条');

    assert.equal(first.isConnected, false);
    assert.equal(harness.currentToast().textContent, '第二条');
    assert.deepEqual(harness.clearedTimers, [1]);
    assert.deepEqual(Array.from(harness.timers.keys()), [2]);
});

test('showing a toast removes a pre-existing toast outside the module', async () => {
    const harness = await createHarness();
    const existing = harness.document.createElement('div');
    existing.className = 'toast info';
    harness.document.body.appendChild(existing);

    harness.toast.showToast('模块提示');

    assert.equal(existing.isConnected, false);
    assert.equal(harness.currentToast().textContent, '模块提示');
});

test('timer and explicit dismissal remove only the active toast', async () => {
    const harness = await createHarness();

    harness.toast.showToast('自动关闭');
    harness.runTimer(1);
    assert.equal(harness.currentToast(), null);

    harness.toast.showToast('手动关闭');
    harness.toast.dismissToast();
    assert.equal(harness.currentToast(), null);
    assert.deepEqual(harness.clearedTimers, [2]);
    assert.equal(harness.timers.size, 0);
});
