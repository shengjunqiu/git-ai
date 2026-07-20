export const RefreshMode = Object.freeze({
    INITIAL: 'initial',
    MANUAL: 'manual',
    AUTO: 'auto',
});

export const AUTO_REFRESH_MS = 60000;

export function isSilentRefresh(options) {
    return options?.mode === RefreshMode.AUTO;
}

export function refreshCollisionAction(mode, hasInFlight) {
    if (!hasInFlight) return 'start';
    if (mode === RefreshMode.AUTO) return 'skip';
    if (mode === RefreshMode.MANUAL) return 'queue';
    return 'replace';
}

export function createDashboardRefresh({
    currentSection,
    isPageHidden,
    loadSectionData,
    afterSectionSuccess,
    onSectionSuccess,
    onSectionError,
    onStatusChange,
    autoRefreshMs = AUTO_REFRESH_MS,
    now = () => new Date(),
    createAbortController = () => new AbortController(),
    setIntervalImpl = (callback, delay) => setInterval(callback, delay),
    clearIntervalImpl = handle => clearInterval(handle),
}) {
    const sectionRefreshes = new Map();
    const queuedManualRefreshes = new Map();
    const successfulSections = new Set();
    let refreshInterval = null;
    let lastRefreshAttemptAt = null;
    let lastRefreshSuccessAt = null;

    function getRefreshSnapshot({ stale = false } = {}) {
        return Object.freeze({
            lastRefreshAttemptAt,
            lastRefreshSuccessAt,
            stale,
        });
    }

    function updateStatus(options) {
        onStatusChange(getRefreshSnapshot(options));
    }

    function currentSectionRequestSignal() {
        return sectionRefreshes.get(currentSection())?.controller.signal;
    }

    function loadSection(id, { mode = RefreshMode.MANUAL } = {}) {
        const existingRefresh = sectionRefreshes.get(id);
        const collisionAction = refreshCollisionAction(mode, Boolean(existingRefresh));
        if (collisionAction === 'skip') return Promise.resolve(false);
        if (collisionAction === 'queue') {
            const existingQueuedRefresh = queuedManualRefreshes.get(id);
            if (existingQueuedRefresh) return existingQueuedRefresh;

            const queuedRefresh = existingRefresh.promise.then(() => {
                queuedManualRefreshes.delete(id);
                return loadSection(id, { mode: RefreshMode.MANUAL });
            });
            queuedManualRefreshes.set(id, queuedRefresh);
            return queuedRefresh;
        }
        if (collisionAction === 'replace') existingRefresh.controller.abort();

        const controller = createAbortController();
        const refresh = { controller, promise: null };
        refresh.promise = performSectionLoad(id, { mode, controller })
            .finally(() => {
                if (sectionRefreshes.get(id) === refresh) {
                    sectionRefreshes.delete(id);
                }
            });
        sectionRefreshes.set(id, refresh);
        return refresh.promise;
    }

    async function performSectionLoad(id, { mode, controller }) {
        const background = isSilentRefresh({ mode }) && successfulSections.has(id);
        if (currentSection() === id) {
            lastRefreshAttemptAt = now();
            updateStatus({ stale: false });
        }
        try {
            await loadSectionData(id, { mode, signal: controller.signal });
            if (controller.signal.aborted) return false;
            successfulSections.add(id);
            onSectionSuccess(id);
            if (currentSection() === id) {
                await afterSectionSuccess(id, { mode, signal: controller.signal });
                lastRefreshSuccessAt = now();
                updateStatus({ stale: false });
            }
            return true;
        } catch (error) {
            onSectionError(id, error, { background });
            return false;
        }
    }

    function refreshCurrentSection({ mode = RefreshMode.MANUAL } = {}) {
        return loadSection(currentSection(), { mode });
    }

    function startAutoRefresh() {
        stopAutoRefresh();
        refreshInterval = setIntervalImpl(() => {
            if (!isPageHidden()) {
                refreshCurrentSection({ mode: RefreshMode.AUTO });
            }
        }, autoRefreshMs);
    }

    function stopAutoRefresh() {
        if (refreshInterval === null) return;
        clearIntervalImpl(refreshInterval);
        refreshInterval = null;
    }

    function handleVisibilityChange() {
        if (!isPageHidden()) {
            refreshCurrentSection({ mode: RefreshMode.AUTO });
        }
    }

    return Object.freeze({
        currentSectionRequestSignal,
        getRefreshSnapshot,
        handleVisibilityChange,
        loadSection,
        refreshCurrentSection,
        startAutoRefresh,
        stopAutoRefresh,
    });
}
