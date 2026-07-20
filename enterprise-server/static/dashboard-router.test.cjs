const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

function moduleUrl(source) {
    return `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`;
}

const stateSource = fs.readFileSync(
    path.join(__dirname, 'dashboard', 'state.js'),
    'utf8',
);
const routerSource = fs.readFileSync(
    path.join(__dirname, 'dashboard', 'router.js'),
    'utf8',
).replace("'./state.js'", JSON.stringify(moduleUrl(stateSource)));
const routerModulePromise = import(moduleUrl(routerSource));

function createHarness(href, isAdmin = false) {
    const location = { href };
    const calls = [];
    const history = {
        pushState(state, title, url) {
            calls.push({ method: 'pushState', state, title, url });
            location.href = new URL(url, location.href).href;
        },
        replaceState(state, title, url) {
            calls.push({ method: 'replaceState', state, title, url });
            location.href = new URL(url, location.href).href;
        },
    };
    return routerModulePromise.then(({ createDashboardRouter }) => ({
        calls,
        location,
        router: createDashboardRouter({ isAdmin, location, history }),
    }));
}

test('router fails closed for missing, empty, unknown, and restricted sections', async () => {
    for (const href of [
        'https://example.test/dashboard',
        'https://example.test/dashboard?section=',
        'https://example.test/dashboard?section=unknown',
        'https://example.test/dashboard?section=users',
    ]) {
        const { router } = await createHarness(href);
        assert.equal(router.dashboardSectionFromLocation(), 'overview');
    }

    const { router: memberRouter } = await createHarness(
        'https://example.test/dashboard?section=projects',
    );
    assert.equal(memberRouter.dashboardSectionFromLocation(), 'projects');
    assert.equal(memberRouter.canAccessDashboardSection('users'), false);

    const { router: adminRouter } = await createHarness(
        'https://example.test/dashboard?section=users',
        true,
    );
    assert.equal(adminRouter.dashboardSectionFromLocation(), 'users');
    assert.equal(adminRouter.canAccessDashboardSection('users'), true);
    assert.equal(adminRouter.canAccessDashboardSection('Users'), false);
});

test('router reads the requested section without trimming or normalizing it', async () => {
    const { router } = await createHarness(
        'https://example.test/dashboard?section=%20projects%20',
    );

    assert.equal(router.requestedDashboardSection(), ' projects ');
    assert.equal(router.dashboardSectionFromLocation(), 'overview');
});

test('router pushes non-default sections and preserves unrelated query parameters', async () => {
    const { calls, location, router } = await createHarness(
        'https://example.test/dashboard?range=30d#summary',
    );

    router.updateDashboardSectionUrl('projects');

    assert.deepEqual(calls, [{
        method: 'pushState',
        state: { section: 'projects' },
        title: '',
        url: '/dashboard?range=30d&section=projects',
    }]);
    assert.equal(
        location.href,
        'https://example.test/dashboard?range=30d&section=projects',
    );
});

test('router replaces with the default section by removing section and hash', async () => {
    const { calls, location, router } = await createHarness(
        'https://example.test/dashboard?range=7d&section=users#details',
    );

    router.updateDashboardSectionUrl('overview', true);

    assert.deepEqual(calls, [{
        method: 'replaceState',
        state: { section: 'overview' },
        title: '',
        url: '/dashboard?range=7d',
    }]);
    assert.equal(location.href, 'https://example.test/dashboard?range=7d');
});

test('reading a route never writes history or clears the current hash', async () => {
    const { calls, location, router } = await createHarness(
        'https://example.test/dashboard?section=overview#summary',
    );

    assert.equal(router.requestedDashboardSection(), 'overview');
    assert.equal(router.dashboardSectionFromLocation(), 'overview');
    assert.deepEqual(calls, []);
    assert.equal(
        location.href,
        'https://example.test/dashboard?section=overview#summary',
    );
});
