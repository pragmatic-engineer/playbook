# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# shellcheck shell=bash
# (sourced, no shebang; the directive tells shellcheck the dialect)
#
# retention.sh: bound disk use for transcripts.
#
# Keep only the newest $CCD_KEEP transcripts (default 5) for the current
# project; delete older ones plus their tool-result sidecar and runtime state.
# CCD_KEEP=0 disables. A floor of 2 protects fork/clean parents (always the
# 2nd newest). Uses portable stat (BSD stat -f or GNU stat -c) to get mtime,
# falling back to 0. No zsh-only builtins or glob qualifiers.
#
# Sourceable in bash and zsh. No explicit array indexing by number.
_cc_prune() {
    local keep=${CCD_KEEP:-5}
    [ "$keep" -le 0 ] && return 0
    [ "$keep" -lt 2 ] && keep=2
    local pd
    pd="$HOME/.claude/projects/${PWD//[^a-zA-Z0-9]/-}"
    [ -d "$pd" ] || return 0

    local -a ranked=()
    local f m
    while IFS= read -r f; do
        ranked+=("$f")
    done < <(
        while IFS= read -r f; do
            m=$(stat -f %m "$f" 2>/dev/null || stat -c %Y "$f" 2>/dev/null || printf '0')
            printf '%s %s\n' "$m" "$f"
        done < <(find "$pd" -maxdepth 1 -name '*.jsonl' -type f 2>/dev/null) \
        | sort -rn | cut -d' ' -f2-
    )

    local total="${#ranked[@]}"
    [ "$total" -gt "$keep" ] || return 0

    local i=0 sid
    for f in "${ranked[@]}"; do
        i=$(( i + 1 ))
        [ "$i" -le "$keep" ] && continue
        sid="${f##*/}"; sid="${sid%.jsonl}"
        rm -f "$f" 2>/dev/null
        rm -rf "${pd:?}/${sid:?}" "$HOME/.config/playbook/runtime/${sid:?}" 2>/dev/null
    done
}
