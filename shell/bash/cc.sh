# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# shellcheck shell=bash
# (sourced, so it has no shebang; the directive tells shellcheck the dialect)
# Claude Code launcher: bash-compatible cc/ccd.  Source from ~/.bashrc.
#
# Provides cc() and ccd() mirroring the zsh launcher's core semantics:
#   - Default: resume this directory's most recent session (matched by name),
#     or start fresh when no session exists.
#   - Adds --system-prompt-file only when the file exists (opt-in).
#   - ccd adds --dangerously-skip-permissions.
#   - Prunes old transcripts (CCD_KEEP, default 5) after every launch.
#
# Supported subcommands:
#   fresh            Start a brand-new session (no resume).
#   list / ls        Show recent sessions for $PWD with timestamps.
#   prune            Run the transcript prune immediately.
#   worktree / new   Create or enter a git worktree, then launch a session.
#
# Subcommands not ported (use zsh): clean, raw, config-drift fork.
#
# Install: source ~/.claude/shell/bash/cc.sh from ~/.bashrc
#   source ~/.claude/shell/bash/cc.sh
#
# bash -n shell/bash/cc.sh must pass.  No zsh-isms: no typeset -A, no (N) globs,
# no setopt, no zparseopts, no ${(f)...}, no zmodload.

# ─── worktree engine ─────────────────────────────────────────────────────────
# shellcheck source=shell/shared/worktree.sh
source "$HOME/.claude/shell/shared/worktree.sh"

# ─── private helpers ─────────────────────────────────────────────────────────

_cc_bash_bust_cache() {
    local d="$HOME/.claude"
    rm -f "$d"/shell-snapshots/snapshot-*.sh 2>/dev/null
    find "$d/runtime" -name "config-hash" -delete 2>/dev/null
    rm -f "$d/plugins/plugin-catalog-cache.json" 2>/dev/null
    rm -f "$d"/backups/* 2>/dev/null
}

# Replicate Claude's slug: replace every non-alphanumeric char with '-'.
_cc_bash_project_dir() {
    local slug="${PWD//[^a-zA-Z0-9]/-}"
    printf '%s/.claude/projects/%s' "$HOME" "$slug"
}

# Find the most-recent .jsonl whose body contains "customTitle":"<name>".
# Prints the session UUID (no extension), or nothing if not found.
_cc_bash_find_session() {
    local project_dir="$1" name="$2"
    [[ -d "$project_dir" ]] || return 0
    local -a jsonl_files=()
    local f
    while IFS= read -r f; do
        jsonl_files+=("$f")
    done < <(find "$project_dir" -maxdepth 1 -name '*.jsonl' -type f 2>/dev/null)
    [[ ${#jsonl_files[@]} -eq 0 ]] && return 0
    local match_file
    match_file=$(grep -rl "\"customTitle\":\"$name\"" "${jsonl_files[@]}" 2>/dev/null \
                   | xargs ls -t 2>/dev/null | head -1)
    if [[ -n "$match_file" ]]; then
        local base="${match_file##*/}"
        printf '%s' "${base%.jsonl}"
    fi
}

