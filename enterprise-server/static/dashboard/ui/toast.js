export const ToastType = Object.freeze({
    INFO: 'info',
    SUCCESS: 'success',
    ERROR: 'error',
});

const TOAST_TYPES = new Set(Object.values(ToastType));
const DEFAULT_TOAST_DURATION_MS = 3000;

export function createToast({
    document,
    durationMs = DEFAULT_TOAST_DURATION_MS,
    setTimeoutImpl = (callback, delay) => setTimeout(callback, delay),
    clearTimeoutImpl = handle => clearTimeout(handle),
}) {
    let activeToast = null;
    let removalTimer = null;

    function dismissToast() {
        if (removalTimer !== null) {
            clearTimeoutImpl(removalTimer);
            removalTimer = null;
        }
        activeToast?.remove();
        activeToast = null;
    }

    function showToast(message, type = ToastType.INFO) {
        dismissToast();
        document.querySelector('.toast')?.remove();
        const toastType = TOAST_TYPES.has(type) ? type : ToastType.INFO;
        const toast = document.createElement('div');
        toast.className = `toast ${toastType}`;
        toast.textContent = String(message ?? '');
        toast.setAttribute('role', toastType === ToastType.ERROR ? 'alert' : 'status');
        toast.setAttribute(
            'aria-live',
            toastType === ToastType.ERROR ? 'assertive' : 'polite',
        );
        toast.setAttribute('aria-atomic', 'true');
        document.body.appendChild(toast);
        activeToast = toast;
        removalTimer = setTimeoutImpl(() => {
            if (activeToast === toast) {
                activeToast = null;
                removalTimer = null;
            }
            toast.remove();
        }, durationMs);
    }

    return Object.freeze({
        dismissToast,
        showToast,
    });
}
