#!/usr/bin/env bats

setup() {
    export TEST_TEMP_DIR="$(mktemp -d)"
    export ORIGINAL_HOME="$HOME"
    export ORIGINAL_PATH="$PATH"
    export HOME="$TEST_TEMP_DIR/home"
    mkdir -p "$HOME"

    {
        printf '%s\n' 'error() { echo "$1" >&2; return 1; }'
        sed -n '/^resolve_std_git_candidate()/,/^# Detect standard git path/p' install.sh | sed '$d'
    } > "$TEST_TEMP_DIR/git-detection-functions.sh"
    # shellcheck source=/dev/null
    source "$TEST_TEMP_DIR/git-detection-functions.sh"
}

teardown() {
    export HOME="$ORIGINAL_HOME"
    export PATH="$ORIGINAL_PATH"
    rm -rf "$TEST_TEMP_DIR"
}

make_fake_git() {
    local path="$1"
    mkdir -p "$(dirname "$path")"
    printf '%s\n' '#!/usr/bin/env bash' 'echo "git version 2.50.0"' > "$path"
    chmod +x "$path"
}

canonical_path() {
    local path="$1"
    local dir
    dir=$(cd -P "$(dirname "$path")" && pwd)
    printf '%s/%s\n' "$dir" "$(basename "$path")"
}

@test "detect_std_git skips an existing git-ai shim and continues through PATH" {
    local shim_dir="$HOME/.git-ai/bin"
    local real_git="$TEST_TEMP_DIR/real-bin/git"
    make_fake_git "$shim_dir/git-ai"
    ln -s "$shim_dir/git-ai" "$shim_dir/git"
    make_fake_git "$real_git"
    export PATH="$shim_dir:$(dirname "$real_git"):/usr/bin:/bin"

    run detect_std_git

    [ "$status" -eq 0 ]
    [ "$output" = "$(canonical_path "$real_git")" ]
}

@test "detect_std_git recovers the real Git from an existing git-og symlink" {
    local saved_git="$TEST_TEMP_DIR/saved-bin/git"
    local path_git="$TEST_TEMP_DIR/path-bin/git"
    make_fake_git "$saved_git"
    make_fake_git "$path_git"
    mkdir -p "$HOME/.git-ai/bin"
    ln -s "$saved_git" "$HOME/.git-ai/bin/git-og"
    export PATH="$(dirname "$path_git"):/usr/bin:/bin"

    run detect_std_git

    [ "$status" -eq 0 ]
    [ "$output" = "$(canonical_path "$saved_git")" ]
}

@test "detect_std_git uses a valid git_path from existing config" {
    local configured_git="$TEST_TEMP_DIR/configured-bin/git"
    local path_git="$TEST_TEMP_DIR/path-bin/git"
    make_fake_git "$configured_git"
    make_fake_git "$path_git"
    mkdir -p "$HOME/.git-ai"
    printf '{"git_path":"%s"}\n' "$configured_git" > "$HOME/.git-ai/config.json"
    export PATH="$(dirname "$path_git"):/usr/bin:/bin"

    run detect_std_git

    [ "$status" -eq 0 ]
    [ "$output" = "$(canonical_path "$configured_git")" ]
}
