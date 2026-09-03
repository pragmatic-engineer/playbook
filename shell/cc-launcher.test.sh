#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# cc-launcher.test.sh: parity + system-prompt guard for the cc/ccd launchers.
#
# The bash entry (shell/bash/cc.sh) and the zsh entry (shell/zsh/cc.zsh) are now
# thin wrappers that both source the SAME modules under shell/shared/. So this
# suite asserts the two shells reach parity: both define the full function set,
# both honour the system-prompt guard.
#
# Hermetic: a mktemp HOME with a full fake install (shared modules, entries, a
# config-hash stub, a stub claude on a temp PATH). Nothing real launches.
# Run: bash shell/cc-launcher.test.sh
set -uo pipefail

PASS=0; FAIL=0; SKIP=0
pass() { printf 'PASS  %s\n' "$1"; PASS=$(( PASS + 1 )); }
fail() { printf 'FAIL  %s\n' "$1" >&2; FAIL=$(( FAIL + 1 )); }
skip() { printf 'SKIP  %s\n' "$1"; SKIP=$(( SKIP + 1 )); }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

TESTHOME=$(mktemp -d)
TMPBIN=$(mktemp -d)
cleanup() { rm -rf "$TESTHOME" "$TMPBIN"; }
trap cleanup EXIT

# Stub claude: writes one arg-per-line to $CC_TEST_ARGS_FILE (when set), exits 0.
cat > "$TMPBIN/claude" << 'STUBEOF'
#!/bin/sh
[ -n "${CC_TEST_ARGS_FILE:-}" ] && printf '%s\n' "$@" >> "$CC_TEST_ARGS_FILE"
exit 0
STUBEOF
chmod +x "$TMPBIN/claude"

# Build a full fake install under $TESTHOME so an entry point can source every
# shared module the way it does in production.
PLAYBOOK_HOME="$TESTHOME/.config/playbook"
mkdir -p "$PLAYBOOK_HOME/shell/shared" "$PLAYBOOK_HOME/shell/bash" \
         "$PLAYBOOK_HOME/shell/zsh" "$PLAYBOOK_HOME/hooks/lib"
cp "$REPO_DIR"/shell/shared/*.sh "$PLAYBOOK_HOME/shell/shared/"
cp "$REPO_DIR"/shell/bash/cc.sh  "$PLAYBOOK_HOME/shell/bash/"
cp "$REPO_DIR"/shell/zsh/cc.zsh  "$PLAYBOOK_HOME/shell/zsh/"
# Stub config-hash so config-drift loads without real settings.
printf 'config_hash() { printf "testhash\\n"; }\n' \
    > "$PLAYBOOK_HOME/hooks/lib/config-hash.sh"

BASH_ENTRY="$PLAYBOOK_HOME/shell/bash/cc.sh"
ZSH_ENTRY="$PLAYBOOK_HOME/shell/zsh/cc.zsh"

# The full function set both shells must expose after sourcing their entry.
FNS="cc ccd _claude _cc_prune _cc_clean_resume _cc_find_session_by_title _cc_config_drifted _cc_worktree _cc_bust_cache"

# check_defines <shell> <entry> <label>: source the entry, assert every function
# in $FNS is defined. This is the parity assertion.
check_defines() {
    local sh="$1" entry="$2" label="$3" missing
    missing=$(HOME="$TESTHOME" PATH="$TMPBIN:$PATH" "$sh" -c \
        "source '$entry' >/dev/null 2>&1; for fn in $FNS; do type \"\$fn\" >/dev/null 2>&1 || printf '%s ' \"\$fn\"; done")
    if [ -z "$missing" ]; then
        pass "$label: defines the full function set"
    else
        fail "$label: missing [$missing]"
    fi
}

# check_prompt_guard <shell> <entry> <label>: cc fresh must pass
# --system-prompt-file only when SYSTEM_PROMPT.md exists.
check_prompt_guard() {
    local sh="$1" entry="$2" label="$3" args
    # Without the prompt file.
    rm -f "$PLAYBOOK_HOME/prompts/SYSTEM_PROMPT.md"
    args=$(mktemp)
    HOME="$TESTHOME" PATH="$TMPBIN:$PATH" CC_TEST_ARGS_FILE="$args" \
        "$sh" -c "source '$entry'; cc fresh" >/dev/null 2>&1 || true
    if grep -q -- '--system-prompt-file' "$args"; then
        fail "$label: without SYSTEM_PROMPT.md should NOT pass --system-prompt-file"
    else
        pass "$label: without SYSTEM_PROMPT.md does not pass --system-prompt-file"
    fi
    rm -f "$args"
    # With the prompt file.
    mkdir -p "$PLAYBOOK_HOME/prompts"
    printf '# test system prompt\n' > "$PLAYBOOK_HOME/prompts/SYSTEM_PROMPT.md"
    args=$(mktemp)
    HOME="$TESTHOME" PATH="$TMPBIN:$PATH" CC_TEST_ARGS_FILE="$args" \
        "$sh" -c "source '$entry'; cc fresh" >/dev/null 2>&1 || true
    if grep -q -- '--system-prompt-file' "$args"; then
        pass "$label: with SYSTEM_PROMPT.md passes --system-prompt-file"
    else
        fail "$label: with SYSTEM_PROMPT.md should pass --system-prompt-file"
    fi
    rm -f "$args" "$PLAYBOOK_HOME/prompts/SYSTEM_PROMPT.md"
}

printf '\n=== bash entry ===\n'
check_defines      bash "$BASH_ENTRY" "(bash)"
check_prompt_guard bash "$BASH_ENTRY" "(bash)"

printf '\n=== zsh entry ===\n'
if command -v zsh >/dev/null 2>&1; then
    check_defines      zsh "$ZSH_ENTRY" "(zsh)"
    check_prompt_guard zsh "$ZSH_ENTRY" "(zsh)"
else
    skip "(zsh) not on PATH: skipping zsh parity tests"
fi

printf '\n--- %d passed  %d failed  %d skipped ---\n' "$PASS" "$FAIL" "$SKIP"
[[ "$FAIL" -eq 0 ]]
