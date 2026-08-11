# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# Dependencies for this ~/.claude config (hooks, shell/, statusline).
#
# The canonical dependency list. `/setup` reads the `brew "X"` formula names
# from here and runs a per-dependency check-then-install (shell/ensure-deps.sh):
# for each one it uses the version already on PATH (brew, nvm, pyenv, system, a
# manual install) and installs via brew only when the tool is missing, so an
# existing tool is never shadowed by a duplicate brew install. You can still run
# `brew bundle --file ~/.claude/Brewfile` directly, which installs the brew
# formula for each regardless of a non-brew copy.
#
# Not available via Homebrew, install separately:
#   - claude  (Claude Code)  https://docs.claude.com/en/docs/claude-code  (npm i -g @anthropic-ai/claude-code, or the native installer)

# Core: required by hooks and the cc launcher
brew "git"          # used everywhere; hooks drive git directly
brew "jq"           # JSON parsing in hooks and statusline.sh
brew "python@3.13"  # hooks (Python >=3.9)
brew "rtk"          # CLI proxy that a PreToolUse hook routes every Bash command through

# Statusline and PR/CI integration
brew "gh"           # statusline PR and CI status (optional but recommended)
brew "node"         # statusline shows the active Node version (installed only if no node is on PATH)

# Browser automation (optional): agent-browser MCP, used by /brainstorm for web-only tickets and attachments
brew "agent-browser"  # register: claude mcp add --scope user agent-browser -- agent-browser mcp --tools core

# zsh ships with macOS; the `cc` launcher is zsh-only. Uncomment to pin a Homebrew zsh.
# brew "zsh"

# macOS ships bash 3.2; the hooks use no bash 4+ features so the system bash works.
# Uncomment for a modern bash.
# brew "bash"
