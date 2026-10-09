#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# install-hooks-fire.test.sh: WU-11's headline acceptance criterion. Installs
# into a scratch HOME via install.sh (the PLAYBOOK_SRC local-source seam, no
# network), reads the command strings the resulting settings.json carries,
# asserts exactly 18 distinct hook names (13 functional hooks and 5 safety
# guards, all wired as `playbook hook <name>`), then
# EXECUTES every one of them with a hook-specific payload and asserts a
# hook-specific observable effect. The name list is derived from settings.json
# itself, not hardcoded: a name present there with no case below FAILS rather
# than being silently skipped, which is what stops this matrix shrinking back
# to 12 (or fewer) without anyone noticing.
#
# Needs a `playbook` binary: builds it with `cargo build` (debug profile) if
# target/debug/playbook is not already present.
#
# Run:  bash shell/install-hooks-fire.test.sh
set -u

# A CI runner sets CI=true, which means headless; these hooks must run as a person would see them.
export PLAYBOOK_HEADLESS=0

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALL="$REPO_ROOT/install.sh"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; PASS=$((PASS + 1)); }
fail() { echo "FAIL: $1"; FAIL=$((FAIL + 1)); }

command -v cargo >/dev/null 2>&1 || { echo "cargo not found on PATH" >&2; exit 2; }

BIN_SRC="$REPO_ROOT/target/debug/playbook"
if [ ! -x "$BIN_SRC" ]; then
  echo "Building playbook (cargo build)..."
  ( cd "$REPO_ROOT" && cargo build --quiet ) || { echo "cargo build failed" >&2; exit 2; }
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

TH="$WORK/home"; CH="$TH/.claude"; BIN_DIR="$WORK/bin"
mkdir -p "$TH" "$BIN_DIR"
cp "$BIN_SRC" "$BIN_DIR/playbook"
chmod 0755 "$BIN_DIR/playbook"
PLAYBOOK="$BIN_DIR/playbook"

HOME="$TH" CLAUDE_HOME="$CH" PLAYBOOK_SRC="$REPO_ROOT" PLAYBOOK_BIN_DIR="$BIN_DIR" \
  bash "$INSTALL" --yes --skip-plugin >"$WORK/install.log" 2>&1
rc=$?
if [ "$rc" -eq 0 ]; then
  pass "install.sh --yes --skip-plugin exits 0"
else
  fail "install.sh exit=$rc"
  cat "$WORK/install.log"
fi

SETTINGS="$CH/settings.json"
[ -f "$SETTINGS" ] || { fail "settings.json not written by install"; cat "$WORK/install.log"; }

# --- 1. exactly 18 distinct hook names -------------------------------------
# Ported hooks: bare `playbook hook <name>`. Guards: `~/.claude/hooks/<name>.sh`.
# Needs the raw command strings themselves (to strip down to bare names below),
# not a count, so this uses the raw hook-commands listing rather than a
# hook-commands-matching count.
NAMES_FILE="$WORK/names.txt"
"$PLAYBOOK" doctor hook-commands "$SETTINGS" 2>/dev/null \
  | sed -E \
      -e 's#^playbook hook ([a-z0-9-]+)$#\1#' \
      -e 's#^~/\.claude/hooks/([a-z0-9-]+)\.sh$#\1#' \
  | sort -u > "$NAMES_FILE"

n_names="$(wc -l < "$NAMES_FILE" | tr -d ' ')"
if [ "${n_names:-0}" -eq 18 ]; then
  pass "settings.json wires exactly 18 distinct hook names"
else
  fail "expected 18 distinct hook names, got ${n_names:-0}: $(tr '\n' ' ' < "$NAMES_FILE")"
fi

# --- 2. execute every one and assert its observable -------------------------

json_str() {  # escape <string> as a JSON string literal
  local s="$1"
  s="${s//\\/\\\\}"
  s="${s//\"/\\\"}"
  printf '"%s"' "$s"
}

# run_ported <name> <home> <payload>: fires a ported hook through the real
# binary, the same way settings.json's bare `playbook hook <name>` resolves
# it on PATH.
run_ported() {
  printf '%s' "$3" | HOME="$2" "$PLAYBOOK" hook "$1" 2>/dev/null
}

