#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# SessionStart hook:
#   1. Prepare per-session runtime dir + zero counters.
#   2. On fresh startup (source=startup), detect orphaned previous sessions
#      that may have crashed and surface a recovery hint to the user.
# shellcheck source=hooks/lib/common.sh
. "$(dirname "$0")/lib/common.sh"
# shellcheck source=hooks/lib/config-hash.sh
. "$(dirname "$0")/lib/config-hash.sh"

dir="$(session_dir)"
if [[ -n "$dir" ]]; then
  : > "$dir/search-count" 2>/dev/null
  : > "$dir/tool-count" 2>/dev/null
  : > "$dir/edit-count" 2>/dev/null
  : > "$dir/edits.jsonl" 2>/dev/null
  : > "$dir/seen-reads" 2>/dev/null
  date +%s > "$dir/start-ts" 2>/dev/null
fi

# Clear the statusline PR/CI cache for the current repo+branch so the first
# render of each session fetches fresh data rather than reusing stale cache.
_sl_cache="${STATUSLINE_CACHE_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/statusline}"
_sl_branch=$(git --no-optional-locks rev-parse --abbrev-ref HEAD 2>/dev/null)
if [[ -n "$_sl_branch" && "$_sl_branch" != "HEAD" ]]; then
  _sl_slug=$(printf '%s' "${PWD}::${_sl_branch}" | sed 's/[^A-Za-z0-9_.-]/_/g')
  rm -f "${_sl_cache}/pr-${_sl_slug}.json" "${_sl_cache}/ci-${_sl_slug}.json"
fi

source="$(hi_field '.source')"

# ── Config hash: warn on resume if settings have drifted ──
# Hash = sha256 of settings.json + every hook script. Stored in the session
# dir on first startup; on resume we recompute and compare. Divergence means
# the resumed session is running on the OLD config (plugins, hooks, model
# default, output style frozen at original session start).
hash_file="$dir/config-hash"
current_hash="$(config_hash)"

# Accumulate any SessionStart output; emit a single payload at the end.
system_message=""
extra_context=""

if [[ "$source" == "resume" && -n "$current_hash" ]]; then
  prev_hash="$(cat "$hash_file" 2>/dev/null)"
  if [[ -n "$prev_hash" && "$prev_hash" != "$current_hash" ]]; then
    system_message="⚠ Claude config (settings.json + hooks) has drifted since this session was created. Plugins, output style, model default, and new hooks will NOT take effect on this resumed session: they're frozen at the original startup snapshot. To apply current config: exit and run \`cc fresh\` (or \`claude\` without --resume)."
    extra_context="The user resumed this session, but the config hash has changed since session creation. The harness has the OLD settings loaded. If the user asks about why a recent settings change isn't showing up, point them to 'cc fresh' or starting a new \`claude\` invocation."
  fi
fi

# Always refresh the stored hash on startup (the new baseline going forward).
[[ -n "$current_hash" && "$source" == "startup" ]] && \
  printf '%s' "$current_hash" > "$hash_file" 2>/dev/null

