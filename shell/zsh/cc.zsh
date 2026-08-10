# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Claude Code launcher: drop-in replacement for ~/.zshrc's _claude that adds
# subcommand dispatch (clean / fresh / raw / list) while preserving the original
# resume-by-customTitle default behavior.
#
# Plays nice with your existing cc() and ccd() wrappers; they call _claude
# unchanged; this file just redefines _claude with extra capabilities.
#
# Subcommands (works under both cc and ccd):
#
#   cc                Default: resume the most recent session for $PWD whose
#                     customTitle matches the directory name. If none found,
#                     start fresh with that name. (Identical to your original.)
#
#   cc clean          Clone the latest matching transcript with config-override
#                     slash commands stripped, then resume the clone.
#                     Stripped: /model, /effort, /config, /output-style, /style.
#                     Result: conversation preserved, runtime config defaults
#                     back to ~/.claude/settings.json. Use after editing
#                     settings, plugins, or hooks.
#                     Original transcript is untouched.
#
#   cc fresh          Start a brand-new session (no resume). Use after big
#                     settings rewrites or when you want zero conversation
#                     baggage.
#
#   cc raw [sid]      Resume verbatim: no fork, no clean, preserves the
#                     original UUID and all frozen overrides. Defaults to the
#                     latest matching session if sid omitted.
#
#   cc list           Show recent sessions for $PWD with timestamps + titles.
#
#   cc worktree <branch> [env-base]   (alias: cc new <branch>)
#                     Create or enter a git worktree for <branch> off the base
#                     branch, then launch a session in it. Folder is named after
#                     the JIRA key in the branch. Claude auto-resolves rebase
#                     conflicts (--ai-resolve always set).
#
#   ccd <any of the above>: same dispatch, with --dangerously-skip-permissions.
#
# Default-path extras:
#   - Auto-fork on config drift: if settings.json or any hook changed since this
#     project last launched, the default resume forks to reload config (mints a
#     new transcript). No change → plain resume, no new file. Baseline tracked
#     in ~/.claude/cc-state/<project-slug>.
#   - Retention: after every cc/ccd, keeps only the newest $CCD_KEEP transcripts
#     (default 5) per project, deleting older ones + sidecar + runtime state.
#     Set CCD_KEEP=0 to disable. Floor of 2 protects fork/clean parents.
#
# Install: source this file from ~/.zshrc *after* your existing _claude/cc/ccd
# definitions. It overrides _claude only; cc() and ccd() in your zshrc stay.
#   source ~/.claude/shell/zsh/cc.zsh
#
# Layout: this file is the entry point. Implementation lives in focused modules
# under shell/zsh/, each loadable/editable on its own:
#   bust-cache.zsh    cache buster (cleared before every launch)
#   config-drift.zsh  config-hash tracking for the conditional fork
#   retention.zsh     transcript pruning (_cc_prune)
#   sessions.zsh      session lookup / enumerate / list
#   clean-resume.zsh  clone-and-strip resume (_cc_clean_resume)
#   dispatch.zsh      the _claude subcommand dispatcher

# ─── modules ─────────────────────────────────────────────────────────────────
# Function definitions are lazy, so module order only needs all of them loaded
# before cc/ccd run. bust-cache lives alongside the other modules under zsh/.
source "$HOME/.claude/shell/zsh/bust-cache.zsh"
source "$HOME/.claude/shell/shared/worktree.sh"
for _cc_mod in config-drift retention sessions clean-resume dispatch; do
    source "$HOME/.claude/shell/zsh/$_cc_mod.zsh"
done
unset _cc_mod

# ─── public wrappers ─────────────────────────────────────────────────────────
# cc/ccd both dispatch through _claude. Each carries the custom system prompt;
# ccd adds --dangerously-skip-permissions. (These are the leading flags _claude
# splits off and passes through to `command claude`.)
cc() {
    local -a _cc_sys=()
    [[ -f "$HOME/.claude/prompts/SYSTEM_PROMPT.md" ]] \
        && _cc_sys=(--system-prompt-file "$HOME/.claude/prompts/SYSTEM_PROMPT.md")
    _claude "${_cc_sys[@]}" "$@"; local rc=$?; _cc_prune; return $rc
}
ccd() {
    local -a _cc_sys=()
    [[ -f "$HOME/.claude/prompts/SYSTEM_PROMPT.md" ]] \
        && _cc_sys=(--system-prompt-file "$HOME/.claude/prompts/SYSTEM_PROMPT.md")
    _claude --dangerously-skip-permissions "${_cc_sys[@]}" "$@"; local rc=$?; _cc_prune; return $rc
}
