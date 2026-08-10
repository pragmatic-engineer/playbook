# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# shellcheck shell=bash
# (sourced, no shebang; the directive tells shellcheck the dialect)
#
# config-drift.sh: config-drift tracking (conditional fork).
#
# Determines whether runtime config changed since a project last launched, so
# the default resume can fork to reload settings/plugins/hooks only when needed.
# Baseline is stored per-project under ~/.claude/cc-state/<project-slug>.
# Sourceable in bash and zsh. No zsh-only builtins.

# Shared config hash (settings.json + hook scripts, excluding tests).
# shellcheck source=hooks/lib/config-hash.sh
source "$HOME/.claude/hooks/lib/config-hash.sh"
command -v config_hash >/dev/null 2>&1 \
    || printf '%s\n' "cc: config-hash.sh failed to load; config-drift disabled" >&2

# Per-project marker holding the config hash the project's session last ran.
_cc_config_marker() {
    printf '%s\n' "$HOME/.claude/cc-state/${PWD//[^a-zA-Z0-9]/-}"
}

# Record current config as this project's baseline (call when launching a
# session that already runs current config: fresh / clean / new).
_cc_config_stamp() {
    local m; m="$(_cc_config_marker)"
    mkdir -p "${m%/*}" 2>/dev/null
    config_hash > "$m" 2>/dev/null
}

# Echo "1" if config changed since this project's baseline; ALWAYS re-stamps to
# the current hash. Empty output = unchanged. Used to decide --fork-session.
_cc_config_drifted() {
    local m cur stored
    m="$(_cc_config_marker)"
    cur="$(config_hash)"
    stored="$(cat "$m" 2>/dev/null)"
    mkdir -p "${m%/*}" 2>/dev/null
    printf '%s\n' "$cur" > "$m" 2>/dev/null
    [ "$stored" != "$cur" ] && printf '%s\n' "1"
}
