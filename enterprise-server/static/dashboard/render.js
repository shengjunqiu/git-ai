export function fmt(value) {
    return typeof value === 'number' ? value.toLocaleString() : '0';
}

function finiteNumber(value, fallback = 0) {
    const number = Number(value);
    return Number.isFinite(number) ? number : fallback;
}

export function clampPercent(value) {
    return Math.min(100, Math.max(0, finiteNumber(value)));
}

export function pctBar(percent) {
    return `<div class="bar"><div class="bar-fill" style="width:${clampPercent(percent)}%"></div></div>`;
}

export function escapeHtml(value) {
    return String(value ?? '')
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;');
}

export function escapeAttribute(value) {
    return String(value ?? '')
        .replace(/&/g, '&amp;')
        .replace(/"/g, '&quot;')
        .replace(/'/g, '&#39;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;');
}

export function fmtTimeAgo(value, nowMs = Date.now()) {
    if (!value) return '从未';
    const date = new Date(value);
    if (Number.isNaN(date.getTime())) return '未知';
    const seconds = Math.max(0, Math.floor((nowMs - date.getTime()) / 1000));
    if (seconds < 60) return '刚刚';
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes} 分钟前`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours} 小时前`;
    const days = Math.floor(hours / 24);
    return `${days} 天前`;
}

export function formatRefreshTime(value) {
    if (!value) return '—';
    return value.toLocaleTimeString('zh-CN', { hour12: false });
}

export function replaceHtmlIfChanged(element, nextHtml) {
    if (!element) return false;
    let comparableHtml = nextHtml;
    if (typeof element.cloneNode === 'function') {
        const comparisonElement = element.cloneNode(false);
        comparisonElement.innerHTML = nextHtml;
        comparableHtml = comparisonElement.innerHTML;
    }
    if (element.innerHTML === comparableHtml) return false;
    element.innerHTML = nextHtml;
    return true;
}

export function setDisplayIfChanged(element, display) {
    if (!element || element.style.display === display) return false;
    element.style.display = display;
    return true;
}

export function setTextIfChanged(element, text) {
    const nextText = String(text);
    if (!element || element.textContent === nextText) return false;
    element.textContent = nextText;
    return true;
}

export function setClassNameIfChanged(element, className) {
    if (!element || element.className === className) return false;
    element.className = className;
    return true;
}

export function setTitleIfChanged(element, title) {
    if (!element || element.title === title) return false;
    element.title = title;
    return true;
}
