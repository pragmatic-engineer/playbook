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

# _dep_command_for <formula>: the command that proves <formula> is installed.
# The command name equals the formula name except where they differ, e.g. the
# python@<version> formula is proven by python3.
_dep_command_for() {
    case "$1" in
        python@*) printf 'python3' ;;
        *)        printf '%s' "$1" ;;
    esac
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
    # persists. Formula names come from the `brew "X"` lines only.
    while IFS= read -r formula; do
        [ -n "$formula" ] || continue
        cmd="$(_dep_command_for "$formula")"
        ensure_dep "$cmd" "$formula" || rc=1
    done <<EOF
$(grep -oE '^brew "[^"]+"' "$brewfile" 2>/dev/null | sed -E 's/^brew "([^"]+)"$/\1/')
EOF
    return "$rc"
}
