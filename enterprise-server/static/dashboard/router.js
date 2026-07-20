import {
    ADMIN_ONLY_DASHBOARD_SECTIONS,
    DASHBOARD_DEFAULT_SECTION,
    DASHBOARD_SECTIONS,
} from './state.js';

export function createDashboardRouter({ isAdmin, location, history }) {
    function canAccessDashboardSection(id) {
        return DASHBOARD_SECTIONS.includes(id)
            && (isAdmin || !ADMIN_ONLY_DASHBOARD_SECTIONS.includes(id));
    }

    function requestedDashboardSection() {
        return new URL(location.href).searchParams.get('section');
    }

    function dashboardSectionFromLocation() {
        const requestedSection = requestedDashboardSection();
        return canAccessDashboardSection(requestedSection)
            ? requestedSection
            : DASHBOARD_DEFAULT_SECTION;
    }

    function updateDashboardSectionUrl(id, replace = false) {
        const url = new URL(location.href);
        url.hash = '';
        if (id === DASHBOARD_DEFAULT_SECTION) {
            url.searchParams.delete('section');
        } else {
            url.searchParams.set('section', id);
        }
        const nextUrl = `${url.pathname}${url.search}${url.hash}`;
        history[replace ? 'replaceState' : 'pushState']({ section: id }, '', nextUrl);
    }

    return Object.freeze({
        canAccessDashboardSection,
        dashboardSectionFromLocation,
        requestedDashboardSection,
        updateDashboardSectionUrl,
    });
}
