#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Behavioral tests for precommit-check.sh.
#
# The guard reads a Bash tool-call JSON on stdin and, when the command is a git
# commit with a problematic staged diff, emits an additionalContext warning.
# "warn" when it prints JSON; "quiet" when it prints nothing. It must never
# block, so a deny decision is always a failure.
#
# Each case runs inside a throwaway git repo so the staged diff is controlled.
#
# Run:  bash hooks/precommit-check.test.sh
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
GUARD="$HERE/precommit-check.sh"
pass=0
fail=0
WORK=""

json_str() {
  local s="$1"
  s="${s//\\/\\\\}"
  s="${s//\"/\\\"}"
  printf '"%s"' "$s"
}

new_repo() {
  WORK="$(mktemp -d)"
  git -C "$WORK" init -q
  git -C "$WORK" config user.email t@example.com
  git -C "$WORK" config user.name Test
  git -C "$WORK" config commit.gpgsign false
}

drop_repo() {
  [[ -n "$WORK" && -d "$WORK" ]] && rm -rf "$WORK"
  WORK=""
}

# run <expect: warn|quiet> <label> <command-string>
run() {
  local expect="$1" label="$2" cmd="$3" out got
  out="$(cd "$WORK" && printf '{"tool_input":{"command":%s}}' "$(json_str "$cmd")" | bash "$GUARD" 2>/dev/null)"
  got="quiet"
  [[ -n "$out" ]] && got="warn"
  if printf '%s' "$out" | grep -q '"permissionDecision"[[:space:]]*:[[:space:]]*"deny"'; then
    fail=$((fail + 1))
    printf 'FAIL: guard must never block, but denied: %s\n' "$label" >&2
    return
  fi
  if [[ "$got" == "$expect" ]]; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    printf 'FAIL: expected %s, got %s for: %s\n' "$expect" "$got" "$label" >&2
  fi
}

# --- Quiet: not a commit at all ---
new_repo
echo "hello" > "$WORK/a.txt"
git -C "$WORK" add a.txt
run quiet "git log is not a commit" "git log --oneline -5"
run quiet "git status is not a commit" "git status"
run quiet "commit --help is not a commit" "git commit --help"
run quiet "clean small staged diff" "git commit -m 'feat: add a'"
drop_repo

# --- Quiet: nothing staged ---
new_repo
echo "untracked" > "$WORK/b.txt"
run quiet "nothing staged" "git commit -m 'feat: nothing'"
drop_repo

# --- Warn: secret-shaped files ---
new_repo
echo "TOKEN=abc" > "$WORK/.env"
git -C "$WORK" add -f .env
run warn "staged .env" "git commit -m 'chore: env'"
drop_repo

new_repo
echo "key" > "$WORK/server.pem"
git -C "$WORK" add server.pem
run warn "staged .pem" "git commit -m 'chore: cert'"
drop_repo

# --- Warn: debug leftovers in added lines ---
new_repo
printf 'function f() {\n  console.log("dbg");\n}\n' > "$WORK/app.js"
git -C "$WORK" add app.js
run warn "console.log added" "git commit -m 'feat: app'"
drop_repo

new_repo
printf 'def f():\n    breakpoint()\n' > "$WORK/app.py"
git -C "$WORK" add app.py
run warn "breakpoint added" "git commit -m 'feat: app'"
drop_repo

# --- Warn: oversized commit by line count ---
new_repo
for i in $(seq 1 700); do echo "line $i" >> "$WORK/big.txt"; done
git -C "$WORK" add big.txt
run warn "over 600 changed lines" "git commit -m 'feat: big'"
drop_repo

# --- Warn: oversized commit by file count ---
new_repo
for i in $(seq 1 25); do echo "x" > "$WORK/f$i.txt"; done
git -C "$WORK" add .
run warn "over 20 files" "git commit -m 'feat: many'"
drop_repo

# --- Quiet: disabled by env ---
new_repo
echo "TOKEN=abc" > "$WORK/.env"
git -C "$WORK" add -f .env
out="$(cd "$WORK" && printf '{"tool_input":{"command":"git commit -m x"}}' | PRECOMMIT_CHECK=0 bash "$GUARD" 2>/dev/null)"
if [[ -z "$out" ]]; then
  pass=$((pass + 1))
else
  fail=$((fail + 1))
  printf 'FAIL: PRECOMMIT_CHECK=0 should silence the guard\n' >&2
fi
drop_repo

printf 'precommit-check: %d passed, %d failed\n' "$pass" "$fail"
[[ "$fail" -eq 0 ]]
