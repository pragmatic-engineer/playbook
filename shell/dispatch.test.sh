#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# dispatch.test.sh: flag-parser and subcommand-routing tests for
# shell/zsh/dispatch.zsh (_claude). Covers value-taking flag consumption,
# --opt=value self-contained form, subcommand dispatch, and residual arg
# forwarding.
#
# Run:  bash shell/dispatch.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENGINE="${SCRIPT_DIR}/zsh/dispatch.zsh"
PASS=0
FAIL=0
TOTAL=4

if ! command -v zsh >/dev/null 2>&1; then
  echo "SKIP: zsh not available; dispatch.zsh tests need zsh"
  exit 0
fi

# ── Shared fixtures ──────────────────────────────────────────────────────────
TMP="$(mktemp -d)"
# shellcheck disable=SC2064
trap "rm -rf '$TMP'" EXIT INT TERM

# claude shim: prepended to PATH; records "claude arg1 arg2 ..." to the
# per-scenario record file passed via $DISPATCH_RECORD (env var).
mkdir -p "$TMP/bin"
cat > "$TMP/bin/claude" << 'SHIM'
#!/bin/sh
printf 'claude' >> "$DISPATCH_RECORD"
for a in "$@"; do printf ' %s' "$a"; done >> "$DISPATCH_RECORD"
printf '\n' >> "$DISPATCH_RECORD"
exit 0
SHIM
chmod +x "$TMP/bin/claude"

# Collaborator stubs sourced into each zsh invocation.
# Each stub appends its name + args to $DISPATCH_RECORD and produces no stdout,
# so command-substitution callers (e.g. raw_sid=$(...)) receive an empty string.
cat > "$TMP/stubs.zsh" << 'STUBS'
_cc_bust_cache()            { printf '%s\n' '_cc_bust_cache'                    >> "$DISPATCH_RECORD"; }
_cc_config_stamp()          { printf '%s\n' '_cc_config_stamp'                  >> "$DISPATCH_RECORD"; }
_cc_config_drifted()        { printf '%s\n' '_cc_config_drifted'                >> "$DISPATCH_RECORD"; return 1; }
_cc_find_session_by_title() { printf '_cc_find_session_by_title %s\n' "$*"      >> "$DISPATCH_RECORD"; }
_cc_clean_resume()          { { printf '_cc_clean_resume'; for a in "$@"; do printf ' %s' "$a"; done; printf '\n'; } >> "$DISPATCH_RECORD"; }
_cc_list_sessions()         { printf '_cc_list_sessions %s\n' "$1"              >> "$DISPATCH_RECORD"; }
_cc_worktree()              { { printf '_cc_worktree'; for a in "$@"; do printf ' %s' "$a"; done; printf '\n'; } >> "$DISPATCH_RECORD"; return 0; }
clear()                     { : ; }
STUBS

# ── Helpers ──────────────────────────────────────────────────────────────────
run_scenario() {
  local name="$1" fn="$2"
  if "$fn" 2>&1; then
    echo "PASS: $name"
    (( PASS++ )) || true
  else
    echo "FAIL: $name"
    (( FAIL++ )) || true
  fi
}

# Invoke _claude inside a fresh zsh process with the shim on PATH and stubs loaded.
# Usage: invoke_claude <rec_file> <home_dir> [_claude args...]
invoke_claude() {
  local rec="$1" home="$2"
  shift 2
  # shellcheck disable=SC2016  # $1/$2/$@ are for the zsh subshell, not bash
  DISPATCH_RECORD="$rec" HOME="$home" PATH="$TMP/bin:$PATH" \
    zsh -c 'source "$1"; source "$2"; shift 2; _claude "$@"' \
    _ "$ENGINE" "$TMP/stubs.zsh" "$@"
}

