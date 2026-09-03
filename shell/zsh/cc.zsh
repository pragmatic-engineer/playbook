# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Claude Code launcher: zsh entry point.
#
# Thin entry that sources the shared launcher modules from shell/shared/. The
# implementation is shared with the bash entry point (shell/bash/cc.sh), so the
# two shells behave identically: same subcommands (fresh, list, clean, raw,
# worktree, new), same config-drift auto-fork, same retention. There is no
# zsh-only or bash-only launcher logic anymore.
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
# Install: source this file from ~/.zshrc.
#   source ~/.config/playbook/shell/zsh/cc.zsh

source "$HOME/.config/playbook/shell/shared/bust-cache.sh"
source "$HOME/.config/playbook/shell/shared/worktree.sh"
source "$HOME/.config/playbook/shell/shared/config-drift.sh"
source "$HOME/.config/playbook/shell/shared/retention.sh"
source "$HOME/.config/playbook/shell/shared/sessions.sh"
source "$HOME/.config/playbook/shell/shared/clean-resume.sh"
source "$HOME/.config/playbook/shell/shared/dispatch.sh"
