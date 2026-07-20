const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

function importSource(relativePath) {
    const source = fs.readFileSync(path.join(__dirname, relativePath), 'utf8');
    return import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
}

const stateModulePromise = importSource(path.join('dashboard', 'state.js'));
const renderModulePromise = importSource(path.join('dashboard', 'render.js'));

function createHtmlElement(initialHtml = '') {
    let innerHTML = initialHtml;
    let writes = 0;
    return {
        className: '',
        style: { display: '' },
        textContent: '',
        title: '',
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

test('dashboard state instances are isolated and invalid sections fail closed', async () => {
    const {
        ADMIN_ONLY_DASHBOARD_SECTIONS,
        DASHBOARD_DEFAULT_SECTION,
        DASHBOARD_SECTIONS,
        createDashboardState,
    } = await stateModulePromise;
    const first = createDashboardState('projects');
    const second = createDashboardState('not-a-section');

    assert.equal(first.currentSection, 'projects');
    assert.equal(second.currentSection, DASHBOARD_DEFAULT_SECTION);
    first.successfulSections.add('projects');
    first.sectionRefreshes.set('projects', { promise: Promise.resolve() });
    assert.equal(second.successfulSections.size, 0);
    assert.equal(second.sectionRefreshes.size, 0);
    assert.equal(Object.isFrozen(DASHBOARD_SECTIONS), true);
    assert.equal(Object.isFrozen(ADMIN_ONLY_DASHBOARD_SECTIONS), true);
});

test('pure render helpers escape, clamp, and format deterministic values', async () => {
    const {
        clampPercent,
        escapeAttribute,
        escapeHtml,
        fmtTimeAgo,
    } = await renderModulePromise;
    const now = Date.parse('2026-07-20T12:00:00Z');

    assert.equal(escapeHtml(`<研发 & "平台">`), '&lt;研发 &amp; "平台"&gt;');
    assert.equal(
        escapeAttribute(`研发 "A&B" <核心> '组'`),
        '研发 &quot;A&amp;B&quot; &lt;核心&gt; &#39;组&#39;',
    );
    assert.equal(clampPercent(-10), 0);
    assert.equal(clampPercent(125), 100);
    assert.equal(clampPercent('not-a-number'), 0);
    assert.equal(fmtTimeAgo('2026-07-20T11:58:00Z', now), '2 分钟前');
});

test('element render helpers avoid unchanged writes and normalize text', async () => {
    const {
        replaceHtmlIfChanged,
        setClassNameIfChanged,
        setDisplayIfChanged,
        setTextIfChanged,
        setTitleIfChanged,
    } = await renderModulePromise;
    const element = createHtmlElement('<span>稳定</span>');

    assert.equal(replaceHtmlIfChanged(element, '<span>稳定</span>'), false);
    assert.equal(replaceHtmlIfChanged(element, '<span>更新</span>'), true);
    assert.equal(replaceHtmlIfChanged(element, '<span>更新</span>'), false);
    assert.equal(element.writes(), 1);

    assert.equal(setTextIfChanged(element, 42), true);
    assert.equal(element.textContent, '42');
    assert.equal(setTextIfChanged(element, 42), false);
    assert.equal(setDisplayIfChanged(element, 'none'), true);
    assert.equal(setDisplayIfChanged(element, 'none'), false);
    assert.equal(setClassNameIfChanged(element, 'ready'), true);
    assert.equal(setClassNameIfChanged(element, 'ready'), false);
    assert.equal(setTitleIfChanged(element, '状态'), true);
    assert.equal(setTitleIfChanged(element, '状态'), false);
});
