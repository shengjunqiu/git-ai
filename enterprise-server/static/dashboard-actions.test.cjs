const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const html = fs.readFileSync(path.join(__dirname, 'dashboard.html'), 'utf8');
const dashboardSource = fs.readFileSync(
    path.join(__dirname, 'dashboard.js'),
    'utf8',
);

function actionSource() {
    const startMarker = 'function dashboardActionElement';
    const endMarker = "window.addEventListener('popstate'";
    const start = dashboardSource.indexOf(startMarker);
    const end = dashboardSource.indexOf(endMarker, start);
    assert.notEqual(start, -1, `missing source marker: ${startMarker}`);
    assert.notEqual(end, -1, `missing source marker: ${endMarker}`);
    return dashboardSource.slice(start, end);
}

function createHarness() {
    const calls = [];
    const listeners = new Map();
    const context = vm.createContext({
        document: {
            addEventListener(type, listener) {
                listeners.set(type, listener);
            },
        },
        window: { location: { href: '' } },
        canAccessDashboardSection: section => section !== 'forbidden',
        activateDashboardSection: (section, options) => {
            calls.push(['navigate', section, options]);
        },
        closeMobileNavigation: () => calls.push(['close-navigation']),
        refreshCurrentSection: () => calls.push(['refresh']),
        goToTablePage: (key, direction) => calls.push(['table-page', key, direction]),
        changeDeveloperSorting: () => calls.push(['developer-sorting']),
        showCreateUserModal: () => calls.push(['show-create-user']),
        bulkAuthorizeGitTrackingUpload: element => calls.push(['bulk-authorize', element]),
        toggleAllGitTrackingUsers: checked => calls.push(['toggle-all', checked]),
        showCreateDepartmentModal: () => calls.push(['show-create-department']),
        backDepartmentLevel: () => calls.push(['back-department']),
        showCreateApiKeyModal: () => calls.push(['show-create-api-key']),
        renderSelectedReleaseFiles: () => calls.push(['render-release-files']),
        publishCliRelease: element => calls.push(['publish-release', element]),
        renderSelectedManagedFile: () => calls.push(['render-managed-file']),
        uploadManagedFile: element => calls.push(['upload-managed-file', element]),
    });
    vm.runInContext(`${actionSource()}
globalThis.actionTestExports = {
    handleDashboardAction,
    initializeDashboardActions,
};`, context);
    return {
        ...context.actionTestExports,
        calls,
        context,
        listeners,
    };
}

function actionEvent(dataset, extra = {}) {
    const actionElement = { dataset, ...extra };
    const target = { closest: () => actionElement };
    return {
        actionElement,
        event: {
            type: dataset.actionEvent || 'click',
            target,
            preventDefault() {
                this.defaultPrevented = true;
            },
            defaultPrevented: false,
        },
    };
}

test('static navigation, refresh, and pagination use delegated data actions', () => {
    assert.doesNotMatch(
        html,
        /data-section="[^"]+"\s+onclick=/,
    );
    assert.match(
        html,
        /data-action="navigate-section" data-section="overview"/,
    );
    assert.match(
        dashboardSource,
        /data-action="table-page" data-table-key="\$\{escapeAttribute\(key\)\}"/,
    );
    assert.doesNotMatch(
        dashboardSource,
        /onclick="goToTablePage/,
    );
});

test('delegated dispatcher reads action arguments from the action element dataset', () => {
    const harness = createHarness();
    const navigation = actionEvent({
        action: 'navigate-section',
        section: 'projects',
    });
    harness.handleDashboardAction(navigation.event);
    assert.equal(navigation.event.defaultPrevented, true);
    assert.equal(harness.calls[0][0], 'navigate');
    assert.equal(harness.calls[0][1], 'projects');
    assert.equal(harness.calls[0][2].updateUrl, true);
    assert.deepEqual(harness.calls[1], ['close-navigation']);

    const pagination = actionEvent({
        action: 'table-page',
        tableKey: 'developers',
        pageDirection: 'next',
    });
    harness.handleDashboardAction(pagination.event);
    assert.deepEqual(
        harness.calls.at(-1),
        ['table-page', 'developers', 'next'],
    );

    const selection = actionEvent(
        {
            action: 'toggle-all-git-tracking-users',
            actionEvent: 'change',
        },
        { checked: true },
    );
    harness.handleDashboardAction(selection.event);
    assert.deepEqual(harness.calls.at(-1), ['toggle-all', true]);
});

test('change-only controls ignore click events before their value changes', () => {
    const harness = createHarness();
    const selection = actionEvent(
        {
            action: 'toggle-all-git-tracking-users',
            actionEvent: 'change',
        },
        { checked: false },
    );
    selection.event.type = 'click';
    harness.handleDashboardAction(selection.event);
    assert.deepEqual(harness.calls, []);

    selection.event.type = 'change';
    selection.actionElement.checked = true;
    harness.handleDashboardAction(selection.event);
    assert.deepEqual(harness.calls, [['toggle-all', true]]);
});

test('dashboard action listeners are registered once per delegated event type', () => {
    const harness = createHarness();
    harness.initializeDashboardActions();

    assert.equal(harness.listeners.get('click'), harness.handleDashboardAction);
    assert.equal(harness.listeners.get('change'), harness.handleDashboardAction);
    assert.equal(harness.listeners.size, 2);
});
