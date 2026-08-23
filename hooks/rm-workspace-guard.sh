#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# PreToolUse(Bash) guard: block `rm` targets outside PLAYBOOK_SAFE_ROOTS/** and
# ~/.claude/**. PLAYBOOK_SAFE_ROOTS is a colon-separated allowlist of root
# directories, like $PATH. Unset or empty defaults to the git repo root of
# this guard's own cwd, falling back to the cwd itself outside a repo: a
# fresh checkout needs no configuration, work inside your own project is
# allowed, and ~/.ssh and ~/.aws stay blocked exactly as before ($HOME itself
# is deliberately NOT the default: that would unblock them). $HOME/.claude is
# always allowed, regardless of PLAYBOOK_SAFE_ROOTS, and so is anything INSIDE
# /tmp or /private/tmp; the temp root itself still blocks.
# Best-effort protection against an accidental rm, NOT a security boundary: it only
# guards `rm` (not find -delete, unlink, or `>` truncation), and `rm` reached through
# `$(...)` or a backtick is blocked conservatively rather than evaluated. A `cd` in
# the command makes relative targets unresolvable, so those are blocked
# conservatively too. A quoted path containing a space still splits into two
# tokens and is evaluated as two separate paths (pre-existing gap, fails closed).
# A target containing `$` or a backtick is unresolvable and blocks: it expands at
# runtime to a path this guard never sees. That rule fixed a FAIL-OPEN, so be
# careful about reverting it.
# A bare `rm` inside a quoted region is treated as prose unless a separator puts
# it in command position, so a commit message or PR title that mentions `rm` no
# longer blocks, while `sh -c "cd /x && rm -rf /etc"` still does.
#
# ACCEPTED MISS, pinned so it is not re-reported as a vulnerability later: a
# command name that is obfuscated or built at runtime is not resolved, so it is
# not recognised as a deletion. Escaped and quote-split spellings, and names
# produced by command or parameter substitution, all fall in this class. Closing
# it needs word expansion, which cannot be done statically for the substitution
# cases at all. This is deliberate and in scope for a guard that exists to catch
# an ACCIDENT: no agent writes an obfuscated command name by accident, and the
# ordinary forms it would write (including `sudo`, `xargs`, `/bin/rm`, env
# prefixes and multi-line commands) are all still caught.
set -u  # not -e: a parse failure must not exit non-zero and let the rm through

CMD=$(jq -r '.tool_input.command // ""' -)
[[ -z "$CMD" ]] && exit 0

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

# PLAYBOOK_SAFE_ROOTS: colon-separated allowlist of root directories, like
# $PATH. Unset or empty defaults to the git repo root of this guard's cwd,
# falling back to the cwd itself outside a repo. The git call runs at most
# once per invocation, is guarded with 2>/dev/null, and never fails the
# guard open: no repo just falls through to $(pwd).
PLAYBOOK_SAFE_ROOTS="${PLAYBOOK_SAFE_ROOTS:-}"
if [[ -z "$PLAYBOOK_SAFE_ROOTS" ]]; then
  repo_root="$(git rev-parse --show-toplevel 2>/dev/null)"
  PLAYBOOK_SAFE_ROOTS="${repo_root:-$(pwd)}"
fi

SAFE_ROOTS=()
IFS=':' read -ra raw_roots <<< "$PLAYBOOK_SAFE_ROOTS"
for root in ${raw_roots[@]+"${raw_roots[@]}"}; do
  [[ -z "$root" ]] && continue
  SAFE_ROOTS+=("$(canon "$root")")
done