# ── Project memory ──
# Project facts live in the central store at ~/.claude/memory/<owner>/<repo>/,
# where <owner>/<repo> is derived from the repo's origin remote. The whole
# store is git-ignored at the .claude level. No-op outside a git repo, with
# no origin remote, or when the repo has no project store yet.
#
# Preferred source: shell/memory-context.sh renders a repo-scoped slice of
# the graph-first store, facts in scope, their typed edges, and an anchor
# index mapping code paths to the facts that describe them; fact bodies are
# read on demand. Fall back to the legacy MEMORY.md index when that script
# is missing (a partial install where shell/ was not copied) or produces no
# output (no graph yet). This runs on every session start, so every path
# here must degrade to no memory block rather than an error.
_repo_root="$(git --no-optional-locks rev-parse --show-toplevel 2>/dev/null)"
_mem_slug="$(repo_slug)"
_mem_body=""
_mem_preamble=""
if [[ -n "$_repo_root" && -n "$_mem_slug" ]]; then
  _mem_script="$(dirname "$0")/../shell/memory-context.sh"
  if [[ -f "$_mem_script" ]]; then
    _mem_body="$(bash "$_mem_script" --repo "$_mem_slug" 2>/dev/null)"
  fi
  if [[ -n "$_mem_body" ]]; then
    _mem_preamble="Project memory for this repo ($_mem_slug), stored in the central memory store at ~/.claude/memory/$_mem_slug/. A scoped slice: facts in scope, their typed edges, and an anchor index mapping code paths to the facts that describe them. Fact bodies are read on demand."
  elif [[ -f "$HOME/.claude/memory/$_mem_slug/MEMORY.md" ]]; then
    _mem_body="$(head -c 16000 "$HOME/.claude/memory/$_mem_slug/MEMORY.md" 2>/dev/null)"
    if [[ -n "$_mem_body" ]]; then
      _mem_preamble="Project memory for this repo ($_mem_slug), stored in the central memory store at ~/.claude/memory/$_mem_slug/. These facts apply only in this repo; read the referenced fact files on demand. Index:"
    fi
  fi
fi
if [[ -n "$_mem_body" ]]; then
  _mem_ctx="$_mem_preamble
$_mem_body"
  if [[ -n "$extra_context" ]]; then
    extra_context="$extra_context

$_mem_ctx"
  else
    extra_context="$_mem_ctx"
  fi
fi

# ── Auto-learn nudge ──
# If the previous session in this repo did substantive work, session-clean-exit
# left a flag in ~/.claude/runtime/to-learn/. Surface a one-time nudge to run
# /learn-project, then consume the flag. Stale flags are pruned so the dir
# doesn't grow. Disable with AUTO_LEARN_NUDGE=0.
if [[ "${AUTO_LEARN_NUDGE:-1}" != "0" && -n "$_repo_root" ]]; then
  _qdir="$RUNTIME_ROOT/to-learn"
  [[ -d "$_qdir" ]] && find "$_qdir" -name '*.json' -type f -mtime "+${AUTO_LEARN_MAX_AGE_DAYS:-14}" -delete 2>/dev/null
  _learn_slug="$(printf '%s' "$_repo_root" | sed 's/[^A-Za-z0-9_.-]/_/g')"
  _learn_flag="$_qdir/$_learn_slug.json"
  if [[ -f "$_learn_flag" ]]; then
    _learn_edits="$(jq -r '.edits // 0' "$_learn_flag" 2>/dev/null)"
    _learn_nudge="A previous session in this repo made ${_learn_edits:-some} edits, so project memory may be stale. Consider running /learn-project to refresh it, or /learn-project --stage to queue candidate facts for review."
    if [[ -n "$extra_context" ]]; then
      extra_context="$extra_context

$_learn_nudge"
    else
      extra_context="$_learn_nudge"
    fi
    rm -f "$_learn_flag" 2>/dev/null
  fi
fi

