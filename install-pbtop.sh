#!/bin/sh
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# Installer for pbtop alone, the standalone usage dashboard, on a machine
# without playbook. The README shows the curl command that pipes it to sh.
#
# It runs install.sh in its pbtop mode. That fetches
# pbtop-<version>-<target>.tar.gz from the latest release, or from the
# release named by PLAYBOOK_REF, checks its line in SHA256SUMS, its build
# attestation when gh can check it, and `pbtop --version`, then installs pbtop
# into PLAYBOOK_BIN_DIR (default $HOME/.local/bin) and puts that folder on
# PATH. Nothing else runs. Run from a checkout, it uses the install.sh next
# to it, else the one on main. Homebrew has the same binary as the `pbtop`
# formula of pragmatic-engineer/tap.
set -eu

command -v bash >/dev/null 2>&1 || { echo "error: bash is required" >&2; exit 1; }

case "$0" in
    */install-pbtop.sh)
        here="$(cd "$(dirname "$0")" && pwd)"
        if [ -f "$here/install.sh" ]; then
            PLAYBOOK_INSTALL_ONLY=pbtop exec bash "$here/install.sh" "$@"
        fi
        ;;
esac

command -v curl >/dev/null 2>&1 || { echo "error: curl is required" >&2; exit 1; }
url="https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh"
script="$(curl -fsSL "$url")" || { echo "error: could not download $url" >&2; exit 1; }
PLAYBOOK_INSTALL_ONLY=pbtop exec bash -c "$script" install.sh "$@"