# ── Scenario 1 ───────────────────────────────────────────────────────────────
# _claude --system-prompt-file /tmp/p fresh
#
# fresh path: _cc_config_stamp fires; claude receives --system-prompt-file /tmp/p
# and then -n <name> (proving /tmp/p was consumed as the flag's value so -n is
# NOT swallowed — the swallow bug is documented at dispatch.zsh:27-31).
scenario_system_prompt_fresh() {
  local rec="$TMP/s1_record" home="$TMP/s1_home"
  mkdir -p "$home"
  : > "$rec"
  invoke_claude "$rec" "$home" --system-prompt-file /tmp/p fresh

  grep -q '_cc_config_stamp' "$rec" \
    || { echo "  _cc_config_stamp not fired"; return 1; }
  grep -q -- '--system-prompt-file /tmp/p' "$rec" \
    || { echo "  claude not called with --system-prompt-file /tmp/p"; return 1; }
  # -n must appear AFTER --system-prompt-file /tmp/p (not swallowed as the flag's value)
  grep -qE 'claude.*--system-prompt-file /tmp/p.* -n ' "$rec" \
    || { echo "  -n not found after --system-prompt-file /tmp/p (value swallowed?)"; return 1; }
}

# ── Scenario 2 ───────────────────────────────────────────────────────────────
# _claude --model=haiku list
#
# --opt=value kept as a single token (self-contained form matched first); list
# path taken → _cc_list_sessions fires; claude is NOT invoked (list only
# prints sessions). Correct parse of --model=haiku is proved implicitly: if the
# token were split, "haiku" would be mistaken for the subcommand and
# _cc_list_sessions would not fire.
scenario_model_equals_list() {
  local rec="$TMP/s2_record" home="$TMP/s2_home"
  mkdir -p "$home"
  : > "$rec"
  invoke_claude "$rec" "$home" --model=haiku list

  grep -q '_cc_list_sessions' "$rec" \
    || { echo "  _cc_list_sessions not fired"; return 1; }
  ! grep -q '^claude' "$rec" \
    || { echo "  claude was unexpectedly invoked for the list path"; return 1; }
}

# ── Scenario 3 ───────────────────────────────────────────────────────────────
# _claude clean extra
#
# clean path: _cc_clean_resume fires and receives the residual positional arg.
scenario_clean_residual() {
  local rec="$TMP/s3_record" home="$TMP/s3_home"
  mkdir -p "$home"
  : > "$rec"
  invoke_claude "$rec" "$home" clean extra

  grep -q '_cc_clean_resume' "$rec" \
    || { echo "  _cc_clean_resume not fired"; return 1; }
  grep -qE '_cc_clean_resume.* extra' "$rec" \
    || { echo "  residual arg 'extra' not passed to _cc_clean_resume"; return 1; }
}

# ── Scenario 4 ───────────────────────────────────────────────────────────────
# _claude -n custom raw
#
# -n consumes 'custom' as its value (not the subcommand name); raw path taken →
# _cc_find_session_by_title fires (stub returns empty → no-session branch);
# claude is invoked with -n custom preserved in its argv.
scenario_n_value_raw() {
  local rec="$TMP/s4_record" home="$TMP/s4_home"
  mkdir -p "$home"
  : > "$rec"
  invoke_claude "$rec" "$home" -n custom raw

  grep -q '_cc_find_session_by_title' "$rec" \
    || { echo "  _cc_find_session_by_title not fired"; return 1; }
  grep -qE 'claude.* -n custom' "$rec" \
    || { echo "  claude not called with -n custom in argv"; return 1; }
}

# ── Run all scenarios ─────────────────────────────────────────────────────────
run_scenario "system-prompt-file consumes value; fresh path fires _cc_config_stamp" scenario_system_prompt_fresh
run_scenario "--model=haiku kept whole; list path fires _cc_list_sessions"           scenario_model_equals_list
run_scenario "clean path forwards residual arg to _cc_clean_resume"                  scenario_clean_residual
run_scenario "-n consumes value; raw path fires _cc_find_session_by_title"           scenario_n_value_raw

echo "${PASS}/${TOTAL} scenarios passed"
[[ $FAIL -eq 0 ]]
