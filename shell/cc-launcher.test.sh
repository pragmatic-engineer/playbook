#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# cc-launcher.test.sh: smoke-tests for shell/bash/cc.sh and the system-prompt guard.
#
# Hermetic: uses mktemp HOME and a stub claude on a temp PATH.  Nothing real
# launches.  Run: bash shell/cc-launcher.test.sh
#
# Tests:
#   (a) sourcing shell/bash/cc.sh in bash defines cc and ccd
#   (b) without SYSTEM_PROMPT.md, --system-prompt-file is NOT passed
#   (c) with    SYSTEM_PROMPT.md, --system-prompt-file IS  passed
#   (d) cc worktree is available in bash: _cc_worktree is defined and prints no zsh-only stub
#   (e-g) same guard tests (b-c) for shell/zsh/cc.zsh in zsh, if zsh is on PATH

set -uo pipefail

# ─── minimal test harness ────────────────────────────────────────────────────
PASS=0; FAIL=0; SKIP=0

pass() { printf 'PASS  %s\n' "$1"; PASS=$(( PASS + 1 )); }
fail() { printf 'FAIL  %s\n' "$1" >&2; FAIL=$(( FAIL + 1 )); }
skip() { printf 'SKIP  %s\n' "$1"; SKIP=$(( SKIP + 1 )); }

# Locate repo root: script lives at shell/cc-launcher.test.sh.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# ─── temp environment ────────────────────────────────────────────────────────
TESTHOME=$(mktemp -d)
TMPBIN=$(mktemp -d)
cleanup() { rm -rf "$TESTHOME" "$TMPBIN" "${ZSH_HOME:-}"; }
trap cleanup EXIT

# Stub claude: writes one arg-per-line to $CC_TEST_ARGS_FILE (when set), exits 0.
cat > "$TMPBIN/claude" << 'STUBEOF'
#!/bin/sh
[ -n "${CC_TEST_ARGS_FILE:-}" ] && printf '%s\n' "$@" >> "$CC_TEST_ARGS_FILE"
exit 0
STUBEOF
chmod +x "$TMPBIN/claude"

# ─── bash tests ──────────────────────────────────────────────────────────────
printf '\n=== bash tests ===\n'

# (a) sourcing defines cc and ccd
if HOME="$TESTHOME" PATH="$TMPBIN:$PATH" bash -c \
    "source '$REPO_DIR/shell/bash/cc.sh' && type cc > /dev/null && type ccd > /dev/null" \
    >/dev/null 2>&1; then
    pass "(a) bash: sourcing shell/bash/cc.sh defines cc and ccd"
else
    fail "(a) bash: sourcing shell/bash/cc.sh defines cc and ccd"
fi

# (b) no SYSTEM_PROMPT.md → --system-prompt-file must NOT appear in args
ARGS_B=$(mktemp)
HOME="$TESTHOME" PATH="$TMPBIN:$PATH" CC_TEST_ARGS_FILE="$ARGS_B" \
    bash -c "source '$REPO_DIR/shell/bash/cc.sh'; cc fresh" >/dev/null 2>&1 || true
if grep -q -- '--system-prompt-file' "$ARGS_B"; then
    fail "(b) bash: without SYSTEM_PROMPT.md should NOT pass --system-prompt-file"
else
    pass "(b) bash: without SYSTEM_PROMPT.md does not pass --system-prompt-file"
fi
rm -f "$ARGS_B"

# (c) with SYSTEM_PROMPT.md → --system-prompt-file MUST appear in args
mkdir -p "$TESTHOME/.claude/prompts"
printf '# test system prompt\n' > "$TESTHOME/.claude/prompts/SYSTEM_PROMPT.md"
ARGS_C=$(mktemp)
HOME="$TESTHOME" PATH="$TMPBIN:$PATH" CC_TEST_ARGS_FILE="$ARGS_C" \
    bash -c "source '$REPO_DIR/shell/bash/cc.sh'; cc fresh" >/dev/null 2>&1 || true
if grep -q -- '--system-prompt-file' "$ARGS_C"; then
    pass "(c) bash: with SYSTEM_PROMPT.md passes --system-prompt-file"
else
    fail "(c) bash: with SYSTEM_PROMPT.md should pass --system-prompt-file"
fi
rm -f "$ARGS_C"
rm -f "$TESTHOME/.claude/prompts/SYSTEM_PROMPT.md"

# (d) cc worktree is now available in bash (_cc_worktree defined, no zsh-only stub)
# cc.sh sources worktree.sh via $HOME/.claude/shell/shared/worktree.sh; stage it.
mkdir -p "$TESTHOME/.claude/shell/shared"
cp "$REPO_DIR/shell/shared/worktree.sh" "$TESTHOME/.claude/shell/shared/worktree.sh"

WORKTREE_D_DEFINED=0
HOME="$TESTHOME" PATH="$TMPBIN:$PATH" \
    bash -c "source '$REPO_DIR/shell/bash/cc.sh'; type _cc_worktree >/dev/null 2>&1" \
    && WORKTREE_D_DEFINED=1 || true

