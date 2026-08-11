# shellcheck shell=sh
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Shared config hash: settings.json + hook scripts (excluding tests).
# Hook scripts are matched in both languages (.sh and .py) so the hash keeps
# covering every hook after the python migration; test files in either language
# are excluded so editing a test never trips a false config-drift warning.
# Sourceable by both bash hooks and the zsh cc modules.
config_hash() {
    {
        cat "$HOME/.claude/settings.json" 2>/dev/null
        find "$HOME/.claude/hooks" \( -name '*.sh' -o -name '*.py' \) \
            ! -name '*.test.sh' ! -name '*.test.py' ! -name '*_test.py' \
            -type f -print0 2>/dev/null |
            sort -z | xargs -0 cat 2>/dev/null
    } | shasum -a 256 2>/dev/null | cut -c1-16
}
