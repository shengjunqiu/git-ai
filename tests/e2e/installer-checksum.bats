#!/usr/bin/env bats

setup() {
    export TEST_TEMP_DIR="$(mktemp -d)"
    export ORIGINAL_PATH="$PATH"

    {
        sed -n '/^error() {/,/^}/p' install.sh
        sed -n '/^warn() {/,/^}/p' install.sh
        sed -n '/^success() {/,/^}/p' install.sh
        sed -n '/^verify_checksum() {/,/^}/p' install.sh
    } > "$TEST_TEMP_DIR/checksum-functions.sh"
    # shellcheck source=/dev/null
    source "$TEST_TEMP_DIR/checksum-functions.sh"
}

teardown() {
    export PATH="$ORIGINAL_PATH"
    rm -rf "$TEST_TEMP_DIR"
}

@test "release checksum verification fails closed when no SHA-256 tool exists" {
    local payload="$TEST_TEMP_DIR/git-ai-test"
    local tools_dir="$TEST_TEMP_DIR/tools"
    mkdir -p "$tools_dir"
    printf '%s\n' 'test payload' > "$payload"
    ln -s "$(command -v awk)" "$tools_dir/awk"
    ln -s "$(command -v rm)" "$tools_dir/rm"

    EMBEDDED_CHECKSUMS="0000000000000000000000000000000000000000000000000000000000000000  git-ai-test"
    export PATH="$tools_dir"

    run verify_checksum "$payload" 'git-ai-test'

    [ "$status" -ne 0 ]
    [[ "$output" == *"neither sha256sum nor shasum is installed"* ]]
    [ ! -e "$payload" ]
}

@test "unreleased template still skips checksum verification placeholder" {
    local payload="$TEST_TEMP_DIR/git-ai-test"
    printf '%s\n' 'test payload' > "$payload"
    EMBEDDED_CHECKSUMS='__CHECKSUMS_PLACEHOLDER__'
    export PATH="$TEST_TEMP_DIR/empty-path"

    run verify_checksum "$payload" 'git-ai-test'

    [ "$status" -eq 0 ]
    [ -e "$payload" ]
}