# Call cc worktree with no branch: fails early ("not a git repository") but
# must NOT print the old zsh-only message.
WORKTREE_D_OUT=$(
    HOME="$TESTHOME" PATH="$TMPBIN:$PATH" \
    bash -c "source '$REPO_DIR/shell/bash/cc.sh'; cc worktree 2>&1" || true
)

if [[ "$WORKTREE_D_DEFINED" -eq 1 ]] && ! printf '%s' "$WORKTREE_D_OUT" | grep -q "zsh-only"; then
    pass "(d) bash: cc worktree is available (_cc_worktree defined, no zsh-only stub)"
else
    fail "(d) bash: cc worktree should be available (defined=$WORKTREE_D_DEFINED, out=$WORKTREE_D_OUT)"
fi

rm -f "$TESTHOME/.claude/shell/shared/worktree.sh"

# ─── zsh tests (guard only; skip gracefully if zsh absent) ───────────────────
printf '\n=== zsh tests ===\n'

if ! command -v zsh >/dev/null 2>&1; then
    skip "(e) zsh not on PATH: skipping zsh guard tests"
    skip "(f) zsh not on PATH: skipping zsh guard tests"
    skip "(g) zsh not on PATH: skipping zsh guard tests"
else
    # Build a minimal HOME structure so cc.zsh can source its modules.
    ZSH_HOME=$(mktemp -d)

    mkdir -p "$ZSH_HOME/.claude/shell/zsh"
    mkdir -p "$ZSH_HOME/.claude/shell/shared"
    mkdir -p "$ZSH_HOME/.claude/hooks/lib"

    # Copy the real modules.
    cp "$REPO_DIR/shell/zsh/bust-cache.zsh"      "$ZSH_HOME/.claude/shell/zsh/"
    cp "$REPO_DIR/shell/zsh/config-drift.zsh"    "$ZSH_HOME/.claude/shell/zsh/"
    cp "$REPO_DIR/shell/zsh/retention.zsh"       "$ZSH_HOME/.claude/shell/zsh/"
    cp "$REPO_DIR/shell/zsh/sessions.zsh"        "$ZSH_HOME/.claude/shell/zsh/"
    cp "$REPO_DIR/shell/zsh/clean-resume.zsh"    "$ZSH_HOME/.claude/shell/zsh/"
    cp "$REPO_DIR/shell/zsh/dispatch.zsh"        "$ZSH_HOME/.claude/shell/zsh/"
    cp "$REPO_DIR/shell/zsh/cc.zsh"              "$ZSH_HOME/.claude/shell/zsh/"

    # Stub worktree.sh: complex + not needed for guard tests.
    printf '# stub\n_cc_worktree() { printf "worktree stub\\n" >&2; return 1; }\n' \
        > "$ZSH_HOME/.claude/shell/shared/worktree.sh"

    # Stub config-hash.sh (sourced by config-drift.zsh).
    printf 'config_hash() { printf "testhash\\n"; }\n' \
        > "$ZSH_HOME/.claude/hooks/lib/config-hash.sh"

    ZSH_SRC="$ZSH_HOME/.claude/shell/zsh/cc.zsh"

    # (e) no SYSTEM_PROMPT.md → --system-prompt-file must NOT appear
    ARGS_E=$(mktemp)
    HOME="$ZSH_HOME" PATH="$TMPBIN:$PATH" CC_TEST_ARGS_FILE="$ARGS_E" \
        zsh -c "source '$ZSH_SRC'; cc fresh" >/dev/null 2>&1 || true
    if grep -q -- '--system-prompt-file' "$ARGS_E"; then
        fail "(e) zsh: without SYSTEM_PROMPT.md should NOT pass --system-prompt-file"
    else
        pass "(e) zsh: without SYSTEM_PROMPT.md does not pass --system-prompt-file"
    fi
    rm -f "$ARGS_E"

    # (f) with SYSTEM_PROMPT.md → --system-prompt-file MUST appear
    mkdir -p "$ZSH_HOME/.claude/prompts"
    printf '# zsh system prompt\n' > "$ZSH_HOME/.claude/prompts/SYSTEM_PROMPT.md"
    ARGS_F=$(mktemp)
    HOME="$ZSH_HOME" PATH="$TMPBIN:$PATH" CC_TEST_ARGS_FILE="$ARGS_F" \
        zsh -c "source '$ZSH_SRC'; cc fresh" >/dev/null 2>&1 || true
    if grep -q -- '--system-prompt-file' "$ARGS_F"; then
        pass "(f) zsh: with SYSTEM_PROMPT.md passes --system-prompt-file"
    else
        fail "(f) zsh: with SYSTEM_PROMPT.md should pass --system-prompt-file"
    fi
    rm -f "$ARGS_F"
    rm -f "$ZSH_HOME/.claude/prompts/SYSTEM_PROMPT.md"

    rm -rf "$ZSH_HOME"
    unset ZSH_HOME
fi

# ─── summary ─────────────────────────────────────────────────────────────────
printf '\n--- %d passed  %d failed  %d skipped ---\n' "$PASS" "$FAIL" "$SKIP"
[[ "$FAIL" -eq 0 ]]
