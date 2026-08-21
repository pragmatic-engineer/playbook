#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# ci-git-identity.test.sh: .github/workflows/shell-ci.yml must not impose a
# git identity that overrides a fixture's own.
#
# GIT_AUTHOR_NAME/EMAIL and GIT_COMMITTER_NAME/EMAIL sit ABOVE per-repo config
# in git's precedence, so setting them as step-level env vars silently
# overrides every test fixture that sets its own identity via `git config`.
# That broke shell/worktree.test.sh scenario M: it passed locally, where
# those vars are unset, and failed only in CI. The fix scopes the CI identity
# through GIT_CONFIG_GLOBAL pointed at a throwaway file instead, which a
# fixture's own `git config user.email` still beats, exactly as before.
#
# Scoped to the workflow file only: shell/worktree.test.sh legitimately sets
# GIT_AUTHOR_*/GIT_COMMITTER_* inline for one scenario of its own, and that is
# not this defect.
#
# Run:  bash shell/ci-git-identity.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKFLOW="${SCRIPT_DIR}/../.github/workflows/shell-ci.yml"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1${2:+ -- $2}"; (( FAIL++ )) || true; }

if [ ! -f "$WORKFLOW" ]; then
  fail "shell-ci.yml exists" "not found at $WORKFLOW"
  echo ""
  echo "${PASS}/$(( PASS + FAIL )) scenarios passed"
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

# Real code lines only: strips comment lines (leading whitespace then '#'),
# so the module's own prose explaining why GIT_AUTHOR_*/GIT_COMMITTER_* must
# not appear does not trip the very check it documents.
uncommented() {
  grep -vE '^[[:space:]]*#' "$1"
}

# Whether `file` sets GIT_AUTHOR_NAME/EMAIL or GIT_COMMITTER_NAME/EMAIL
# anywhere outside a comment: 0 = clean, 1 = found.
sets_author_or_committer_env() {
  uncommented "$1" | grep -qE '\bGIT_(AUTHOR|COMMITTER)_(NAME|EMAIL)\b'
}

# Whether `file` scopes its git identity through GIT_CONFIG_GLOBAL pointed at
# a throwaway (mktemp'd) file, the mechanism that replaced the env vars.
scopes_identity_via_throwaway_git_config_global() {
  uncommented "$1" | grep -qE 'GIT_CONFIG_GLOBAL="\$\(mktemp\)"' \
    && uncommented "$1" | grep -qE '^\s*export GIT_CONFIG_GLOBAL\s*$' \
    && uncommented "$1" | grep -qE 'git config --global user\.email' \
    && uncommented "$1" | grep -qE 'git config --global user\.name'
}

# A: the real workflow file sets neither GIT_AUTHOR_* nor GIT_COMMITTER_*.
if sets_author_or_committer_env "$WORKFLOW"; then
  fail "shell-ci.yml sets no GIT_AUTHOR_*/GIT_COMMITTER_*" \
    "$(uncommented "$WORKFLOW" | grep -E '\bGIT_(AUTHOR|COMMITTER)_(NAME|EMAIL)\b')"
else
  pass "shell-ci.yml sets no GIT_AUTHOR_*/GIT_COMMITTER_*"
fi

# B: the real workflow file scopes its identity through a throwaway
# GIT_CONFIG_GLOBAL rather than the repo's own .git/config.
if scopes_identity_via_throwaway_git_config_global "$WORKFLOW"; then
  pass "shell-ci.yml scopes its git identity via a throwaway GIT_CONFIG_GLOBAL"
else
  fail "shell-ci.yml scopes its git identity via a throwaway GIT_CONFIG_GLOBAL" \
    "expected GIT_CONFIG_GLOBAL=\"\$(mktemp)\", export GIT_CONFIG_GLOBAL, and \
git config --global user.email/user.name"
fi

# C: the check itself actually catches the regression, not just the absence
# of one today. A fixture copy with the defect reinstated (a step-level
# GIT_AUTHOR_NAME env var, the exact shape that broke worktree.test.sh
# scenario M) must fail scenario A's check.
FIXTURE_REGRESSED="${WORK}/shell-ci-regressed.yml"
{
  cat "$WORKFLOW"
  printf '\n      - name: Run behavioral test suites\n'
  printf '        env:\n'
  printf '          GIT_AUTHOR_NAME: CI\n'
} > "$FIXTURE_REGRESSED"
if sets_author_or_committer_env "$FIXTURE_REGRESSED"; then
  pass "a reinstated GIT_AUTHOR_NAME step env var is caught"
else
  fail "a reinstated GIT_AUTHOR_NAME step env var is caught" \
    "sets_author_or_committer_env did not flag the regressed fixture"
fi

# D: worktree.test.sh's own legitimate inline use of GIT_AUTHOR_*/
# GIT_COMMITTER_* (scenario M's fixture) is out of scope for this check,
# since it is scoped to the workflow file argument only, never to the repo.
WORKTREE_TEST="${SCRIPT_DIR}/worktree.test.sh"
if [ -f "$WORKTREE_TEST" ] && grep -q 'GIT_AUTHOR_NAME=' "$WORKTREE_TEST"; then
  pass "worktree.test.sh's own inline identity is untouched by this check"
else
  fail "worktree.test.sh's own inline identity is untouched by this check" \
    "expected shell/worktree.test.sh to still set GIT_AUTHOR_NAME= inline"
fi

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ "$FAIL" -eq 0 ]]
