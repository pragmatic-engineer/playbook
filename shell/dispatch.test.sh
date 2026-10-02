#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# dispatch.test.sh: flag-parser and subcommand-routing tests for
# shell/shared/dispatch.sh (_claude). Covers value-taking flag consumption,
# --opt=value self-contained form, subcommand dispatch, and residual arg
# forwarding.
#
# Run:  bash shell/dispatch.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENGINE="${SCRIPT_DIR}/shared/dispatch.sh"
PASS=0
FAIL=0
TOTAL=8

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

# playbook stub: records "playbook <args>" to the same record file, so the
# order of trust and claude calls is provable. $PLAYBOOK_STUB_MODE picks the
# behaviour: ok (default) or old (a binary with no `trust` subcommand).
cat > "$TMP/bin/playbook" << 'SHIM'
#!/bin/sh
printf 'playbook' >> "$DISPATCH_RECORD"
for a in "$@"; do printf ' %s' "$a"; done >> "$DISPATCH_RECORD"
printf '\n' >> "$DISPATCH_RECORD"
case "${PLAYBOOK_STUB_MODE:-ok}" in
  old) echo "error: unrecognized subcommand 'trust'" >&2; exit 2 ;;
esac
exit 0
SHIM
chmod +x "$TMP/bin/playbook"

# A PATH with the claude shim but no playbook anywhere on it.
mkdir -p "$TMP/nopb"
cp "$TMP/bin/claude" "$TMP/nopb/claude"
ZSH_BIN="$(command -v zsh)"

# Collaborator stubs sourced into each zsh invocation.
# Each stub appends its name + args to $DISPATCH_RECORD and produces no stdout,
# so command-substitution callers (e.g. raw_sid=$(...)) receive an empty string.
cat > "$TMP/stubs.zsh" << 'STUBS'
_cc_bust_cache()            { printf '%s\n' '_cc_bust_cache'                    >> "$DISPATCH_RECORD"; }
_cc_config_stamp()          { printf '%s\n' '_cc_config_stamp'                  >> "$DISPATCH_RECORD"; }
_cc_config_drifted()        { printf '%s\n' '_cc_config_drifted'                >> "$DISPATCH_RECORD"; return 1; }
_cc_find_session_by_title() { printf '_cc_find_session_by_title %s\n' "$*"      >> "$DISPATCH_RECORD"; [ -z "${DISPATCH_SESSION_ID:-}" ] || printf '%s\n' "$DISPATCH_SESSION_ID"; }
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
# NOT swallowed: the swallow bug is documented at dispatch.zsh:27-31).
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

# ── Trust scenarios ──────────────────────────────────────────────────────────
# Run _claude from a real, physical launch directory so PWD is predictable.
# Usage: launch_in <dir> <rec_file> <home_dir> [_claude args...]
launch_in() {
  local dir="$1"
  shift
  ( cd "$dir" && invoke_claude "$@" )
}

new_launch_dir() {
  local d="$TMP/launch_$1"
  mkdir -p "$d"
  ( cd "$d" && pwd -P )
}

line_of() { grep -n "$1" "$2" | head -1 | cut -d: -f1; }

# ── Scenario 5 ───────────────────────────────────────────────────────────────
# A plain launch calls `playbook trust` with exactly the launch directory,
# and records it BEFORE claude starts (one shared record, so order is real).
scenario_trust_dir_and_order() {
  local rec="$TMP/s5_record" home="$TMP/s5_home" dir
  dir="$(new_launch_dir s5)"
  mkdir -p "$home"; : > "$rec"
  launch_in "$dir" "$rec" "$home"

  [ "$(grep -c '^playbook trust ' "$rec")" = "1" ] \
    || { echo "  expected exactly one trust call"; cat "$rec"; return 1; }
  grep -qxF "playbook trust $dir" "$rec" \
    || { echo "  trust not called with exactly $dir"; cat "$rec"; return 1; }
  local t c
  t="$(line_of '^playbook trust ' "$rec")"; c="$(line_of '^claude' "$rec")"
  [ -n "$t" ] && [ -n "$c" ] && [ "$t" -lt "$c" ] \
    || { echo "  trust (line ${t:-none}) not before claude (line ${c:-none})"; return 1; }
}

