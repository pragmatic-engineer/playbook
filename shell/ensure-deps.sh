# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# shellcheck shell=bash
# (sourced by setup-local.sh; no shebang)
#
# ensure-deps.sh: for each toolkit dependency, use the version already installed
# (from any source: Homebrew, nvm, pyenv, a system package, a manual install)
# and install via Homebrew only when the tool is not found on PATH. This
# generalizes the earlier node-only check to every dependency, so an existing
# tool is never shadowed by a duplicate brew install.
#
# The Brewfile stays the canonical dependency list; ensure_all_deps reads the
# formula names from it, so adding a dep there is enough (no second list to keep
# in sync). Uses only shell builtins plus grep, sed, and brew, so it is unit
# tested in isolation with stubbed tools.

# Minimum Python the hooks need (no-dash-guard.sh, rebuild-memory-graph.sh use
# python3 features that require >= 3.9).
PY_MIN_MAJOR=3
PY_MIN_MINOR=9

# ensure_python <formula>: ensure a python3 of at least PY_MIN on PATH. A python3
# that is present but too old does not satisfy the hooks, so it is treated like
# absent and <formula> is installed via Homebrew. Returns non-zero only when an
# attempted install fails. Kept separate from ensure_dep because only Python
# carries a version floor: presence alone is not enough.
ensure_python() {
    local formula="$1" ver=""
    if command -v python3 >/dev/null 2>&1; then
        ver="$(python3 -c 'import sys; print("%d.%d.%d" % sys.version_info[:3])' 2>/dev/null)"
        if python3 -c "import sys; sys.exit(0 if sys.version_info[:2] >= (${PY_MIN_MAJOR}, ${PY_MIN_MINOR}) else 1)" 2>/dev/null; then
            printf 'ensure-deps: python3 %s already installed (%s)\n' "$ver" "$(command -v python3)"
            return 0
        fi
        printf 'ensure-deps: python3 %s is older than %d.%d; installing %s via Homebrew\n' \
            "$ver" "$PY_MIN_MAJOR" "$PY_MIN_MINOR" "$formula"
    else
        printf 'ensure-deps: python3 not found; installing %s via Homebrew\n' "$formula"
    fi
    if command -v brew >/dev/null 2>&1; then
        brew install "$formula" </dev/null
        return $?
    fi
    printf 'ensure-deps: cannot install %s: Homebrew is unavailable; install Python %d.%d+ manually\n' \
        "$formula" "$PY_MIN_MAJOR" "$PY_MIN_MINOR" >&2
    return 0
}

# ensure_dep <command> <formula> [label]: keep <command> if it is on PATH,
# otherwise install <formula> via Homebrew when brew is available, otherwise
# print guidance. Returns non-zero only when an attempted install fails.
ensure_dep() {
    local cmd="$1" formula="$2" label="${3:-$1}"
    if command -v "$cmd" >/dev/null 2>&1; then
        printf 'ensure-deps: %s already installed (%s)\n' "$label" "$(command -v "$cmd")"
        return 0
    fi
    if command -v brew >/dev/null 2>&1; then
        printf 'ensure-deps: %s not found; installing %s via Homebrew\n' "$label" "$formula"
        brew install "$formula" </dev/null
        return $?
    fi
    printf 'ensure-deps: %s not found and Homebrew is unavailable; install %s manually\n' "$label" "$formula" >&2
    return 0
}

# ensure_all_deps <brewfile>: check-then-install every `brew "X"` formula listed
# in <brewfile>, respecting a version already on PATH. Returns non-zero if any
# install failed.
ensure_all_deps() {
    local brewfile="$1" formula cmd rc=0
    if [ ! -f "$brewfile" ]; then
        printf 'ensure-deps: no Brewfile at %s\n' "$brewfile" >&2
        return 0
    fi
    # The while runs in the current shell (heredoc redirect, not a pipe), so rc
    # persists. Formula names come from the `brew "X"` lines only. Python routes
    # to the version-aware check; every other formula's command equals its name.
    while IFS= read -r formula; do
        [ -n "$formula" ] || continue
        case "$formula" in
            python@*) ensure_python "$formula" || rc=1 ;;
            *)        ensure_dep "$formula" "$formula" || rc=1 ;;
        esac
    done <<EOF
$(grep -oE '^brew "[^"]+"' "$brewfile" 2>/dev/null | sed -E 's/^brew "([^"]+)"$/\1/')
EOF
    return "$rc"
}
