const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const html = fs.readFileSync(path.join(__dirname, 'dashboard.html'), 'utf8');
const css = fs.readFileSync(path.join(__dirname, 'dashboard.css'), 'utf8');
const javascript = fs.readFileSync(path.join(__dirname, 'dashboard.js'), 'utf8');

test('mobile navigation exposes menu, account, logout, drawer, and backdrop controls', () => {
    for (const marker of [
        'id="mobile-menu-button"',
        'aria-controls="dashboard-sidebar"',
        'id="dashboard-sidebar"',
        'id="sidebar-close-button"',
        'id="sidebar-backdrop"',
        'class="mobile-account-name"',
        'class="mobile-logout" href="/logout"',
    ]) {
        assert.equal(html.includes(marker), true, marker);
    }
});

test('mobile navigation handles focus, escape, backdrop, and responsive state', () => {
    for (const behavior of [
        'function openMobileNavigation(',
        'function closeMobileNavigation(',
        "event.key === 'Escape'",
        "event.key !== 'Tab'",
        "sidebarBackdrop.addEventListener('click'",
        'mobileNavigationReturnFocus',
        'dashboardSidebar.inert = true',
        "mobileNavigationMediaQuery.addEventListener('change'",
    ]) {
        assert.equal(javascript.includes(behavior), true, behavior);
    }
});

test('every dashboard table has a keyboard-accessible horizontal scroll region', () => {
    const tableCount = (html.match(/<table>/g) || []).length;
    const scrollRegions = html.match(
        /<div class="table-scroll[^"]*" tabindex="0" role="region" aria-label="[^"]+">/g,
    ) || [];

    assert.equal(tableCount, 9);
    assert.equal(scrollRegions.length, tableCount);
    assert.equal(css.includes('.table-card { background:'), true);
    assert.equal(css.includes('overflow: visible; margin-bottom: 1.5rem;'), true);
    assert.equal(css.includes('.table-scroll:focus-visible'), true);
    assert.equal(css.includes('.table-scroll-wide table { min-width: 980px; }'), true);
});

test('small-screen CSS keeps navigation and primary controls usable', () => {
    assert.equal(css.includes('@media (max-width: 768px)'), true);
    assert.equal(css.includes('.sidebar.open { transform: translateX(0); visibility: visible; }'), true);
    assert.equal(css.includes('body.mobile-nav-open .main { overflow: hidden; }'), true);
    assert.equal(css.includes('@media (max-width: 480px)'), true);
    assert.equal(css.includes('.stats-grid { grid-template-columns: 1fr; }'), true);
    assert.equal(css.includes('min-height: 44px;'), true);
    assert.equal(css.includes('@media (prefers-reduced-motion: reduce)'), true);
    assert.doesNotMatch(css, /\.sidebar\s*\{\s*display:\s*none/);
});

test('mobile help code controls no longer overlay the code content', () => {
    assert.match(
        css,
        /@media \(max-width: 480px\)[\s\S]*\.help-code-block button \{ position: static;/,
    );
    assert.match(
        css,
        /@media \(max-width: 768px\)[\s\S]*\.modal \{ width: 100%; max-height: calc\(100dvh - 2rem\);/,
    );
});
