# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# shellcheck shell=bash
# (sourced, so it has no shebang; the directive tells shellcheck the dialect)
# Claude Code launcher: bash entry point.
#
# Thin entry that sources the shared launcher modules from shell/shared/. The
# implementation is shared with the zsh entry point (shell/zsh/cc.zsh), so bash
# and zsh behave identically: same subcommands (fresh, list, clean, raw,
# worktree, new), same config-drift auto-fork, same retention. The old
# bash-only reimplementation is gone, and with it the drift where cc clean and
# cc raw silently fell through to a plain resume.
#
# Subcommands (work under both cc and ccd):
#   cc                Resume this dir's most recent session by customTitle, or
#                     start fresh. Forks a new transcript on config drift.
#   cc clean          Clone the latest transcript with config-override slash
#                     commands stripped, then resume the clone.
#   cc fresh          Start a brand-new session (no resume).
#   cc raw [sid]      Resume verbatim: no fork, overrides preserved.
#   cc list           Show recent sessions for this dir.
#   cc worktree <b>   (alias: cc new <b>) Create/enter a git worktree, launch
#                     a session in it.
#   ccd <any>         Same, with --dangerously-skip-permissions.
#
# Install: source this file from ~/.bashrc.
#   source ~/.config/playbook/shell/bash/cc.sh

source "$HOME/.config/playbook/shell/shared/bust-cache.sh"
# shellcheck source=shell/shared/worktree.sh
source "$HOME/.config/playbook/shell/shared/worktree.sh"
source "$HOME/.config/playbook/shell/shared/config-drift.sh"
source "$HOME/.config/playbook/shell/shared/retention.sh"
source "$HOME/.config/playbook/shell/shared/sessions.sh"
source "$HOME/.config/playbook/shell/shared/clean-resume.sh"
source "$HOME/.config/playbook/shell/shared/dispatch.sh"