is_allowed() {
  local path="${1/#\~/$HOME}"
  path="$(canon "$path")"
  [[ "$path" == "$CLAUDE_DIR" || "$path" == "$CLAUDE_DIR/"* ]] && return 0
  # Scratch space, allowed whatever SAFE_ROOTS says. Both spellings are listed
  # because canon() is lexical and macOS /tmp is a symlink to /private/tmp, so
  # matching one would allow /tmp/x while blocking the identical /private/tmp/x.
  # The `/`* pattern requires something INSIDE the root, so `rm -rf /tmp` itself
  # still blocks: it would take out sockets and runtime state that live
  # processes depend on. canon() has already collapsed `..`, so /tmp/../etc is
  # judged as /etc and blocks.
  local tmp_root
  for tmp_root in /tmp /private/tmp; do
    [[ "$path" == "$tmp_root/"* ]] && return 0
  done
  local root
  for root in ${SAFE_ROOTS[@]+"${SAFE_ROOTS[@]}"}; do
    [[ "$path" == "$root" || "$path" == "$root/"* ]] && return 0
  done
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
# Tokenize on spaces exactly as before, INCLUDING inside quotes: splitting
# inside quotes is what still catches `sh -c "cd /x && rm -rf /etc"`. Quote
# characters stay in the token text so target judging is unchanged. The only new
# output is a parallel array recording whether each token began inside a quoted
# region, which is what lets a `rm` in a commit message or PR title be told from
# a real command. An unbalanced quote leaves the rest marked quoted, which only
# ever demands a separator before believing a deletion, so it fails closed.
tokens=()
tok_quoted=()
_cur=""
_cur_quoted=false
_in_single=false
_in_double=false
_i=0
_len=${#CMD_NORM}
while (( _i < _len )); do
  _ch="${CMD_NORM:_i:1}"
  if [[ "$_ch" == " " ]]; then
    if [[ -n "$_cur" ]]; then
      tokens+=("$_cur"); tok_quoted+=("$_cur_quoted"); _cur=""
    fi
    _i=$(( _i + 1 )); continue
  fi
  if [[ -z "$_cur" ]]; then
    if [[ "$_in_single" == true || "$_in_double" == true ]]; then
      _cur_quoted=true
    else
      _cur_quoted=false
    fi
  fi
  _cur+="$_ch"
  if [[ "$_ch" == "'" && "$_in_double" == false ]]; then
    if [[ "$_in_single" == true ]]; then _in_single=false; else _in_single=true; fi
  elif [[ "$_ch" == '"' && "$_in_single" == false ]]; then
    if [[ "$_in_double" == true ]]; then _in_double=false; else _in_double=true; fi
  fi
  _i=$(( _i + 1 ))
done
if [[ -n "$_cur" ]]; then tokens+=("$_cur"); tok_quoted+=("$_cur_quoted"); fi

# The very start of a command is a command position, same as after a `;`.
prev_was_separator=true
_n=${#tokens[@]}
_j=0
while (( _j < _n )); do
  token="${tokens[$_j]}"
  quoted="${tok_quoted[$_j]}"
  _j=$(( _j + 1 ))
  [[ -z "$token" ]] && continue
  # A word inside quotes is prose unless a separator put it in command position.
  # Outside quotes nothing changes, so `sudo rm` and `xargs rm` stay caught.
  if [[ "$quoted" == false || "$prev_was_separator" == true ]]; then
    is_command=true
  else
    is_command=false
  fi
  if [[ "$is_command" == true && ( "$token" == "cd" || "$token" == */cd ) ]]; then
    saw_cd=true; prev_was_separator=false; continue
  fi
  if [[ "$is_command" == true && ( "$token" == "rm" || "$token" == */rm ) ]]; then
    in_rm=true; saw_rm=true; prev_was_separator=false; continue
  fi
  if [[ "$token" == ";" || "$token" == "&&" || "$token" == "||" || "$token" == "|" || "$token" == "&" ]]; then
    in_rm=false; prev_was_separator=true; continue
  fi
  prev_was_separator=false
  if [[ "$in_rm" == true ]]; then
    [[ "$token" == -* ]] && continue
    # A target carrying `$` or a backtick expands at runtime to a path the guard
    # cannot see, so it is unresolvable the same way a relative target after a
    # `cd` is. This closes a FAIL-OPEN: canon() treated a leading `$` as a
    # relative path and joined it to the cwd, so `rm -rf "$HOME/.cache/x"`
    # resolved to <repo>/$HOME/.cache/x, landed inside a safe root and was
    # ALLOWED, while the shell expanded it to a real path outside the workspace.
    # The cost is that `rm -rf "$REPO/target"` blocks too; that is the correct
    # direction to be wrong in, and the caller can retry with a literal path.
    if [[ "$token" == *'$'* || "$token" == *'`'* ]]; then
      outside+=("$token")            # unexpanded expansion: unresolvable, block
    elif [[ "$saw_cd" == true && "$token" != /* && "$token" != '~'* ]]; then
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
  roots_joined=""
  for root in ${SAFE_ROOTS[@]+"${SAFE_ROOTS[@]}"}; do
    [[ -n "$roots_joined" ]] && roots_joined+=", "
    roots_joined+="$root/**"
  done
  [[ -z "$roots_joined" ]] && roots_joined="(no safe roots configured)"
  jq -n --arg r "rm blocked: $joined is outside $roots_joined and ~/.claude/**" \
    '{hookSpecificOutput:{hookEventName:"PreToolUse",permissionDecision:"deny",permissionDecisionReason:$r}}'
fi