# ── Skills & commands primer ──
# Surface the toolkit so the model reaches for a fitting skill/command instead
# of ad-hoc steps. The catalog is built from the global skills/ and commands/
# frontmatter, so new entries appear automatically. Disable with SKILLS_PRIMER=0.
if [[ "${SKILLS_PRIMER:-1}" != "0" ]]; then
  # _fm_field <file> <field>: value of a field in the first frontmatter block.
  _fm_field() {
    awk -v f="$2" '
      NR==1 && $0=="---"{inb=1; next}
      inb && $0=="---"{exit}
      inb && index($0, f":")==1 { sub("^"f":[[:space:]]*",""); gsub(/^"|"$/,""); print; exit }
    ' "$1"
  }
  # _one_line <text>: collapse whitespace to one line, cap the length.
  _one_line() {
    local s; s="$(printf '%s' "$1" | tr '\n\t' '  ')"
    if (( ${#s} > 150 )); then printf '%s...' "${s:0:147}"; else printf '%s' "$s"; fi
  }
  _skill_lines=""
  for _sf in "$HOME"/.claude/skills/*/SKILL.md; do
    [[ -f "$_sf" ]] || continue
    _nm="$(_fm_field "$_sf" name)"; [[ -z "$_nm" ]] && _nm="$(basename "$(dirname "$_sf")")"
    _skill_lines+="- ${_nm}: $(_one_line "$(_fm_field "$_sf" description)")"$'\n'
  done
  _cmd_lines=""
  for _cf in "$HOME"/.claude/commands/*.md; do
    [[ -f "$_cf" ]] || continue
    _cmd_lines+="- /$(basename "$_cf" .md): $(_one_line "$(_fm_field "$_cf" description)")"$'\n'
  done
  if [[ -n "$_skill_lines" || -n "$_cmd_lines" ]]; then
    _toolkit="Your toolkit. Before substantive work, check whether one of these fits and use it instead of ad-hoc steps: plan a feature with /scope, execute a ready plan with /implement, record a decision with /adr, commit and push with /commit-and-push, open a PR with /create-pull-request, review a PR with /quick-review or /deep-review, debug a failure with the systematic-debugging skill. Invoke skills via the Skill tool, commands as slash commands. Full catalog (name: what it is for):"
    [[ -n "$_skill_lines" ]] && _toolkit+=$'\n\nSkills:\n'"$_skill_lines"
    [[ -n "$_cmd_lines" ]] && _toolkit+=$'\nCommands:\n'"$_cmd_lines"
    if [[ -n "$extra_context" ]]; then
      extra_context="$extra_context"$'\n\n'"$_toolkit"
    else
      extra_context="$_toolkit"
    fi
  fi
fi

# ── Async & deferred-tool discipline ──
# Two recurring failure classes cost real tokens: (1) calling a deferred tool
# (surfaced by name only, schema not loaded) with guessed params, which fails
# client-side validation; (2) backgrounding a command whose result the next
# step needs, then racing or polling it. One compact reminder heads both off.
# Disable with ASYNC_DISCIPLINE=0.
if [[ "${ASYNC_DISCIPLINE:-1}" != "0" ]]; then
  _async_ctx="Async and deferred-tool discipline. (1) Deferred tools are surfaced by name only (e.g. Monitor, TaskCreate, TaskStop, TaskUpdate, ScheduleWakeup): their schemas are NOT loaded, so calling them with guessed parameters fails validation. Before calling any tool that is not already in your active tool list, load it first with ToolSearch (query \"select:NAME\"), then call it; never guess its parameters. (2) Don't run a command in the background when the next step needs its result (installs, builds, typechecks): run it in the foreground with an extended timeout (up to 600000ms). A backgrounded job re-invokes you only when it exits, and shell state (including \`wait\`) does not persist across Bash calls, so there is nothing to poll."
  if [[ -n "$extra_context" ]]; then
    extra_context="$extra_context"$'\n\n'"$_async_ctx"
  else
    extra_context="$_async_ctx"
  fi
fi

# Emit a single SessionStart payload if there is anything to say.
if [[ -n "$system_message" || -n "$extra_context" ]]; then
  jq -cn --arg um "$system_message" --arg cm "$extra_context" '
    ( (if $um != "" then { systemMessage: $um } else {} end)
      + (if $cm != "" then { hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: $cm } } else {} end) )
  '
fi

# NOTE: crash-recovery warning removed. `ccd` restores the most recent session
# per directory, so the "did not end cleanly" warning was redundant, and it
# could not distinguish a live session (no clean-exit yet, but running) from a
# real crash, so it kept flagging healthy sessions. Recover any session with
# `ccd` in its directory, or `claude --resume <id>` for a specific one.
exit 0
