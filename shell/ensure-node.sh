# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# shellcheck shell=bash
# (sourced by setup-local.sh; no shebang)
#
# ensure-node.sh: detect an existing Node and use it; install only if none is
# found. Respects a node already on PATH (nvm, fnm, volta, a system package, or
# a prior brew install) instead of forcing a duplicate that could shadow the
# active version. The statusline reads `node --version` from PATH, so whichever
# node wins on PATH is the one it shows.
#
# The function uses only shell builtins plus node and brew, so it is unit-tested
# in isolation with a minimal PATH of stubs (shell/ensure-node.test.sh).

# ensure_node: if a node is on PATH, keep and report it. Otherwise install via
# Homebrew when available, else print guidance. Returns non-zero only when an
# attempted install fails.
ensure_node() {
    if command -v node >/dev/null 2>&1; then
        printf 'ensure-node: using the Node already installed: %s (%s)\n' \
            "$(node --version 2>/dev/null)" "$(command -v node)"
        return 0
    fi
    if command -v brew >/dev/null 2>&1; then
        printf 'ensure-node: no Node found; installing via Homebrew\n'
        brew install node </dev/null
        return $?
    fi
    printf 'ensure-node: no Node found and Homebrew is unavailable; install Node from https://nodejs.org\n' >&2
    return 0
}
