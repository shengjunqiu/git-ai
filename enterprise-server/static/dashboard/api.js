export class ApiRequestError extends Error {
    constructor(message, { status = null, requestId = null, cause = null } = {}) {
        super(message, cause ? { cause } : undefined);
        this.name = this.constructor.name;
        this.status = status;
        this.requestId = requestId;
    }
}

export class AuthExpiredError extends ApiRequestError {}
export class PermissionDeniedError extends ApiRequestError {}
export class HttpError extends ApiRequestError {}
export class InvalidResponseError extends ApiRequestError {}
export class NetworkError extends ApiRequestError {}
export class TimeoutError extends ApiRequestError {}
export class AbortError extends ApiRequestError {}

const API_DEFAULT_TIMEOUT_MS = 15000;
const API_GET_RETRIES = 1;
const API_RETRYABLE_STATUSES = new Set([429, 502, 503, 504]);

function requestIdFromResponse(response) {
    return response.headers?.get?.('x-request-id') || null;
}

function safeResponseMessage(data, status, fallback) {
    if (status >= 500) return fallback;
    const value = typeof data?.error === 'string'
        ? data.error
        : typeof data?.message === 'string'
            ? data.message
            : '';
    const message = value.replace(/\s+/g, ' ').trim();
    return message && message.length <= 300 ? message : fallback;
}

function authReturnTo(location) {
    return `${location.pathname}${location.search}${location.hash}`;
}

function redirectToLogin(location) {
    const loginUrl = `/auth/login?return_to=${encodeURIComponent(authReturnTo(location))}`;
    location.assign(loginUrl);
}

function waitForRetry(delayMs, signal) {
    return new Promise((resolve, reject) => {
        if (signal?.aborted) {
            reject(new AbortError('请求已取消'));
            return;
        }
        const onAbort = () => {
            clearTimeout(timer);
            reject(new AbortError('请求已取消'));
        };
        const timer = setTimeout(() => {
            signal?.removeEventListener('abort', onAbort);
            resolve();
        }, delayMs);
        signal?.addEventListener('abort', onAbort, { once: true });
    });
}

function createRequestSignal(externalSignal, timeoutMs) {
    const controller = new AbortController();
    let timedOut = false;
    const onExternalAbort = () => controller.abort(externalSignal.reason);
    if (externalSignal?.aborted) onExternalAbort();
    else externalSignal?.addEventListener('abort', onExternalAbort, { once: true });
    const timeout = Number.isFinite(timeoutMs) && timeoutMs > 0
        ? setTimeout(() => {
            timedOut = true;
            controller.abort();
        }, timeoutMs)
        : null;
    return {
        signal: controller.signal,
        timedOut: () => timedOut,
        cleanup: () => {
            if (timeout) clearTimeout(timeout);
            externalSignal?.removeEventListener('abort', onExternalAbort);
        },
    };
}

async function parseApiResponse(response) {
    const requestId = requestIdFromResponse(response);
    const contentType = response.headers?.get?.('content-type') || '';
    const body = await response.text();
    let data = null;
    if (body) {
        if (!contentType.toLowerCase().includes('application/json')) {
            if (response.ok) {
                throw new InvalidResponseError('服务器返回了非 JSON 响应', {
                    status: response.status,
                    requestId,
                });
            }
        } else {
            try {
                data = JSON.parse(body);
            } catch (cause) {
                if (response.ok) {
                    throw new InvalidResponseError('服务器返回了无效 JSON', {
                        status: response.status,
                        requestId,
                        cause,
                    });
                }
            }
        }
    }

    if (response.ok) {
        if (!body && response.status !== 204) {
            throw new InvalidResponseError('服务器返回了空响应', {
                status: response.status,
                requestId,
            });
        }
        return data;
    }
    if (response.status === 401) {
        throw new AuthExpiredError('登录已过期，请重新登录', {
            status: response.status,
            requestId,
        });
    }
    if (response.status === 403) {
        throw new PermissionDeniedError(
            safeResponseMessage(data, response.status, '没有执行此操作的权限'),
            { status: response.status, requestId },
        );
    }
    throw new HttpError(
        safeResponseMessage(data, response.status, `请求失败（HTTP ${response.status}）`),
        { status: response.status, requestId },
    );
}

export function createApiClient({ fetchImpl, location }) {
    if (typeof fetchImpl !== 'function') {
        throw new TypeError('createApiClient requires fetchImpl');
    }
    if (!location || typeof location.assign !== 'function') {
        throw new TypeError('createApiClient requires a location with assign()');
    }

    async function apiRequest(url, options = {}) {
        const method = String(options.method || 'GET').toUpperCase();
        const retries = options.retries ?? (method === 'GET' ? API_GET_RETRIES : 0);
        const headers = new Headers(options.headers || {});
        if (!headers.has('Accept')) headers.set('Accept', 'application/json');

        for (let attempt = 0; attempt <= retries; attempt += 1) {
            const requestSignal = createRequestSignal(
                options.signal,
                options.timeoutMs ?? API_DEFAULT_TIMEOUT_MS,
            );
            try {
                const response = await fetchImpl(url, {
                    ...options,
                    method,
                    headers,
                    signal: requestSignal.signal,
                });
                if (
                    method === 'GET'
                    && API_RETRYABLE_STATUSES.has(response.status)
                    && attempt < retries
                ) {
                    requestSignal.cleanup();
                    await response.body?.cancel?.();
                    await waitForRetry(250 * (2 ** attempt), options.signal);
                    continue;
                }
                const data = await parseApiResponse(response);
                requestSignal.cleanup();
                return data;
            } catch (error) {
                const didTimeout = requestSignal.timedOut();
                requestSignal.cleanup();
                let typedError = error;
                if (options.signal?.aborted) {
                    typedError = new AbortError('请求已取消', { cause: error });
                } else if (didTimeout) {
                    typedError = new TimeoutError('请求超时，请稍后重试', { cause: error });
                } else if (error instanceof TypeError || error?.name === 'TypeError') {
                    typedError = new NetworkError('网络连接失败，请检查网络后重试', {
                        cause: error,
                    });
                }

                const retryableTransportError =
                    typedError instanceof NetworkError || typedError instanceof TimeoutError;
                if (method === 'GET' && retryableTransportError && attempt < retries) {
                    await waitForRetry(250 * (2 ** attempt), options.signal);
                    continue;
                }
                if (typedError instanceof AuthExpiredError) redirectToLogin(location);
                throw typedError;
            }
        }
    }

    return Object.freeze({ apiRequest });
}
