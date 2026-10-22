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
# Exit codes: 0 rendered; 1 bad arguments or version; 2 bad SHA256SUMS (empty,
# missing or duplicate target, no hash, hash that is not 64 hex digits).
# A refused run writes nothing.
set -euo pipefail

die() { echo "FAIL $2" >&2; exit "$1"; }

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
