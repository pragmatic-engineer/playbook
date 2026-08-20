#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# migration-check.sh: the one hook hooks.json still registers. Detects a
# settings.json that install.sh's `playbook init` handoff never wired (a
# plugin update with no matching installer re-run) and tells the user to
# re-run the installer. Silent once settings.json already carries the
# ported hooks.
#
# Must work with no `playbook` binary on PATH, since that is exactly the
# state of a machine that needs the warning: it never invokes the binary
# itself, and a grep for one marker string is cheap enough to run on every
# SessionStart without adding measurable latency. Never fails the session:
# exits 0 on every path, including a missing or unreadable settings.json.
set -u

SETTINGS="$HOME/.claude/settings.json"

if [ -f "$SETTINGS" ] && grep -q 'playbook hook session-init' "$SETTINGS" 2>/dev/null; then
  exit 0
fi

printf '%s\n' '{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"Your playbook hooks are not wired to the installed binary yet. Re-run the installer to fix this: curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash"}}'
exit 0