# run_guard <name> <payload> [cwd]: fires a guard through the real binary,
# the same way settings.json's bare `playbook hook <name>` resolves it.
# Guards have been wired this way since WU-13 (v0.11.0), not as
# `~/.claude/hooks/<name>.sh`. HOME fixed at the install target so guard
# logic that reads HOME (e.g. rm-workspace-guard's safe roots) sees where
# install.sh actually placed things.
run_guard() {
  local name="$1" payload="$2" cwd="${3:-$TH}"
  ( cd "$cwd" && printf '%s' "$payload" | HOME="$TH" "$PLAYBOOK" hook "$name" 2>/dev/null )
}

test_session_init() {
  local sid="fire-session-init" home f
  home="$(mktemp -d)"
  run_ported session-init "$home" "{\"session_id\":\"$sid\"}" >/dev/null
  f="$home/.config/playbook/runtime/$sid/start-ts"
  [ -s "$f" ] && grep -qE '^[0-9]+$' "$f"
}

test_preread_edit_check() {
  local sid="fire-preread-edit" home f ts out
  home="$(mktemp -d)"
  # Canonicalize: abspath() resolves a real path via the filesystem (on
  # macOS, /var/folders/... is a symlink to /private/var/folders/...), so
  # edits.jsonl must store the same canonical form the hook will compare
  # against, or a real match looks like a miss.
  home="$(cd "$home" && pwd -P)"
  f="$home/target.txt"
  printf 'hello\n' > "$f"
  mkdir -p "$home/.config/playbook/runtime/$sid"
  ts=$(( $(date +%s) - 10 ))
  printf '{"path":"%s","ts":%s}\n' "$f" "$ts" > "$home/.config/playbook/runtime/$sid/edits.jsonl"
  out="$(run_ported preread-edit-check "$home" "{\"session_id\":\"$sid\",\"tool_input\":{\"file_path\":\"$f\"}}")"
  [[ "$out" == *"ago"* ]]
}

test_preread_size_check() {
  local home f out
  home="$(mktemp -d)"
  f="$home/big.log"
  seq 1 1500 > "$f"
  out="$(run_ported preread-size-check "$home" "{\"tool_input\":{\"file_path\":\"$f\"}}")"
  [[ "$out" == *'"permissionDecision":"deny"'* && "$out" == *"1500 lines"* ]]
}

test_search_counter() {
  local sid="fire-search-counter" home payload
  home="$(mktemp -d)"
  payload="{\"session_id\":\"$sid\",\"tool_name\":\"Grep\"}"
  for _ in 1 2 3 4; do run_ported search-counter "$home" "$payload" >/dev/null; done
  [ "$(cat "$home/.config/playbook/runtime/$sid/search-count" 2>/dev/null)" = "4" ]
}

test_memory_anchors() {
  local sid="fire-memory-anchors" home target out
  home="$(mktemp -d)"
  mkdir -p "$home/.config/playbook/memory"
  target="$REPO_ROOT/install.sh"
  cat > "$home/.config/playbook/memory/memory.graph.json" <<EOF
{
  "nodes": [
    {"id": "global/fire-anchor-fact", "file": "fire-anchor-fact.md", "scope": "global", "type": "project", "name": "fire-anchor-fact", "description": "fixture fact"},
    {"id": "code:install.sh", "file": "install.sh", "scope": "code", "type": "code"}
  ],
  "edges": [
    {"from": "global/fire-anchor-fact", "to": "code:install.sh", "relation": "anchors"}
  ]
}
EOF
  out="$(cd "$REPO_ROOT" && run_ported memory-anchors "$home" "{\"session_id\":\"$sid\",\"tool_name\":\"Edit\",\"tool_input\":{\"file_path\":\"$target\"}}")"
  [[ "$out" == *"fire-anchor-fact"* ]]
}

test_post_edit_track() {
  local sid="fire-post-edit-track" home
  home="$(mktemp -d)"
  run_ported post-edit-track "$home" "{\"session_id\":\"$sid\",\"tool_name\":\"Edit\",\"tool_input\":{\"file_path\":\"/tmp/fire-x.txt\"}}" >/dev/null
  [ "$(cat "$home/.config/playbook/runtime/$sid/edit-count" 2>/dev/null)" = "1" ]
}

