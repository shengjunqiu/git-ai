const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const html = fs.readFileSync(path.join(__dirname, 'dashboard.html'), 'utf8');
const dashboardCss = fs.readFileSync(
    path.join(__dirname, 'dashboard.css'),
    'utf8',
);
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

function bootstrapSource() {
    const startMarker = 'function readDashboardBootstrap';
    const endMarker = 'const dashboardBootstrap';
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
        showDeveloperGitInfo: developerId => calls.push(['developer-git-info', developerId]),
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
        openDepartmentLevel: departmentId => calls.push(['open-department', departmentId]),
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
        closeModal: () => calls.push(['close-modal']),
        createUser: () => calls.push(['create-user']),
        createDepartment: () => calls.push(['create-department']),
        copyKey: () => calls.push(['copy-api-key']),
        createApiKey: () => calls.push(['create-api-key']),
        createApiKeyForUser: () => calls.push(['create-api-key-for-user']),
        saveManagedFileSettings: () => calls.push(['save-file-settings']),
        copyHelpCommand: element => calls.push(['copy-help-command', element]),
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

test('modal backdrop closes only when the overlay itself is clicked', () => {
    const harness = createHarness();
    const overlay = {
        dataset: { action: 'close-modal-backdrop' },
        closest() {
            return this;
        },
    };
    harness.handleDashboardAction({
        type: 'click',
        target: {
            closest: () => overlay,
        },
    });
    assert.deepEqual(harness.calls, []);

    harness.handleDashboardAction({
        type: 'click',
        target: overlay,
    });
    assert.deepEqual(harness.calls, [['close-modal']]);
});

test('developer, department, modal, and help actions use the delegated dispatcher', () => {
    const harness = createHarness();
    const developer = actionEvent({
        action: 'show-developer-git-info',
        developerId: 'developer@example.com',
    });
    harness.handleDashboardAction(developer.event);
    assert.deepEqual(
        harness.calls.at(-1),
        ['developer-git-info', 'developer@example.com'],
    );

    const department = actionEvent({
        action: 'open-department-level',
        departmentId: 'department-1',
    });
    harness.handleDashboardAction(department.event);
    assert.deepEqual(
        harness.calls.at(-1),
        ['open-department', 'department-1'],
    );

    const help = actionEvent({ action: 'copy-help-command' });
    harness.handleDashboardAction(help.event);
    assert.deepEqual(
        harness.calls.at(-1),
        ['copy-help-command', help.actionElement],
    );
});

test('dashboard templates contain no inline event handlers', () => {
    const inlineHandler = /\bon(?:click|change|submit|input|keydown|keyup|focus|blur|load|error)\s*=/;
    assert.doesNotMatch(html, inlineHandler);
    assert.doesNotMatch(dashboardSource, inlineHandler);
    assert.equal(dashboardSource.includes('function jsString'), false);
    assert.equal(
        (html.match(/data-action="copy-help-command"/g) || []).length,
        30,
    );
});

test('dashboard role uses inert bootstrap JSON and fails closed', () => {
    assert.match(
        html,
        /<script type="application\/json" id="dashboard-bootstrap">__GITAI_DASHBOARD_BOOTSTRAP__<\/script>/,
    );
    assert.doesNotMatch(html, /const\s+isAdmin\s*=/);
    assert.match(html, /<body class="__GITAI_DASHBOARD_ROLE_CLASS__">/);
    assert.match(
        dashboardCss,
        /\.dashboard-role-member \.admin-only,\s*\.dashboard-role-admin \.member-only\s*\{\s*display:\s*none\s*!important;\s*\}/,
    );
    for (const id of [
        'org-nav-item',
        'admin-nav-section',
        'admin-nav-users',
        'admin-nav-apikeys',
        'developer-count-card',
        'section-organizations',
    ]) {
        assert.match(
            html,
            new RegExp(`class="[^"]*admin-only[^"]*"[^>]*id="${id}"|id="${id}"[^>]*class="[^"]*admin-only[^"]*"`),
            id,
        );
    }

    function parseBootstrap(textContent) {
        const context = vm.createContext({
            document: {
                getElementById: () => textContent === null ? null : { textContent },
            },
        });
        vm.runInContext(
            `${bootstrapSource()}
globalThis.bootstrapForTest = readDashboardBootstrap();`,
            context,
        );
        return context.bootstrapForTest;
    }

    assert.equal(parseBootstrap('{"isAdmin":true}').isAdmin, true);
    assert.equal(parseBootstrap('{"isAdmin":"true"}').isAdmin, false);
    assert.equal(parseBootstrap('{invalid').isAdmin, false);
    assert.equal(parseBootstrap(null).isAdmin, false);
});

test('dashboard request infrastructure is loaded as an explicit module', () => {
    assert.match(
        dashboardSource,
        /from '\.\/dashboard\/api\.js';/,
    );
    assert.match(
        dashboardSource,
        /createApiClient\(\{\s*fetchImpl: window\.fetch\.bind\(window\),\s*location: window\.location,/,
    );
    assert.doesNotMatch(dashboardSource, /class ApiRequestError/);
    assert.match(
        html,
        /<script type="module" src="\/static\/dashboard\.js\?v=__GITAI_DASHBOARD_JS_VERSION__"><\/script>/,
    );
});

test('dashboard action listeners are registered once per delegated event type', () => {
    const harness = createHarness();
    harness.initializeDashboardActions();

    assert.equal(harness.listeners.get('click'), harness.handleDashboardAction);
    assert.equal(harness.listeners.get('change'), harness.handleDashboardAction);
    assert.equal(harness.listeners.size, 2);
});
