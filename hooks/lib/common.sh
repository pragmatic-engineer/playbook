#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Shared helpers for Claude Code hooks.
# Source from each hook script:  . "$(dirname "$0")/lib/common.sh"
#
# Design rules:
#   - Hooks must never break Claude Code. Any uncaught error → exit 0 silent.
#   - Stdout must be either empty or a single valid JSON object.
#   - All file writes are atomic (tmp + mv) so concurrent hooks don't tear state.
#   - Per-session state lives in ~/.claude/runtime/<session_id>/.

set -u
umask 077

RUNTIME_ROOT="${HOME}/.claude/runtime"

# Read the entire JSON payload from stdin ONCE, in the parent shell's scope.
# This is critical: hi_field is called inside $(...) substitutions which run
# in subshells. Each subshell inherits HOOK_INPUT but cannot reach back into
# the parent. If we instead read stdin lazily inside hi_field, the first
# $(...) call would consume stdin and every subsequent call would see EOF.
HOOK_INPUT="${HOOK_INPUT:-}"
if [[ -z "$HOOK_INPUT" ]] && [[ ! -t 0 ]]; then
  HOOK_INPUT="$(cat 2>/dev/null || printf '')"
fi

# Extract a JSON field. Usage: hi_field '.tool_name'
hi_field() {
  if [[ -z "$HOOK_INPUT" ]] || ! command -v jq >/dev/null 2>&1; then
    return 0
  fi
  printf '%s' "$HOOK_INPUT" | jq -r "$1 // empty" 2>/dev/null
}

# Get session id from input. Empty string if absent.
hi_session_id() {
  hi_field '.session_id'
}

# Per-session state dir. Created on demand. Empty string return if no session id.
session_dir() {
  local sid; sid="$(hi_session_id)"
  [[ -z "$sid" ]] && { printf ''; return 0; }
  local dir="${RUNTIME_ROOT}/${sid}"
  [[ -d "$dir" ]] || mkdir -p "$dir" 2>/dev/null
  printf '%s' "$dir"
}

# Resolve a path to absolute, following symlinks. Tolerates non-existent paths
# by resolving the parent dir and re-appending the basename.
abspath() {
  local p="$1"
  [[ -z "$p" ]] && return 0
  # Pure-shell realpath via cd + `pwd -P` (resolves symlinks in the path prefix).
  # Avoids a python3 fork per call on the Read/Edit hot path; portable across
  # macOS and Linux. A leaf that is itself a symlink stays unresolved, which is
  # fine for edit-tracking dedup (we key on the path the tool referenced).
  if [[ -d "$p" ]]; then
    ( cd "$p" 2>/dev/null && pwd -P ) || printf '%s' "$p"
  else
    local d b
    d="$(dirname "$p")"; b="$(basename "$p")"
    if [[ -d "$d" ]]; then
      printf '%s/%s' "$( cd "$d" 2>/dev/null && pwd -P )" "$b"
    else
      printf '%s' "$p"
    fi
  fi
}

# Atomic append. flock to serialize concurrent hook instances.
atomic_append() {
  local file="$1" line="$2"
  local dir; dir="$(dirname "$file")"
  [[ -d "$dir" ]] || mkdir -p "$dir" 2>/dev/null
  # macOS lacks flock by default; fall back to a noop subshell.
  if command -v flock >/dev/null 2>&1; then
    ( flock -x 9; printf '%s\n' "$line" >> "$file" ) 9>>"$file.lock"
  else
    printf '%s\n' "$line" >> "$file"
  fi
}

# Emit a PreToolUse additionalContext (info shown to Claude only, no block).
# Usage: emit_pre_context "PreToolUse" "message text"
emit_pre_context() {
  local event="$1" msg="$2"
  jq -cn --arg ev "$event" --arg msg "$msg" '
    { hookSpecificOutput: { hookEventName: $ev, additionalContext: $msg } }
  '
}

# Emit a PreToolUse deny decision.
# Usage: emit_pre_deny "reason text"
emit_pre_deny() {
  local reason="$1"
  jq -cn --arg r "$reason" '
    { hookSpecificOutput: {
        hookEventName: "PreToolUse",
        permissionDecision: "deny",
        permissionDecisionReason: $r
    } }
  '
}

# Emit a UserPromptSubmit additionalContext (injects info Claude reads before
# generating). Same shape as PreToolUse but with hookEventName UserPromptSubmit.
emit_prompt_context() {
  local msg="$1"
  jq -cn --arg msg "$msg" '
    { hookSpecificOutput: { hookEventName: "UserPromptSubmit", additionalContext: $msg } }
  '
}

# Emit a top-level systemMessage (shown to the *user*, not Claude). Optionally
# combine with additionalContext via a second jq merge if both are needed.
emit_system_message() {
  local msg="$1"
  jq -cn --arg msg "$msg" '{ systemMessage: $msg }'
}

# Atomically increment the integer in $1 (dir-lock, temp-file swap).
# Sets $_INCR_RESULT to the new value; also usable for its file side effect alone.
_incr_counter() {
  local file="$1"
  local lock="${file}.lock"
  local i=0
  until mkdir "$lock" 2>/dev/null || [ "$i" -ge 50 ]; do
    sleep 0.01; i=$((i+1))
  done
  local n
  n="$(cat "$file" 2>/dev/null || echo 0)"
  n=$((n + 1))
  printf '%s' "$n" > "${file}.tmp.$$" && mv "${file}.tmp.$$" "$file"
  rmdir "$lock" 2>/dev/null
  _INCR_RESULT="$n"
}

# repo_slug: print the <owner>/<repo> slug for the current git repo, derived
# from the origin remote. Prints nothing outside a repo or with no origin.
# Canonical definition: the memory store keys project facts on this exact
# string, so every consumer must derive it identically or facts silently fail
# to resolve. shell/memory-context.sh carries its own copy on purpose, because
# it is a standalone CLI that must not depend on the hook library; keep the two
# in step.
repo_slug() {
  git --no-optional-locks remote get-url origin 2>/dev/null \
    | sed -E 's#\.git/?$##; s#^[a-zA-Z]+://##; s#^[^@/]+@##; s#^[^/:]+[:/]##'
}
