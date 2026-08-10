# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Cache buster for the Claude Code launcher (sourced by cc.zsh).
#
# Clears stale shell-snapshots, per-session config-hashes, and the plugin
# catalog cache before every launch so the statusline and settings always load
# fresh from ~/.claude/settings.json.
#
# (N) = null-glob qualifier: a pattern that matches nothing expands to nothing
# instead of raising zsh's nomatch error.
_cc_bust_cache() {
    local claude_dir="$HOME/.claude"

    # Shell snapshots can freeze a stale statusLine or env into the session.
    rm -f "$claude_dir"/shell-snapshots/snapshot-*.sh(N) 2>/dev/null

    # config-hash files let CC skip re-reading settings when resuming. Nuke
    # them so every launch re-evaluates settings.json + plugins + statusline.
    find "$claude_dir/runtime" -name "config-hash" -delete 2>/dev/null

    # Plugin catalog cache can hold a stale compiled plugin list.
    rm -f "$claude_dir/plugins/plugin-catalog-cache.json"(N) 2>/dev/null

    # Backups dir: keep clean.
    rm -f "$claude_dir"/backups/*(N) 2>/dev/null
}
