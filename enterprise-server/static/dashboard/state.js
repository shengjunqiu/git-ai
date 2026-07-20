export const RefreshMode = Object.freeze({
    INITIAL: 'initial',
    MANUAL: 'manual',
    AUTO: 'auto',
});

export const DASHBOARD_DEFAULT_SECTION = 'overview';
export const DASHBOARD_SECTIONS = Object.freeze([
    'overview',
    'trends',
    'organizations',
    'departments',
    'developers',
    'projects',
    'tools',
    'users',
    'apikeys',
    'releases',
    'files',
    'help',
]);
export const ADMIN_ONLY_DASHBOARD_SECTIONS = Object.freeze([
    'organizations',
    'users',
    'apikeys',
    'releases',
    'files',
]);

export function createDashboardState(initialSection = DASHBOARD_DEFAULT_SECTION) {
    const currentSection = DASHBOARD_SECTIONS.includes(initialSection)
        ? initialSection
        : DASHBOARD_DEFAULT_SECTION;
    return {
        currentSection,
        refreshInterval: null,
        sectionRefreshes: new Map(),
        queuedManualRefreshes: new Map(),
        lastRefreshAttemptAt: null,
        lastRefreshSuccessAt: null,
        successfulSections: new Set(),
    };
}
