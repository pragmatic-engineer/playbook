#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# package-smoke.sh <dir> <version> <target>: install the .deb and .rpm of a
# release in clean containers and check them (ADR 0039).
#
# <dir> holds the two packages that `package-linux.sh names <version> <target>`
# lists. The target decides the architecture, and it must be the one of the
# machine, because the containers run natively. The .deb goes into Debian and
# Ubuntu, the .rpm into Fedora. In each container the script
#   1. installs the package with apt or dnf (no sudo: the container user is
#      root and no host file changes),
#   2. runs `playbook --version`, `pb --version` and `pbtop --version`, which
#      must all report <version>,
#   3. writes a file under ~/.config/playbook, upgrades the package in place
#      and removes it, and checks that the file is still there.
#
# The images are pinned by digest. The packages have no dependencies, so the
# containers need no distro mirror, only the registry that holds the image.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

DEBIAN=docker.io/library/debian@sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587    # 12-slim
UBUNTU=docker.io/library/ubuntu@sha256:68ee22004a9109b44973313b02ff7b1de250f1de0f5ecb628d73f579aaff33ac    # 24.04
FEDORA=docker.io/library/fedora@sha256:99e203b80b1c3d8f7e161ec10a68fd02b081ef83a3963553e513c82846b97814    # 42

fail() { echo "FAIL $*" >&2; exit 1; }

dir="${1:-}" version="${2:-}" target="${3:-}"
[ -d "$dir" ] && [ -n "$version" ] && [ -n "$target" ] \
  || { echo "usage: package-smoke.sh <dir> <version> <target>" >&2; exit 2; }
dir="$(cd "$dir" && pwd)"
{ read -r deb; read -r rpm; } < <("$ROOT/scripts/package-linux.sh" names "$version" "$target")
[ -s "$dir/$deb" ] || fail "$dir/$deb is missing"
[ -s "$dir/$rpm" ] || fail "$dir/$rpm is missing"

# The script each container runs. $1 is the package, $2 the version, and the
# install, upgrade and remove commands come from the environment.
inner='
set -eu
pkg="/pkg/$1"; want="$2"
eval "$INSTALL"
for b in playbook pb pbtop; do
  out="$("$b" --version)"
  [ "${out##* }" = "$want" ] || { echo "$b --version says $out, not $want" >&2; exit 1; }
done
[ "$(readlink /usr/bin/pb)" = playbook ] || { echo "/usr/bin/pb is not a link to playbook" >&2; exit 1; }
mkdir -p "$HOME/.config/playbook"
echo keep > "$HOME/.config/playbook/user.conf"
eval "$UPGRADE"
eval "$REMOVE"
[ "$(cat "$HOME/.config/playbook/user.conf")" = keep ] || { echo "user config changed" >&2; exit 1; }
[ ! -e /usr/bin/playbook ] || { echo "playbook is still installed after the removal" >&2; exit 1; }
'

run() {
  local name="$1" image="$2" file="$3" install="$4" upgrade="$5" remove="$6"
  echo "== $name"
  docker run --rm --network none -v "$dir:/pkg:ro" \
    -e INSTALL="$install" -e UPGRADE="$upgrade" -e REMOVE="$remove" \
    "$image" sh -c "$inner" sh "$file" "$version"
  echo "ok $name"
}

deb_install='apt-get install -y "$pkg" >/dev/null'
deb_upgrade='dpkg -i "$pkg" >/dev/null'
deb_remove='apt-get remove -y playbook >/dev/null'
run debian "$DEBIAN" "$deb" "$deb_install" "$deb_upgrade" "$deb_remove"
run ubuntu "$UBUNTU" "$deb" "$deb_install" "$deb_upgrade" "$deb_remove"
run fedora "$FEDORA" "$rpm" \
  'dnf install -y --disablerepo="*" "$pkg" >/dev/null' \
  'rpm -U --replacepkgs "$pkg"' \
  'dnf remove -y --disablerepo="*" playbook >/dev/null'
echo "Smoke tested $deb and $rpm in Debian, Ubuntu and Fedora."
