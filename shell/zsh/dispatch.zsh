# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# cc module: _claude dispatcher
#
# Drop-in replacement for the original _claude that adds subcommand dispatch
# (clean / fresh / raw / list / worktree, alias new) while preserving the
# resume-by-customTitle default. All non-subcommand args (including
# --dangerously-skip-permissions from ccd) are passed through to
# `command claude` unchanged.
#
# Depends on: bust-cache, config-drift, sessions, clean-resume, worktree modules.
_claude() {
    emulate -L zsh 2>/dev/null || true
    _cc_bust_cache
    clear

    local name="${PWD##*/}"
    # Claude derives the project-dir slug by replacing EVERY non-alphanumeric
    # char with "-" (not just "/"). E.g. ~/.claude → "-Users-isantos--claude".
    # Replacing only "/" misses paths containing "." "_" space, etc.
    local project_dir="$HOME/.claude/projects/${PWD//[^a-zA-Z0-9]/-}"

    # Separate leading flags from positional args. The subcommand must be the
    # first non-flag token so `ccd clean` (which expands to
    # `_claude --dangerously-skip-permissions clean`) works.
    #
    # Value-taking options (e.g. `--system-prompt-file PATH`, injected by cc/ccd)
    # must consume their value too. Otherwise the value is mistaken for the
    # subcommand and the `-n <name>` added below is swallowed as the option's
    # argument, turning `--system-prompt-file PATH -n name` into
    # `--system-prompt-file -n` (claude then looks for a prompt file named "-n").
    # The `--opt=value` form is self-contained, so it is matched first.
    local -a flags
    flags=()
    local -A _cc_takes_value
    _cc_takes_value=(
        --system-prompt-file 1 --system-prompt 1
        --append-system-prompt 1 --append-system-prompt-file 1
        --settings 1 --setting-sources 1 --model 1 --permission-mode 1
        --name 1 -n 1
    )
    while (( $# > 0 )); do
        case "$1" in
            -*=*) flags+=("$1"); shift ;;
            -*)
                flags+=("$1")
                if [[ -n "${_cc_takes_value[$1]:-}" ]] && (( $# >= 2 )); then
                    flags+=("$2"); shift 2
                else
                    shift
                fi
                ;;
            *)  break ;;
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
            print -- "→ cc: fresh session (no resume; settings.json applied)"
            _cc_config_stamp
            command claude "${flags[@]}" -n "$name" "$@"
            return $?
            ;;
        raw|--raw)
            shift
            local raw_sid="${1:-}"
            [[ -n "$raw_sid" ]] && shift
            [[ -z "$raw_sid" ]] && raw_sid="$(_cc_find_session_by_title "$project_dir" "$name")"
            if [[ -z "$raw_sid" ]]; then
                print -- "→ cc raw: no matching session; starting fresh"
                command claude "${flags[@]}" -n "$name" "$@"
            else
                print -- "→ cc raw: resuming $raw_sid (no fork, overrides preserved)"
                command claude "${flags[@]}" --resume "$raw_sid" -n "$name" "$@"
            fi
            return $?
            ;;
        list|ls|--list)
            _cc_list_sessions "$project_dir"
            return $?
            ;;
        worktree|--worktree|new|--new)
            shift
            # cc/ccd worktree <branch> (alias: cc new <branch>): create or enter a
            # git worktree (off the project's base branch for a new branch), then
            # launch a session in it. This path always passes --ai-resolve, so
            # Claude resolves any rebase conflicts. worktree() cd's us into the new
            # tree; recursing into _claude with just the leading flags does the
            # normal launch.
            _cc_worktree --ai-resolve "$@" || return $?
            _claude "${flags[@]}"
            return $?
            ;;
    esac

    # ── Default: replicate the original _claude behavior ──
    local session_id
    session_id="$(_cc_find_session_by_title "$project_dir" "$name")"

    if [[ -n "$session_id" ]]; then
        # Plain --resume freezes settings/plugins/hooks at the session's original
        # startup. If config changed since this project last launched, fork to
        # reload it (mints a new transcript; retention prunes the parent). No
        # drift → plain resume, no new file.
        local -a fork
        fork=()
        if [[ -n "$(_cc_config_drifted)" ]]; then
            fork=(--fork-session)
            print -- "→ cc: config changed; forking to reload settings/plugins/hooks"
        fi
        local _cc_err_tmp
        _cc_err_tmp=$(mktemp)
        # always{} guarantees the temp file is removed even if the foreground
        # claude is interrupted (Ctrl+C) before the cleanup line is reached.
        {
            command claude "${flags[@]}" -n "$name" --resume "$session_id" "${fork[@]}" "$@" 2>"$_cc_err_tmp"
            if grep -q "No conversation found" "$_cc_err_tmp" 2>/dev/null; then
                print -- "→ cc: session ${session_id:0:8}… not found; starting fresh"
                command claude "${flags[@]}" -n "$name" "$@"
            else
                cat "$_cc_err_tmp" >&2
            fi
        } always { rm -f "$_cc_err_tmp" 2>/dev/null }
    else
        _cc_config_stamp
        command claude "${flags[@]}" -n "$name" "$@"
    fi
}
