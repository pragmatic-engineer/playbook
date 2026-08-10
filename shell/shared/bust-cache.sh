# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# shellcheck shell=bash
# (sourced, no shebang; the directive tells shellcheck the dialect)
#
# bust-cache.sh: clears stale shell-snapshots, per-session config-hashes, and
# the plugin catalog cache before every launch so the statusline and settings
# always load fresh from ~/.claude/settings.json.
#
# Sourceable in bash and zsh. No zsh-only builtins or glob qualifiers.
_cc_bust_cache() {
    local claude_dir="$HOME/.claude"

    # Shell snapshots can freeze a stale statusLine or env into the session.
    find "$claude_dir/shell-snapshots" -maxdepth 1 -name 'snapshot-*.sh' \
        -delete 2>/dev/null || true

    # config-hash files let CC skip re-reading settings when resuming. Nuke
    # them so every launch re-evaluates settings.json + plugins + statusline.
    find "$claude_dir/runtime" -name "config-hash" -delete 2>/dev/null || true

    # Plugin catalog cache can hold a stale compiled plugin list.
    rm -f "$claude_dir/plugins/plugin-catalog-cache.json" 2>/dev/null

    # Backups dir: keep clean.
    find "$claude_dir/backups" -mindepth 1 -maxdepth 1 -delete 2>/dev/null || true
}
