# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# shellcheck shell=bash
# (sourced, no shebang; the directive tells shellcheck the dialect)
#
# dispatch.sh: _claude dispatcher.
#
# Drop-in replacement for the original _claude that adds subcommand dispatch
# (clean / fresh / raw / list / worktree, alias new) while preserving the
# resume-by-customTitle default. All non-subcommand args (including
# --dangerously-skip-permissions from ccd) are passed through to
# `command claude` unchanged.
#
# Depends on: bust-cache, config-drift, sessions, clean-resume, worktree modules.
# Sourceable in bash and zsh. No zsh-only builtins or associative arrays.

# Returns 0 when the option $1 consumes the next argument as its value.
_cc_opt_takes_value() {
    case "$1" in
        --system-prompt-file|--system-prompt|\
        --append-system-prompt|--append-system-prompt-file|\
        --settings|--setting-sources|--model|--permission-mode|\
        --name|-n) return 0 ;;
        *) return 1 ;;
    esac
}

_claude() {
    _cc_bust_cache
    clear

    local name="${PWD##*/}"
    # Claude derives the project-dir slug by replacing EVERY non-alphanumeric
    # char with "-" (not just "/"). E.g. /Users/me/.claude becomes
    # "-Users-me--claude": note the DOUBLE dash, from the "/" and the "." both
    # being replaced.
    local project_dir="$HOME/.claude/projects/${PWD//[^a-zA-Z0-9]/-}"

    # Separate leading flags from positional args. The subcommand must be the
    # first non-flag token so `ccd clean` (which expands to
    # `_claude --dangerously-skip-permissions clean`) works.
    #
    # Value-taking options must consume their value too; otherwise the value is
    # mistaken for the subcommand and swallowed as the option's argument.
    local -a flags=()
    while [ "$#" -gt 0 ]; do
        case "$1" in
            -*=*) flags+=("$1"); shift ;;
            -*)
                flags+=("$1")
                if _cc_opt_takes_value "$1" && [ "$#" -ge 2 ]; then
                    flags+=("$2"); shift 2
                else
                    shift
                fi
                ;;
            *) break ;;
        esac
    done

    case "${1:-}" in
        clean|--clean)
            shift
            _cc_clean_resume "$project_dir" "$name" "${flags[@]}" "$@"
            return $?
            ;;
        fresh|--fresh)
            shift
            printf '%s\n' "-> cc: fresh session (no resume; settings.json applied)"
            _cc_config_stamp
            command claude "${flags[@]}" -n "$name" "$@"
            return $?
            ;;
        raw|--raw)
            shift
            local raw_sid="${1:-}"
            [ -n "$raw_sid" ] && shift
            [ -z "$raw_sid" ] && raw_sid="$(_cc_find_session_by_title "$project_dir" "$name")"
            if [ -z "$raw_sid" ]; then
                printf '%s\n' "-> cc raw: no matching session; starting fresh"
                command claude "${flags[@]}" -n "$name" "$@"
            else
                printf '%s\n' "-> cc raw: resuming $raw_sid (no fork, overrides preserved)"
                command claude "${flags[@]}" --resume "$raw_sid" -n "$name" "$@"
            fi
            return $?
            ;;
        list|ls|--list)
            _cc_list_sessions "$project_dir"
            return $?
            ;;
        prune|--prune)
            _cc_prune
            return $?
            ;;
        worktree|--worktree|new|--new)
            shift
            _cc_worktree --ai-resolve "$@" || return $?
            _claude "${flags[@]}"
            return $?
            ;;
    esac

    # Default: replicate the original _claude behavior.
    local session_id
    session_id="$(_cc_find_session_by_title "$project_dir" "$name")"

    if [ -n "$session_id" ]; then
        local -a fork=()
        if [ -n "$(_cc_config_drifted)" ]; then
            fork=(--fork-session)
            printf '%s\n' "-> cc: config changed; forking to reload settings/plugins/hooks"
        fi
        local _cc_err_tmp
        _cc_err_tmp=$(mktemp)
        command claude "${flags[@]}" -n "$name" --resume "$session_id" "${fork[@]}" "$@" \
            2>"$_cc_err_tmp"
        local _cc_rc=$?
        if grep -q "No conversation found" "$_cc_err_tmp" 2>/dev/null; then
            printf '%s\n' "-> cc: session ${session_id:0:8}... not found; starting fresh"
            command claude "${flags[@]}" -n "$name" "$@"
            _cc_rc=$?
        else
            cat "$_cc_err_tmp" >&2
        fi
        rm -f "$_cc_err_tmp" 2>/dev/null
        return $_cc_rc
    else
        _cc_config_stamp
        command claude "${flags[@]}" -n "$name" "$@"
    fi
}

# ── Public wrappers ──
# cc/ccd both dispatch through _claude. Each carries the custom system prompt
# when it exists; ccd adds --dangerously-skip-permissions. After every run,
# prune old transcripts. Defined here (not per shell) so both entry points get
# them by sourcing, and there is one implementation to keep honest.
cc() {
    local -a _cc_sys=()
    [ -f "$HOME/.claude/prompts/SYSTEM_PROMPT.md" ] \
        && _cc_sys=(--system-prompt-file "$HOME/.claude/prompts/SYSTEM_PROMPT.md")
    _claude "${_cc_sys[@]}" "$@"; local rc=$?; _cc_prune; return $rc
}
ccd() {
    local -a _cc_sys=()
    [ -f "$HOME/.claude/prompts/SYSTEM_PROMPT.md" ] \
        && _cc_sys=(--system-prompt-file "$HOME/.claude/prompts/SYSTEM_PROMPT.md")
    _claude --dangerously-skip-permissions "${_cc_sys[@]}" "$@"; local rc=$?; _cc_prune; return $rc
}