test_rebuild_memory_graph() {
  local home f
  home="$(mktemp -d)"
  mkdir -p "$home/.config/playbook/memory"
  f="$home/.config/playbook/memory/fire-fact.md"
  cat > "$f" <<'EOF'
---
name: fire-fact
type: reference
links:
  relates_to: fire-other
---

Body text.
EOF
  run_ported rebuild-memory-graph "$home" "{\"tool_input\":{\"file_path\":\"$f\"}}" >/dev/null
  [ -f "$home/.config/playbook/memory/memory.graph.json" ] \
    && [ "$("$PLAYBOOK" json field-length edges < "$home/.config/playbook/memory/memory.graph.json" 2>/dev/null)" = "1" ]
}

test_auto_model_detect() {
  local home out
  home="$(mktemp -d)"
  out="$(run_ported auto-model-detect "$home" '{"prompt":"Should we design a new schema and evaluate the tradeoffs between the two approaches?"}')"
  [[ -n "$out" && "$out" == *"UserPromptSubmit"* ]]
}

test_precompact_warn() {
  local home out
  home="$(mktemp -d)"
  out="$(run_ported precompact-warn "$home" '{"trigger":"auto","session_id":"fire-precompact"}')"
  [[ "$out" == *"(auto)"* ]]
}

test_session_clean_exit() {
  local sid="fire-clean-exit" home
  home="$(mktemp -d)"
  run_ported session-clean-exit "$home" "{\"session_id\":\"$sid\",\"reason\":\"logout\"}" >/dev/null
  [ "$(cat "$home/.config/playbook/runtime/$sid/clean-exit" 2>/dev/null)" = "logout" ]
}

test_memory_capture() {
  local sid="fire-memory-capture" home dir out
  home="$(mktemp -d)"
  dir="$home/.config/playbook/runtime/$sid"
  mkdir -p "$dir"
  : > "$dir/capture-due"
  out="$(run_ported memory-capture "$home" "{\"session_id\":\"$sid\"}")"
  printf '%s' "$out" | "$PLAYBOOK" json field-equals decision block >/dev/null 2>&1
}

test_rm_workspace_guard() {
  local out
  out="$(run_guard rm-workspace-guard '{"tool_input":{"command":"rm -rf /etc/hosts"}}')"
  printf '%s' "$out" | "$PLAYBOOK" json field-equals hookSpecificOutput.permissionDecision deny >/dev/null 2>&1
}

test_bg_await_guard() {
  local out
  out="$(run_guard bg-await-guard '{"tool_input":{"command":"npm install","run_in_background":true}}')"
  [ -n "$out" ]
}

test_no_slop_guard() {
  local emdash cmd payload out
  emdash="$(printf '\xe2\x80\x94')"
  cmd="git commit -m \"x ${emdash} y\""
  payload="$(printf '{"tool_input":{"command":%s}}' "$(json_str "$cmd")")"
  out="$(run_guard no-slop-guard "$payload")"
  [[ "$out" == *'"permissionDecision":"deny"'* ]]
}

test_commit_message_sanitizer() {
  local bad ok payload out
  bad="git commit -m 'feat: x' -m 'Claude-Session: https://claude.ai/code/session_01AbCdEfGhIjKlMnOpQr'"
  payload="$(printf '{"tool_input":{"command":%s}}' "$(json_str "$bad")")"
  out="$(run_guard commit-message-sanitizer "$payload")"
  [[ "$out" == *'"updatedInput"'* && "$out" != *"permissionDecision"* && "$out" != *"Claude-Session: https"* ]] || return 1
  ok="git commit -s -m 'feat: x' -m 'Refs: PLAT-1'"
  payload="$(printf '{"tool_input":{"command":%s}}' "$(json_str "$ok")")"
  out="$(run_guard commit-message-sanitizer "$payload")"
  [ -z "$out" ]
}

test_precommit_check() {
  local repo cmd payload out
  repo="$(mktemp -d)"
  git -C "$repo" init -q
  git -C "$repo" config user.email t@example.com
  git -C "$repo" config user.name Test
  git -C "$repo" config commit.gpgsign false
  echo "TOKEN=abc" > "$repo/.env"
  git -C "$repo" add -f .env
  cmd="git commit -m 'chore: env'"
  payload="$(printf '{"tool_input":{"command":%s}}' "$(json_str "$cmd")")"
  out="$(run_guard precommit-check "$payload" "$repo")"
  rm -rf "$repo"
  [ -n "$out" ] && [[ "$out" != *'"permissionDecision":"deny"'* ]]
}

