const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const dashboardSource = fs.readFileSync(
    path.join(__dirname, 'dashboard.js'),
    'utf8',
);
const requestLayerSource = dashboardSource.split('// --- Auto refresh ---')[0];

function createHarness(fetchImpl) {
    let assignedUrl = null;
    const context = vm.createContext({
        AbortController,
        DOMException,
        Headers,
        Response,
        URL,
        clearTimeout,
        console,
        fetch: fetchImpl,
        setTimeout,
        window: {
            location: {
                hash: '#details',
                pathname: '/me',
                search: '?section=users',
                assign(url) {
                    assignedUrl = url;
                },
            },
        },
    });
    vm.runInContext(`${requestLayerSource}
globalThis.requestTestExports = {
    apiRequest,
    AbortError,
    AuthExpiredError,
    HttpError,
    InvalidResponseError,
    NetworkError,
    PermissionDeniedError,
    TimeoutError,
};`, context);
    return {
        ...context.requestTestExports,
        assignedUrl: () => assignedUrl,
    };
}

function jsonResponse(data, { status = 200, requestId = 'req-test' } = {}) {
    return new Response(JSON.stringify(data), {
        status,
        headers: {
            'content-type': 'application/json',
            'x-request-id': requestId,
        },
    });
}

test('adds Accept and parses a JSON success response', async () => {
    let accept = null;
    const harness = createHarness(async (_url, options) => {
        accept = options.headers.get('Accept');
        return jsonResponse({ ok: true });
    });

    assert.equal((await harness.apiRequest('/ok')).ok, true);
    assert.equal(accept, 'application/json');
});

test('401 preserves metadata and redirects back to the current page', async () => {
    const harness = createHarness(async () =>
        jsonResponse({ error: 'expired' }, { status: 401, requestId: 'req-401' }));

    await assert.rejects(
        harness.apiRequest('/private', { retries: 0 }),
        error => {
            assert.equal(error.name, 'AuthExpiredError');
            assert.equal(error.status, 401);
            assert.equal(error.requestId, 'req-401');
            return true;
        },
    );
    assert.equal(
        harness.assignedUrl(),
        '/auth/login?return_to=%2Fme%3Fsection%3Dusers%23details',
    );
});

test('403 is a typed permission error', async () => {
    const harness = createHarness(async () =>
        jsonResponse({ error: '仅管理员可操作' }, { status: 403 }));

    await assert.rejects(
        harness.apiRequest('/forbidden', { retries: 0 }),
        error => error.name === 'PermissionDeniedError'
            && error.message === '仅管理员可操作',
    );
});

test('GET retries one 429 but POST transport errors are not retried', async () => {
    let getAttempts = 0;
    const getHarness = createHarness(async () => {
        getAttempts += 1;
        return getAttempts === 1
            ? jsonResponse({ error: 'slow down' }, { status: 429 })
            : jsonResponse({ ok: true });
    });
    assert.equal((await getHarness.apiRequest('/retry')).ok, true);
    assert.equal(getAttempts, 2);

    let postAttempts = 0;
    const postHarness = createHarness(async () => {
        postAttempts += 1;
        throw new TypeError('offline');
    });
    await assert.rejects(
        postHarness.apiRequest('/mutation', { method: 'POST' }),
        error => error.name === 'NetworkError',
    );
    assert.equal(postAttempts, 1);
});

test('500 hides server details and keeps request ID', async () => {
    const harness = createHarness(async () =>
        jsonResponse(
            { error: 'database password and internal stack' },
            { status: 500, requestId: 'req-500' },
        ));

    await assert.rejects(
        harness.apiRequest('/broken', { retries: 0 }),
        error => {
            assert.equal(error.name, 'HttpError');
            assert.equal(error.message, '请求失败（HTTP 500）');
            assert.equal(error.requestId, 'req-500');
            return true;
        },
    );
});

test('HTML, malformed JSON, and empty success responses are typed invalid responses', async () => {
    for (const response of [
        new Response('<html>login</html>', {
            headers: { 'content-type': 'text/html' },
        }),
        new Response('{bad json', {
            headers: { 'content-type': 'application/json' },
        }),
        new Response('', {
            headers: { 'content-type': 'application/json' },
        }),
    ]) {
        const harness = createHarness(async () => response);
        await assert.rejects(
            harness.apiRequest('/invalid', { retries: 0 }),
            error => error.name === 'InvalidResponseError',
        );
    }
});

test('offline, timeout, and caller cancellation have distinct error types', async () => {
    const offlineHarness = createHarness(async () => {
        throw new TypeError('offline');
    });
    await assert.rejects(
        offlineHarness.apiRequest('/offline', { retries: 0 }),
        error => error.name === 'NetworkError',
    );

    const hangingFetch = (_url, options) => new Promise((_, reject) => {
        options.signal.addEventListener(
            'abort',
            () => reject(new DOMException('aborted', 'AbortError')),
            { once: true },
        );
    });
    const timeoutHarness = createHarness(hangingFetch);
    await assert.rejects(
        timeoutHarness.apiRequest('/slow', { retries: 0, timeoutMs: 5 }),
        error => error.name === 'TimeoutError',
    );

    const slowBodyHarness = createHarness(async (_url, options) => ({
        headers: new Headers({ 'content-type': 'application/json' }),
        ok: true,
        status: 200,
        text: () => new Promise((_, reject) => {
            options.signal.addEventListener(
                'abort',
                () => reject(new DOMException('aborted', 'AbortError')),
                { once: true },
            );
        }),
    }));
    await assert.rejects(
        slowBodyHarness.apiRequest('/slow-body', { retries: 0, timeoutMs: 5 }),
        error => error.name === 'TimeoutError',
    );

    const cancelHarness = createHarness(hangingFetch);
    const controller = new AbortController();
    const request = cancelHarness.apiRequest('/cancel', {
        retries: 0,
        signal: controller.signal,
    });
    controller.abort();
    await assert.rejects(request, error => error.name === 'AbortError');
});
