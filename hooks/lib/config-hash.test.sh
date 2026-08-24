#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# config-hash.test.sh: tests for config_hash() in hooks/lib/config-hash.sh.
# Runs HOME-isolated under a scratch directory; verifies the test-file exclusion
# fix and cross-shell compatibility under zsh.
#
# Run:  bash hooks/lib/config-hash.test.sh
# Exit: 0 if all cases pass, non-zero otherwise.
set -u

if ! command -v shasum >/dev/null 2>&1; then
  echo "SKIP: shasum not available"
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=hooks/lib/config-hash.sh
source "${SCRIPT_DIR}/config-hash.sh"

PASS=0
FAIL=0

ok()   { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1"; (( FAIL++ )) || true; }

# ── Scratch HOME ──────────────────────────────────────────────────────────────
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT INT TERM

mkdir -p "$SCRATCH/.claude/hooks"
printf '{"model":"claude-opus-4-5"}\n' > "$SCRATCH/.claude/settings.json"
printf '#!/bin/sh\n# hook a\n'         > "$SCRATCH/.claude/hooks/a.sh"
printf '#!/bin/sh\n# hook b\n'         > "$SCRATCH/.claude/hooks/b.sh"
printf '# test only\n'                 > "$SCRATCH/.claude/hooks/a.test.sh"
printf '#!/usr/bin/env python3\n# hook c\n' > "$SCRATCH/.claude/hooks/c.py"
printf '# python test only\n'          > "$SCRATCH/.claude/hooks/c.test.py"

export HOME="$SCRATCH"

# 1. Hash is stable across two calls with no change.
h1="$(config_hash)"; h2="$(config_hash)"
if [[ -n "$h1" && "$h1" == "$h2" ]]; then
  ok "stable across two calls"
else
  fail "stable across two calls (h1='$h1' h2='$h2')"
fi

# 2. Editing a.test.sh leaves the hash UNCHANGED (the exclusion fix).
before="$(config_hash)"
printf '# test - modified\n' >> "$SCRATCH/.claude/hooks/a.test.sh"
after="$(config_hash)"
if [[ "$before" == "$after" ]]; then
  ok "test file edit leaves hash unchanged"
else
  fail "test file edit leaves hash unchanged (before='$before' after='$after')"
fi

# 3. Editing a.sh CHANGES the hash.
before="$(config_hash)"
printf '# new line\n' >> "$SCRATCH/.claude/hooks/a.sh"
after="$(config_hash)"
if [[ "$before" != "$after" ]]; then
  ok "hook edit changes hash"
else
  fail "hook edit changes hash (before='$before' after='$after')"
fi

# 4. Editing settings.json CHANGES the hash.
before="$(config_hash)"
printf '{"model":"claude-haiku-4-5"}\n' > "$SCRATCH/.claude/settings.json"
after="$(config_hash)"
if [[ "$before" != "$after" ]]; then
  ok "settings edit changes hash"
else
  fail "settings edit changes hash (before='$before' after='$after')"
fi

# 5. Missing settings.json still yields a non-empty hash (hooks are still hashed).
rm -f "$SCRATCH/.claude/settings.json"
h="$(config_hash)"
if [[ -n "$h" ]]; then
  ok "missing settings.json still yields a hash"
else
  fail "missing settings.json still yields a hash (got empty)"
fi

# 6. Cross-shell: sources cleanly under zsh and config_hash produces output.
if ! command -v zsh >/dev/null 2>&1; then
  echo "SKIP (zsh): cross-shell source test, zsh not available"
else
  printf '{"model":"claude-opus-4-5"}\n' > "$SCRATCH/.claude/settings.json"
  # shellcheck disable=SC2016
  if HOME="$SCRATCH" zsh -c 'source "$1"; config_hash >/dev/null' _ "${SCRIPT_DIR}/config-hash.sh"; then
    ok "sources and runs cleanly under zsh"
  else
    fail "sources and runs cleanly under zsh"
  fi
fi

# 7. Editing a python hook (c.py) CHANGES the hash (post-migration coverage).
before="$(config_hash)"
printf '# py new line\n' >> "$SCRATCH/.claude/hooks/c.py"
after="$(config_hash)"
if [[ "$before" != "$after" ]]; then
  ok "python hook edit changes hash"
else
  fail "python hook edit changes hash (before='$before' after='$after')"
fi

# 8. Editing a python test file (c.test.py) leaves the hash UNCHANGED.
before="$(config_hash)"
printf '# py test - modified\n' >> "$SCRATCH/.claude/hooks/c.test.py"
after="$(config_hash)"
if [[ "$before" == "$after" ]]; then
  ok "python test file edit leaves hash unchanged"
else
  fail "python test file edit leaves hash unchanged (before='$before' after='$after')"
fi

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} cases passed"
[[ $FAIL -eq 0 ]]
