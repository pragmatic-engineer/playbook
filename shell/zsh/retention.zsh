# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# cc module: retention (bound disk)
#
# Keep only the newest $CCD_KEEP transcripts (default 5) for the current
# project; delete older ones plus their tool-result sidecar + runtime state.
# CCD_KEEP=0 disables. A floor of 2 protects fork/clean parents (always the 2nd
# newest). Uses zsh's `zstat` (portable) instead of the system `stat`, whose
# flags differ (BSD `stat -f` vs GNU `stat -c`) and would silently no-op off
# macOS, letting transcripts grow without bound. No glob qualifiers (they
# misbehave non-interactively).
_cc_prune() {
    local keep=${CCD_KEEP:-5}
    [[ "$keep" -le 0 ]] && return 0
    (( keep < 2 )) && keep=2
    local pd="$HOME/.claude/projects/${PWD//[^a-zA-Z0-9]/-}"
    [[ -d "$pd" ]] || return 0
    zmodload -F zsh/stat b:zstat 2>/dev/null
    local -a ranked
    local f m
    ranked=( ${(f)"$(
        for f in ${(f)"$(find "$pd" -maxdepth 1 -name '*.jsonl' -type f 2>/dev/null)"}; do
            m="$(zstat +mtime -- "$f" 2>/dev/null)" || continue
            printf '%s %s\n' "$m" "$f"
        done | sort -rn | cut -d' ' -f2-
    )"} )
    (( ${#ranked} > keep )) || return 0
    local sid
    for f in "${(@)ranked[keep+1,-1]}"; do
        sid="${f:t:r}"
        rm -f  "$f" 2>/dev/null
        rm -rf "$pd/$sid" "$HOME/.claude/runtime/$sid" 2>/dev/null
    done
}
