#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# PreToolUse(Bash) guard: block `rm` targets outside ~/Workspace/** and ~/.claude/**.
# Best-effort protection against an accidental rm, NOT a security boundary: it only
# guards `rm` (not find -delete, unlink, or `>` truncation), and `rm` reached through
# `$(...)` or a backtick is blocked conservatively rather than evaluated. A `cd` in
# the command makes relative targets unresolvable, so those are blocked
# conservatively too. A quoted path containing a space still splits into two
# tokens and is evaluated as two separate paths (pre-existing gap, fails closed).
set -u  # not -e: a parse failure must not exit non-zero and let the rm through

CMD=$(jq -r '.tool_input.command // ""' -)
[[ -z "$CMD" ]] && exit 0

WORKSPACE="$HOME/Workspace"
CLAUDE_DIR="$HOME/.claude"

# Lexically resolve . and .. without touching the filesystem (the rm target may
# not exist yet). Conservative: a path like ~/Workspace/../secrets collapses to
# a path outside the allowlist and is blocked, closing the `..` traversal bypass.
canon() {
  local p="$1" seg res=""
  local -a out _segs
  [[ "$p" != /* ]] && p="$(pwd)/$p"
  local IFS=/
  read -ra _segs <<< "$p"
  for seg in "${_segs[@]}"; do
    case "$seg" in
      ''|.) ;;
      ..)   [[ ${#out[@]} -gt 0 ]] && unset "out[$(( ${#out[@]} - 1 ))]" ;;
      *)    out+=("$seg") ;;
    esac
  done
  # ${out[@]+"${out[@]}"}: `out` is empty when the path collapses to root (e.g.
  # "/"), and bash 3.2 (macOS system bash) errors "unbound variable" on an empty
  # array under set -u. This form is a no-op when empty, elements when not.
  for seg in ${out[@]+"${out[@]}"}; do res+="/$seg"; done
  printf '%s' "${res:-/}"
}

is_allowed() {
  local path="${1/#\~/$HOME}"
  path="$(canon "$path")"
  [[ "$path" == "$WORKSPACE" || "$path" == "$WORKSPACE/"* ]] && return 0
  [[ "$path" == "$CLAUDE_DIR" || "$path" == "$CLAUDE_DIR/"* ]] && return 0
  return 1
}

in_rm=false
saw_cd=false
saw_rm=false
outside=()

# `read` only tokenizes one line; normalise newlines and tabs to spaces first
# so an rm on any line of a multi-line command is still seen. A newline maps
# to a standalone `;` so it resets in_rm the same as a real command separator.
CMD_NORM="${CMD//$'\n'/ ; }"
CMD_NORM="${CMD_NORM//$'\t'/ }"
IFS=' ' read -ra tokens <<< "$CMD_NORM"

# ${tokens[@]+...}: a whitespace-only command splits to an empty array, which
# would error under set -u on bash 3.2 and fail the guard open. Safe-expand it.
for token in ${tokens[@]+"${tokens[@]}"}; do
  [[ -z "$token" ]] && continue
  # A `cd` anywhere means $(pwd) no longer reflects where a relative rm resolves.
  if [[ "$token" == "cd" || "$token" == */cd ]]; then
    saw_cd=true; continue
  fi
  if [[ "$token" == "rm" || "$token" == */rm ]]; then
    in_rm=true; saw_rm=true; continue
  fi
  if [[ "$token" == ";" || "$token" == "&&" || "$token" == "||" || "$token" == "|" || "$token" == "&" ]]; then
    in_rm=false; continue
  fi
  if [[ "$in_rm" == true ]]; then
    [[ "$token" == -* ]] && continue
    if [[ "$saw_cd" == true && "$token" != /* && "$token" != '~'* ]]; then
      outside+=("$token")            # relative target after a cd: unresolvable, block
    elif ! is_allowed "$token"; then
      outside+=("$token")
    fi
  fi
done

# rm reached through $(...) or a backtick: the tokenizer cannot evaluate what
# the substitution expands to, so block conservatively, same posture as saw_cd.
if [[ "$saw_rm" == true && ( "$CMD_NORM" == *'$('* || "$CMD_NORM" == *'`'* ) ]]; then
  outside+=("<command substitution>")
fi

if (( ${#outside[@]} > 0 )); then
  joined=$(IFS=', '; echo "${outside[*]}")
  jq -n --arg r "rm blocked: $joined is outside ~/Workspace/** and ~/.claude/**" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
fi
