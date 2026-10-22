#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# render-pkgbuild.sh: renders the AUR PKGBUILD of playbook-bin from the
# version and the SHA256SUMS of a release.
#
#   render-pkgbuild.sh <version> <SHA256SUMS> <out> [pkgrel] [epoch]
#     Fills packaging/aur/PKGBUILD.tmpl and writes <out>. It reads only the
#     lines for playbook-<version>-<target> of the two Linux musl targets and
#     ignores every other line. A file name may follow the hash after two
#     spaces or after " *". pkgrel defaults to 1 and epoch to none; the
#     release job passes the values of the AUR repository to keep them.
#
#   render-pkgbuild.sh keep <version> <PKGBUILD>
#     Prints the pkgrel and epoch a render of <version> must carry, as the
#     lines `pkgrel=N` and `epoch=M`, read from the PKGBUILD now on the AUR.
#     The same pkgver keeps its pkgrel, so a manual fix is not overwritten;
#     a new pkgver gives pkgrel=1. The epoch is always kept, and is empty when
#     there is none. A missing file gives `pkgrel=1` and an empty epoch. The
#     file is read as text and never run.
#
# Exit codes: 0 rendered; 1 bad arguments or version; 2 bad SHA256SUMS (empty,
# missing or duplicate target, no hash, hash that is not 64 hex digits).
# A refused run writes nothing.
set -euo pipefail

die() { echo "FAIL $2" >&2; exit "$1"; }

if [ "${1:-}" = keep ]; then
  [ $# -eq 3 ] || die 1 "usage: render-pkgbuild.sh keep <version> <PKGBUILD>"
  [[ "$2" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die 1 "bad version: '$2' is not MAJOR.MINOR.PATCH"
  have_ver="" have_rel="" have_epoch=""
  if [ -f "$3" ]; then
    have_ver="$(sed -n "s/^pkgver=['\"]\{0,1\}\([^'\"]*\)['\"]\{0,1\}\$/\1/p" "$3" | head -n 1)"
    have_rel="$(sed -n "s/^pkgrel=['\"]\{0,1\}\([^'\"]*\)['\"]\{0,1\}\$/\1/p" "$3" | head -n 1)"
    have_epoch="$(sed -n "s/^epoch=['\"]\{0,1\}\([^'\"]*\)['\"]\{0,1\}\$/\1/p" "$3" | head -n 1)"
  fi
  [[ -z "$have_rel" || "$have_rel" =~ ^[1-9][0-9]*$ ]] || die 1 "bad pkgrel in $3: '$have_rel' is not a positive integer"
  [[ -z "$have_epoch" || "$have_epoch" =~ ^[0-9]+$ ]] || die 1 "bad epoch in $3: '$have_epoch' is not an integer"
  if [ "$have_ver" = "$2" ] && [ -n "$have_rel" ]; then echo "pkgrel=$have_rel"; else echo "pkgrel=1"; fi
  echo "epoch=$have_epoch"
  exit 0
fi

[ $# -ge 3 ] && [ $# -le 5 ] || die 1 "usage: render-pkgbuild.sh <version> <SHA256SUMS> <out> [pkgrel] [epoch]"
version="$1" sums="$2" out="$3" pkgrel="${4:-1}" epoch="${5:-}"

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die 1 "bad version: '$version' is not MAJOR.MINOR.PATCH"
[[ "$pkgrel" =~ ^[1-9][0-9]*$ ]] || die 1 "bad pkgrel: '$pkgrel' is not a positive integer"
[[ -z "$epoch" || "$epoch" =~ ^[0-9]+$ ]] || die 1 "bad epoch: '$epoch' is not an integer"

tmpl="$(cd "$(dirname "$0")/.." && pwd)/packaging/aur/PKGBUILD.tmpl"
[ -f "$tmpl" ] || die 1 "no template: $tmpl"
[ -f "$sums" ] || die 2 "no sums: $sums is not a file"
[ -s "$sums" ] || die 2 "no sums: $sums is empty"

# sum_of <target>: prints the hash of playbook-<version>-<target>.
sum_of() {
  local name="playbook-$version-$1" matches hash
  matches="$(awk -v n="$name" '{ f = $NF; sub(/^\*/, "", f); if (f == n || $0 == n) print }' "$sums")"
  [ -n "$matches" ] || die 2 "missing target: $1"
  [ "$(wc -l <<<"$matches" | tr -d ' ')" -eq 1 ] || die 2 "duplicate target: $1"
  hash="$(awk '{ print $1 }' <<<"$matches")"
  [[ "$hash" =~ ^[0-9a-f]{64}$ ]] || die 2 "bad hash for $name: '$hash' is not 64 lowercase hex digits"
  printf '%s' "$hash"
}

x86="$(sum_of x86_64-unknown-linux-musl)"
arm="$(sum_of aarch64-unknown-linux-musl)"
epoch_line=""
if [ -n "$epoch" ] && [ "$epoch" -gt 0 ]; then epoch_line="epoch=$epoch"$'\n'; fi

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
V="$version" R="$pkgrel" X="$x86" A="$arm" E="$epoch_line" awk '
  {
    gsub(/@VERSION@/, ENVIRON["V"]); gsub(/@PKGREL@/, ENVIRON["R"])
    gsub(/@SHA256_X86_64@/, ENVIRON["X"]); gsub(/@SHA256_AARCH64@/, ENVIRON["A"])
    if ($0 ~ /^@EPOCH_LINE@/) { sub(/^@EPOCH_LINE@/, ENVIRON["E"]) }
    print
  }
' "$tmpl" > "$tmp"
cat "$tmp" > "$out"
