const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

function registrationScript() {
    const rustSource = fs.readFileSync(
        path.join(__dirname, '..', 'src', 'handlers', 'auth_pages.rs'),
        'utf8',
    );
    const match = rustSource.match(
        /const REGISTER_PAGE_SCRIPT: &str = r#"<script>([\s\S]*?)<\/script>"#;/,
    );
    assert.ok(match, 'registration page script should be extractable');
    return match[1];
}

test('registration request helper is valid JavaScript and owns the only fetch call', () => {
    const source = registrationScript();
    assert.doesNotThrow(() => new vm.Script(source));
    assert.equal((source.match(/\bfetch\(/g) || []).length, 1);
    assert.equal(source.includes('response.json()'), false);
    for (const errorType of [
        'AuthExpiredError',
        'PermissionDeniedError',
        'HttpError',
        'InvalidResponseError',
        'NetworkError',
        'TimeoutError',
        'AbortError',
    ]) {
        assert.equal(source.includes(`class ${errorType}`), true, errorType);
    }
});
