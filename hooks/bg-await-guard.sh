#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# PreToolUse(Bash) guard: nudge away from backgrounding a command you must await.
#
# When a Bash call sets run_in_background on a package install, build, or
# typecheck (the things a later step almost always depends on), remind the model
# to run it in the FOREGROUND with an extended timeout instead of backgrounding
# it and then racing or polling. This is the failure behind the recurring
# "npm run build before npm install finished -> tsc: not found" and the reach
# for Monitor/`wait` to synchronize a job that shell state can't reach.
#
# Warn only, never block: backgrounding a genuinely long job and awaiting its
# completion notification is legitimate; racing it is the bug. Emits a single
# additionalContext line. Disable with BG_AWAIT_GUARD=0.
# shellcheck source=hooks/lib/common.sh
. "$(dirname "$0")/lib/common.sh"

[[ "${BG_AWAIT_GUARD:-1}" == "0" ]] && exit 0

CMD="$(hi_field '.tool_input.command')"
[[ -z "$CMD" ]] && exit 0

# Only act on backgrounded commands. jq renders the JSON boolean as "true".
BG="$(hi_field '.tool_input.run_in_background')"
case "$BG" in true|1|yes) ;; *) exit 0 ;; esac

# Await-sensitive commands: package installs, builds, and typechecks whose
# output a later step usually needs, plus a node_modules wipe (its reinstall is
# always awaited).
await_regexp='(npm|pnpm|yarn|bun)[[:space:]]+(install|ci|add|i)([[:space:]]|$)'
await_regexp+='|(npm|pnpm|yarn|bun)[[:space:]]+run[[:space:]]+build'
await_regexp+='|(^|[;&|[:space:]])(tsc|make|gradle|mvn)([[:space:]]|$)'
await_regexp+='|(cargo|go)[[:space:]]+build'
await_regexp+='|(pip|poetry|bundle)[[:space:]]+install'
await_regexp+='|rm[[:space:]]+-[a-z]*[[:space:]]*.*node_modules'

if printf '%s' "$CMD" | grep -qiE "$await_regexp"; then
  emit_pre_context "PreToolUse" \
"You backgrounded a command whose result a later step usually needs (install/build/typecheck). Backgrounding it and then running the next command is a common failure: e.g. \`npm run build\` before \`npm install\` finished gives \`tsc: not found\`. If anything downstream depends on this, run it in the FOREGROUND (run_in_background off) with an extended timeout (up to 600000ms) instead. A backgrounded job re-invokes you only when it exits; shell state and \`wait\` do NOT persist across Bash calls, so don't poll it (no Monitor/\`wait\` to synchronize)."
fi
exit 0