# run_auto_hook <name> <mode> <payload>: fires an auto-mode hook with the mode
# env var unset (empty <mode>) or set to <mode>, in a scratch HOME so no real
# config can flip the result. Stdout goes to $AUTO_OUT; returns the exit code.
AUTO_OUT="$WORK/auto-hook.out"
run_auto_hook() {
  local name="$1" mode="$2" payload="$3" home rc
  local -a mode_env=()
  [ -n "$mode" ] && mode_env=(PLAYBOOK_MODE="$mode")
  home="$(mktemp -d)"
  ( cd "$home" && printf '%s' "$payload" \
      | env -u PLAYBOOK_MODE -u XDG_CONFIG_HOME HOME="$home" "${mode_env[@]+"${mode_env[@]}"}" \
          "$PLAYBOOK" hook "$name" 2>/dev/null ) > "$AUTO_OUT"
  rc=$?
  rm -rf "$home"
  return "$rc"
}

test_auto_guard() {
  local ask_payload prompt_payload
  ask_payload='{"session_id":"fire-auto-guard","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_input":{"questions":[]}}'
  prompt_payload='{"session_id":"fire-auto-guard","hook_event_name":"UserPromptSubmit","prompt":"hi"}'
  # ask mode (nothing set): stays silent
  run_auto_hook auto-guard "" "$ask_payload" || return 1
  [ ! -s "$AUTO_OUT" ] || return 1
  # auto mode: denies the question and adds the standing note to a prompt
  run_auto_hook auto-guard auto "$ask_payload" || return 1
  [[ "$(cat "$AUTO_OUT")" == *'"permissionDecision":"deny"'* ]] || return 1
  run_auto_hook auto-guard auto "$prompt_payload" || return 1
  [[ "$(cat "$AUTO_OUT")" == *"AUTO MODE is on"* ]]
}

test_auto_cost() {
  local payload
  payload='{"session_id":"fire-auto-cost","hook_event_name":"PreToolUse","tool_name":"Agent","tool_input":{"prompt":"x"}}'
  # ask mode (nothing set): stays silent
  run_auto_hook auto-cost "" "$payload" || return 1
  [ ! -s "$AUTO_OUT" ] || return 1
  # auto mode with no transcript the spend is unreadable, so a non-read tool is denied
  run_auto_hook auto-cost auto "$payload" || return 1
  [[ "$(cat "$AUTO_OUT")" == *'"permissionDecision":"deny"'* && "$(cat "$AUTO_OUT")" == *"cost unreadable"* ]]
}

run_case_for() {  # <name>: dispatches to the matching test_*, or returns 9
  case "$1" in
    session-init)         test_session_init ;;
    preread-edit-check)   test_preread_edit_check ;;
    preread-size-check)   test_preread_size_check ;;
    search-counter)       test_search_counter ;;
    memory-anchors)       test_memory_anchors ;;
    post-edit-track)      test_post_edit_track ;;
    rebuild-memory-graph) test_rebuild_memory_graph ;;
    auto-model-detect)    test_auto_model_detect ;;
    precompact-warn)      test_precompact_warn ;;
    session-clean-exit)   test_session_clean_exit ;;
    memory-capture)       test_memory_capture ;;
    rm-workspace-guard)   test_rm_workspace_guard ;;
    bg-await-guard)       test_bg_await_guard ;;
    no-slop-guard)        test_no_slop_guard ;;
    commit-message-sanitizer) test_commit_message_sanitizer ;;
    precommit-check)      test_precommit_check ;;
    auto-guard)           test_auto_guard ;;
    auto-cost)            test_auto_cost ;;
    *) return 9 ;;
  esac
}

while IFS= read -r n; do
  [ -z "$n" ] && continue
  run_case_for "$n"
  rc=$?
  if [ "$rc" -eq 9 ]; then
    fail "no fire-matrix case registered for hook: $n"
  elif [ "$rc" -eq 0 ]; then
    pass "$n fires and produces its observable effect"
  else
    fail "$n fires and produces its observable effect"
  fi
done < "$NAMES_FILE"

TOTAL=$((PASS + FAIL))
echo ""
echo "${PASS}/${TOTAL} cases passed"
[ "$FAIL" -eq 0 ]