# Keep only the newest $CCD_KEEP transcripts for $PWD's project directory.
# CCD_KEEP=0 disables.  Floor of 2 protects fork/clean parents.
_cc_bash_prune() {
    local keep=${CCD_KEEP:-5}
    [[ "$keep" -le 0 ]] && return 0
    (( keep < 2 )) && keep=2
    local pd
    pd="$HOME/.claude/projects/${PWD//[^a-zA-Z0-9]/-}"
    [[ -d "$pd" ]] || return 0
    local -a ranked=()
    local f m
    while IFS= read -r f; do
        ranked+=("$f")
    done < <(
        while IFS= read -r f; do
            m=$(stat -f %m "$f" 2>/dev/null || stat -c %Y "$f" 2>/dev/null || printf '0')
            printf '%s %s\n' "$m" "$f"
        done < <(find "$pd" -maxdepth 1 -name '*.jsonl' -type f 2>/dev/null) \
        | sort -rn | cut -d' ' -f2-
    )
    local total=${#ranked[@]}
    (( total > keep )) || return 0
    local i=0 sid
    for f in "${ranked[@]}"; do
        i=$(( i + 1 ))
        (( i <= keep )) && continue
        sid="${f##*/}"; sid="${sid%.jsonl}"
        rm -f "$f" 2>/dev/null
        # ${var:?} so an empty pd or sid aborts instead of expanding to /.
        rm -rf "${pd:?}/${sid:?}" "$HOME/.claude/runtime/${sid:?}" 2>/dev/null
    done
}

_cc_bash_list_sessions() {
    local pd="$1"
    if [[ ! -d "$pd" ]]; then
        printf 'no sessions for %s\n' "${PWD##*/}"
        return 0
    fi
    printf 'Recent sessions for %s:\n' "${PWD##*/}"
    {
        find "$pd" -mindepth 1 -maxdepth 1 -name '*.jsonl' -type f 2>/dev/null |
        while IFS= read -r f; do
            local sid="${f##*/}"; sid="${sid%.jsonl}"
            local ts title
            ts=$(stat -f %m "$f" 2>/dev/null || stat -c %Y "$f" 2>/dev/null || printf '0')
            title=$(grep -m1 -oE '"customTitle":"[^"]*"' "$f" 2>/dev/null | head -1 \
                      | sed 's/"customTitle":"//; s/"$//')
            printf '%s\t%s\t%s\n' "$ts" "$sid" "${title:-(no title)}"
        done | sort -rnu -k1,1 | head -10
    } | while IFS=$'\t' read -r ts sid title; do
        local when
        when=$(date -r "$ts" '+%Y-%m-%d %H:%M' 2>/dev/null \
               || date -d "@$ts" '+%Y-%m-%d %H:%M' 2>/dev/null)
        printf '  %s  %s  %s\n' "$when" "${sid:0:8}…" "$title"
    done
}

# ─── core dispatcher ─────────────────────────────────────────────────────────
# First arg: "plain" or "danger" (controls --dangerously-skip-permissions).
# Remaining args: subcommand + forwarded flags.

_cc_bash_dispatch() {
    local _mode="$1"; shift
    _cc_bash_bust_cache
    clear

    local name="${PWD##*/}"
    local project_dir
    project_dir="$(_cc_bash_project_dir)"

    # Build leading flags.
    local -a flags=()
    [[ "$_mode" == "danger" ]] && flags+=(--dangerously-skip-permissions)
    [[ -f "$HOME/.claude/prompts/SYSTEM_PROMPT.md" ]] \
        && flags+=(--system-prompt-file "$HOME/.claude/prompts/SYSTEM_PROMPT.md")

    case "${1:-}" in
        fresh|--fresh)
            shift
            printf '→ cc: fresh session (no resume; settings.json applied)\n'
            command claude "${flags[@]}" -n "$name" "$@"
            return $?
            ;;
        list|ls|--list)
            _cc_bash_list_sessions "$project_dir"
            return $?
            ;;
        prune|--prune)
            _cc_bash_prune
            return $?
            ;;
        worktree|--worktree|new|--new)
            shift
            _cc_worktree --ai-resolve "$@" || return $?
            [[ -n "${_WT_NO_LAUNCH:-}" ]] && return 0
            name="${PWD##*/}"
            command claude "${flags[@]}" -n "$name"
            return $?
            ;;
    esac

    # Default: resume the most-recent matching session, or start fresh.
    local session_id
    session_id="$(_cc_bash_find_session "$project_dir" "$name")"

    if [[ -n "$session_id" ]]; then
        local err_tmp
        err_tmp=$(mktemp)
        command claude "${flags[@]}" -n "$name" --resume "$session_id" "$@" 2>"$err_tmp"
        local rc=$?
        if grep -q "No conversation found" "$err_tmp" 2>/dev/null; then
            printf '→ cc: session %s… not found; starting fresh\n' "${session_id:0:8}"
            command claude "${flags[@]}" -n "$name" "$@"
            rc=$?
        else
            cat "$err_tmp" >&2
        fi
        rm -f "$err_tmp" 2>/dev/null
        return $rc
    else
        command claude "${flags[@]}" -n "$name" "$@"
        return $?
    fi
}

# ─── public wrappers ─────────────────────────────────────────────────────────

cc()  { _cc_bash_dispatch plain  "$@"; local rc=$?; _cc_bash_prune; return $rc; }
ccd() { _cc_bash_dispatch danger "$@"; local rc=$?; _cc_bash_prune; return $rc; }