# ── Scenario 6 ───────────────────────────────────────────────────────────────
# Every launch path trusts exactly once; paths that launch nothing never do.
# worktree/new trusts only in the re-entered _claude, so it is one call too.
scenario_trust_once_per_path() {
  local home="$TMP/s6_home" dir rec n path_args
  mkdir -p "$home"
  dir="$(new_launch_dir s6)"
  local cases=(
    "fresh|fresh|"
    "clean|clean|"
    "raw|raw|"
    "no-resume||"
    "default-resume||sess-1"
    "worktree|new|"
  )
  local entry label args sid
  for entry in "${cases[@]}"; do
    label="${entry%%|*}"; entry="${entry#*|}"
    args="${entry%%|*}"; sid="${entry#*|}"
    rec="$TMP/s6_${label}_record"; : > "$rec"
    # shellcheck disable=SC2086  # $args is a single optional word on purpose
    DISPATCH_SESSION_ID="$sid" launch_in "$dir" "$rec" "$home" $args
    n="$(grep -c '^playbook trust ' "$rec")"
    [ "$n" = "1" ] || { echo "  $label: expected 1 trust call, got $n"; cat "$rec"; return 1; }
    grep -qxF "playbook trust $dir" "$rec" \
      || { echo "  $label: wrong trust directory"; cat "$rec"; return 1; }
  done
  for path_args in list prune; do
    rec="$TMP/s6_${path_args}_record"; : > "$rec"
    launch_in "$dir" "$rec" "$home" "$path_args"
    ! grep -q '^playbook trust ' "$rec" \
      || { echo "  $path_args launches nothing and must not trust"; return 1; }
  done
}

# ── Scenario 7 ───────────────────────────────────────────────────────────────
# A binary with no `trust` subcommand (plugin updated before the binary):
# silent, and the launch still happens.
scenario_trust_old_binary_silent() {
  local rec="$TMP/s7_record" home="$TMP/s7_home" dir out
  dir="$(new_launch_dir s7)"
  mkdir -p "$home"; : > "$rec"
  out="$(PLAYBOOK_STUB_MODE=old launch_in "$dir" "$rec" "$home" 2>&1)"

  [ -z "$out" ] || { echo "  expected no output, got: $out"; return 1; }
  grep -q '^claude' "$rec" || { echo "  claude was not launched"; return 1; }
}

# ── Scenario 9 ───────────────────────────────────────────────────────────────
# No playbook on PATH at all: the launch is untouched and nothing is printed.
scenario_trust_missing_playbook() {
  local rec="$TMP/s9_record" home="$TMP/s9_home" dir out
  dir="$(new_launch_dir s9)"
  mkdir -p "$home"; : > "$rec"
  if PATH="$TMP/nopb:/usr/bin:/bin" command -v playbook >/dev/null 2>&1; then
    echo "  SKIP: a playbook binary sits in /usr/bin or /bin on this host"
    return 0
  fi
  # shellcheck disable=SC2016  # $1/$2/$@ are for the zsh subshell, not bash
  out="$(cd "$dir" && DISPATCH_RECORD="$rec" HOME="$home" PATH="$TMP/nopb:/usr/bin:/bin" \
    "$ZSH_BIN" -c 'source "$1"; source "$2"; shift 2; _claude "$@"' \
    _ "$ENGINE" "$TMP/stubs.zsh" 2>&1)"

  [ -z "$out" ] || { echo "  expected no output, got: $out"; return 1; }
  grep -q '^claude' "$rec" || { echo "  claude was not launched"; return 1; }
}

# ── Run all scenarios ─────────────────────────────────────────────────────────
run_scenario "system-prompt-file consumes value; fresh path fires _cc_config_stamp" scenario_system_prompt_fresh
run_scenario "--model=haiku kept whole; list path fires _cc_list_sessions"           scenario_model_equals_list
run_scenario "clean path forwards residual arg to _cc_clean_resume"                  scenario_clean_residual
run_scenario "-n consumes value; raw path fires _cc_find_session_by_title"           scenario_n_value_raw
run_scenario "trust gets the launch dir, and runs before claude"                     scenario_trust_dir_and_order
run_scenario "every launch path trusts once; list and prune never do"                scenario_trust_once_per_path
run_scenario "a binary without trust is silent and the launch proceeds"              scenario_trust_old_binary_silent
run_scenario "no playbook on PATH: the launch is untouched"                          scenario_trust_missing_playbook

echo "${PASS}/${TOTAL} scenarios passed"
[[ $FAIL -eq 0 ]]
