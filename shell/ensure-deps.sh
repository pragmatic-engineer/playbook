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
        if brew install "$formula" </dev/null; then
            return 0
        fi
        # Homebrew refuses formulae from an untrusted third-party tap. Trusting a
        # tap grants it code execution, so print the command rather than running
        # it: that decision belongs to the user, not to a setup script.
        case "$formula" in
            */*) printf 'ensure-deps: if this failed as an untrusted tap, review it and run: brew trust %s\n' "${formula%/*}" >&2 ;;
        esac
        return 1
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
    # persists. Formula names come from the `brew "X"` lines only; each
    # formula's command equals its name.
    # Taps first: a formula from a third-party tap cannot install until its tap
    # is added, so `tap "owner/repo"` lines have to be processed before the
    # `brew "X"` loop below. Tapping is read-only (a git clone); it does not by
    # itself let the tap run code, which still needs an explicit `brew trust`.
    if command -v brew >/dev/null 2>&1; then
        while IFS= read -r tapname; do
            [ -n "$tapname" ] || continue
            if brew tap | grep -qxF "$tapname"; then
                printf 'ensure-deps: tap %s already present\n' "$tapname"
            else
                printf 'ensure-deps: adding tap %s\n' "$tapname"
                brew tap "$tapname" </dev/null || rc=1
            fi
        done <<EOF
$(grep -oE '^tap "[^"]+"' "$brewfile" 2>/dev/null | sed -E 's/^tap "([^"]+)"$/\1/')
EOF
    fi

    while IFS= read -r formula; do
        [ -n "$formula" ] || continue
        ensure_dep "${formula##*/}" "$formula" || rc=1
    done <<EOF
$(grep -oE '^brew "[^"]+"' "$brewfile" 2>/dev/null | sed -E 's/^brew "([^"]+)"$/\1/')
EOF
    return "$rc"
}
