const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const dashboardSource = fs.readFileSync(
    path.join(__dirname, 'dashboard.js'),
    'utf8',
);

function uploadHelperSource() {
    const startMarker = 'const REQUIRED_RELEASE_FILES = [';
    const endMarker = 'function renderSelectedReleaseFiles()';
    const start = dashboardSource.indexOf(startMarker);
    const end = dashboardSource.indexOf(endMarker, start);
    assert.notEqual(start, -1, `missing source marker: ${startMarker}`);
    assert.notEqual(end, -1, `missing source marker: ${endMarker}`);
    return dashboardSource.slice(start, end);
}

function createHarness() {
    const context = vm.createContext({ TextEncoder });
    vm.runInContext(`${uploadHelperSource()}
globalThis.uploadTestExports = {
    DEFAULT_MANAGED_FILE_MAX_BYTES,
    DEFAULT_RELEASE_FILE_MAX_BYTES,
    DEFAULT_RELEASE_TOTAL_MAX_BYTES,
    REQUIRED_RELEASE_FILES,
    activeUploads,
    analyzeManagedFiles,
    analyzeReleaseFiles,
    beginUpload,
    finishUpload,
    isSafeUploadFilename,
    isValidUploadVersion,
    managedFileExtension,
    managedSelectionError,
    releaseSelectionError,
    warnBeforeLeavingDuringUpload,
};`, context);
    return context.uploadTestExports;
}

function releaseFiles(size = 1024) {
    return [
        { name: 'git-ai-linux-x64', size },
        { name: 'git-ai-linux-arm64', size },
        { name: 'git-ai-windows-x64.exe', size },
        { name: 'git-ai-windows-arm64.exe', size },
        { name: 'git-ai-macos-x64', size },
        { name: 'git-ai-macos-arm64', size },
    ];
}

test('complete release bundle passes exact filename and size validation', () => {
    const harness = createHarness();
    const analysis = harness.analyzeReleaseFiles(releaseFiles());

    assert.equal(analysis.valid, true);
    assert.equal(analysis.totalBytes, 6 * 1024);
    assert.equal(harness.releaseSelectionError(analysis), '');
    assert.equal(harness.isValidUploadVersion('1.4.0-rc.1+build'), true);
    assert.equal(harness.isValidUploadVersion('../1.4.0'), false);
});

test('release validation reports missing, duplicate, unexpected, empty, and oversized files', () => {
    const harness = createHarness();
    const files = releaseFiles();
    files.splice(0, 1, { name: 'git-ai-linux-arm64', size: 1024 });
    files.push({ name: 'git-ai-linux-x64.zip', size: 0 });
    files[2].size = harness.DEFAULT_RELEASE_FILE_MAX_BYTES + 1;

    const analysis = harness.analyzeReleaseFiles(files);
    const message = harness.releaseSelectionError(analysis);

    assert.equal(analysis.valid, false);
    assert.match(message, /缺少：git-ai-linux-x64/);
    assert.match(message, /文件名重复：git-ai-linux-arm64/);
    assert.match(message, /文件名不正确：git-ai-linux-x64\.zip/);
    assert.match(message, /空文件：git-ai-linux-x64\.zip/);
    assert.match(message, /单文件超过/);
});

test('release validation enforces the combined upload limit', () => {
    const harness = createHarness();
    const files = releaseFiles(90 * 1024 * 1024);
    const analysis = harness.analyzeReleaseFiles(files);

    assert.equal(analysis.oversized.length, 0);
    assert.equal(analysis.totalBytes > harness.DEFAULT_RELEASE_TOTAL_MAX_BYTES, true);
    assert.match(harness.releaseSelectionError(analysis), /总大小超过/);
});

test('managed file validation reports filename, extension, count, and size', () => {
    const harness = createHarness();
    const valid = harness.analyzeManagedFiles([{ name: 'bundle.tar.gz', size: 2048 }]);
    assert.equal(valid.valid, true);
    assert.equal(valid.extension, '.tar.gz');
    assert.equal(harness.managedSelectionError(valid), '');

    const invalidName = harness.analyzeManagedFiles([{ name: '../secret.zip', size: 1 }]);
    assert.equal(harness.isSafeUploadFilename('../secret.zip'), false);
    assert.match(harness.managedSelectionError(invalidName), /文件名无效/);

    const tooMany = harness.analyzeManagedFiles([
        { name: 'one.zip', size: 1 },
        { name: 'two.zip', size: 1 },
    ]);
    assert.match(harness.managedSelectionError(tooMany), /只能选择一个文件/);

    const oversized = harness.analyzeManagedFiles([{
        name: 'large.zip',
        size: harness.DEFAULT_MANAGED_FILE_MAX_BYTES + 1,
    }]);
    assert.match(harness.managedSelectionError(oversized), /超过/);
    assert.equal(harness.managedFileExtension('LICENSE'), '无扩展名');
});

test('upload operation guard blocks duplicate submissions and restores the button', () => {
    const harness = createHarness();
    const button = { disabled: false, textContent: '上传并发布' };

    assert.equal(harness.beginUpload('release', button, '正在上传...'), true);
    assert.equal(harness.beginUpload('release', button, '正在上传...'), false);
    assert.equal(button.disabled, true);
    assert.equal(button.textContent, '正在上传...');

    const event = {
        prevented: false,
        preventDefault() {
            this.prevented = true;
        },
    };
    harness.warnBeforeLeavingDuringUpload(event);
    assert.equal(event.prevented, true);
    assert.equal(event.returnValue, '');

    harness.finishUpload('release');
    assert.equal(button.disabled, false);
    assert.equal(button.textContent, '上传并发布');
    assert.equal(harness.activeUploads.size, 0);
});
