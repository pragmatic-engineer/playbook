#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# render-formula.sh <version> <SHA256SUMS>: print the Homebrew formula for a
# release, filling the four url+sha256 pairs from the release's SHA256SUMS.
set -euo pipefail

version="${1:?usage: render-formula.sh <version> <SHA256SUMS>}"
sums="${2:?usage: render-formula.sh <version> <SHA256SUMS>}"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "render-formula: bad version '$version'" >&2; exit 1 ;;
esac

out="$(cat "$here/formula.rb.tmpl")"
out="${out//@VERSION@/$version}"

for target in aarch64-apple-darwin x86_64-apple-darwin \
  aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
  sha="$(awk -v f="playbook-${version}-${target}" '$2 == f || $2 == "*" f { print $1 }' "$sums")"
  if ! [[ "$sha" =~ ^[0-9a-f]{64}$ ]]; then
    echo "render-formula: no valid sha256 for $target in $sums" >&2
    exit 1
  fi
  key="$(echo "$target" | tr 'a-z-' 'A-Z_')"
  out="${out//@SHA_${key}@/$sha}"
done

printf '%s\n' "$out"
