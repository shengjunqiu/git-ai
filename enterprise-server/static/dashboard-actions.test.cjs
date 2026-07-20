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

function escapeAttributeSource() {
    const startMarker = 'function escapeAttribute';
    const endMarker = 'function fmtTimeAgo';
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
        toggleGitTrackingUser: (userId, checked) => {
            calls.push(['toggle-user', userId, checked]);
        },
        setGitTrackingUploadAuthorization: (userId, userName, authorized, element) => {
            calls.push(['set-authorization', userId, userName, authorized, element]);
        },
        showCreateApiKeyForUser: (userId, userName) => {
            calls.push(['show-user-api-key', userId, userName]);
        },
        deleteUser: (userId, userName) => calls.push(['delete-user', userId, userName]),
        showCreateDepartmentModal: () => calls.push(['show-create-department']),
        backDepartmentLevel: () => calls.push(['back-department']),
        showCreateApiKeyModal: () => calls.push(['show-create-api-key']),
        revokeApiKey: (keyId, keyName) => calls.push(['revoke-key', keyId, keyName]),
        renderSelectedReleaseFiles: () => calls.push(['render-release-files']),
        publishCliRelease: element => calls.push(['publish-release', element]),
        promoteCliRelease: (version, checksum) => {
            calls.push(['promote-release', version, checksum]);
        },
        renderSelectedManagedFile: () => calls.push(['render-managed-file']),
        uploadManagedFile: element => calls.push(['upload-managed-file', element]),
        copyPublishedUrl: pathValue => calls.push(['copy-url', pathValue]),
        publishManagedFileVersion: (slug, version) => {
            calls.push(['publish-file', slug, version]);
        },
        deleteManagedFileVersion: (slug, version) => {
            calls.push(['delete-file', slug, version]);
        },
        showEditManagedFileModal: (slug, name, description, isPublic) => {
            calls.push(['edit-file', slug, name, description, isPublic]);
        },
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

test('dynamic management actions preserve special characters from dataset values', () => {
    const harness = createHarness();
    const specialName = `研发 "A&B" <核心> '组'`;
    const authorization = actionEvent({
        action: 'set-git-tracking-authorization',
        userId: 'user-1',
        userName: specialName,
        authorized: 'true',
    });
    harness.handleDashboardAction(authorization.event);
    assert.deepEqual(harness.calls[0].slice(0, 4), [
        'set-authorization',
        'user-1',
        specialName,
        true,
    ]);
    assert.equal(harness.calls[0][4], authorization.actionElement);

    const editFile = actionEvent({
        action: 'show-edit-managed-file',
        fileSlug: 'company-config',
        fileName: specialName,
        fileDescription: `说明 "${specialName}"`,
        filePublic: 'false',
    });
    harness.handleDashboardAction(editFile.event);
    assert.deepEqual(harness.calls.at(-1), [
        'edit-file',
        'company-config',
        specialName,
        `说明 "${specialName}"`,
        false,
    ]);
});

test('dynamic action attributes escape HTML-significant characters', () => {
    const context = vm.createContext({});
    vm.runInContext(`${escapeAttributeSource()}
globalThis.escapeAttributeForTest = escapeAttribute;`, context);

    assert.equal(
        context.escapeAttributeForTest(`研发 "A&B" <核心> '组'`),
        '研发 &quot;A&amp;B&quot; &lt;核心&gt; &#39;组&#39;',
    );
    for (const marker of [
        'data-action="set-git-tracking-authorization"',
        'data-user-name="${actionName}"',
        'data-action="revoke-api-key"',
        'data-action="promote-cli-release"',
        'data-action="publish-managed-file-version"',
        'data-action="delete-managed-file-version"',
        'data-action="show-edit-managed-file"',
    ]) {
        assert.equal(dashboardSource.includes(marker), true, marker);
    }
    for (const legacyHandler of [
        'onclick="setGitTrackingUploadAuthorization',
        'onclick="showCreateApiKeyForUser',
        'onclick="deleteUser',
        'onclick="revokeApiKey',
        'onclick="promoteCliRelease',
        'onclick="publishManagedFileVersion',
        'onclick="deleteManagedFileVersion',
        'onclick="showEditManagedFileModal',
    ]) {
        assert.equal(dashboardSource.includes(legacyHandler), false, legacyHandler);
    }
});

test('dashboard action listeners are registered once per delegated event type', () => {
    const harness = createHarness();
    harness.initializeDashboardActions();

    assert.equal(harness.listeners.get('click'), harness.handleDashboardAction);
    assert.equal(harness.listeners.get('change'), harness.handleDashboardAction);
    assert.equal(harness.listeners.size, 2);
});
