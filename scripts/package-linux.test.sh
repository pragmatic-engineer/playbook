#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# package-linux.test.sh: offline checks of the .deb and .rpm packaging.
#
# Usage: package-linux.test.sh
#
# It checks the package names, that the release workflow downloads and counts
# exactly the assets those names make, and the pinned nfpm sums. When nfpm is
# on PATH it also builds both packages from fake binaries.
set -euo pipefail

cd "$(dirname "$0")/.."

fail() { echo "FAIL $*" >&2; exit 1; }
eq() { [ "$1" = "$2" ] || fail "$3: got '$1', want '$2'"; }

pk=scripts/package-linux.sh

eq "$("$pk" names 1.2.3 x86_64-unknown-linux-musl)" \
  "playbook_1.2.3_amd64.deb
playbook-1.2.3.x86_64.rpm" "x86_64 names"
eq "$("$pk" names 1.2.3 aarch64-unknown-linux-musl)" \
  "playbook_1.2.3_arm64.deb
playbook-1.2.3.aarch64.rpm" "aarch64 names"
"$pk" names 1.2.3 aarch64-apple-darwin >/dev/null 2>&1 && fail "a macOS target got package names"
"$pk" names 1.2 x86_64-unknown-linux-musl >/dev/null 2>&1 && fail "a short version got package names"

# The pinned sums are SHA-256 hex.
for v in NFPM_SHA256_X86_64 NFPM_SHA256_ARM64; do
  grep -Eq "^$v=[0-9a-f]{64}\$" "$pk" || fail "$v is not a 64 digit hex sum"
done

# The publish job downloads, counts and sums the assets by these patterns. A
# renamed package must fail here and not as a partial release.
wf=.github/workflows/release.yml
grep -qF 'for p in "playbook-$VERSION-*" "pbtop-$VERSION-*" "pbdoc-$VERSION-*" "playbook-$VERSION.*.rpm" "playbook_${VERSION}_*.deb"; do' "$wf" \
  || fail "$wf does not download the assets by these patterns"
grep -qF '[ "$count" -ne 18 ]' "$wf" || fail "$wf does not expect 18 assets"
grep -qF 'shasum -a 256 playbook-* playbook_* pbtop-* pbdoc-*' "$wf" || fail "$wf does not put the .deb in SHA256SUMS"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
version=1.2.3
for t in aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-pc-windows-msvc; do
  : > "$tmp/playbook-$version-$t"
  : > "$tmp/pbtop-$version-$t.tar.gz"
  case "$t" in *windows*) ;; *) : > "$tmp/pbdoc-$version-$t.tar.gz" ;; esac
done
for t in x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
  while IFS= read -r f; do : > "$tmp/$f"; done < <("$pk" names "$version" "$t")
done
(
  cd "$tmp"
  n=0
  for f in playbook-"$version"-* pbtop-"$version"-* pbdoc-"$version"-* playbook-"$version".*.rpm playbook_"${version}"_*.deb; do
    [ -e "$f" ] && n=$((n + 1))
  done
  eq "$n" 18 "assets the publish patterns match"
)

if command -v nfpm >/dev/null 2>&1; then
  bin="$tmp/bin" out="$tmp/out"
  mkdir -p "$bin"
  printf '#!/bin/sh\necho playbook %s\n' "$version" > "$bin/playbook"
  cp "$bin/playbook" "$bin/pbtop"
  chmod +x "$bin/playbook" "$bin/pbtop"
  "$pk" build "$version" x86_64-unknown-linux-musl "$bin" "$out" >/dev/null
  while IFS= read -r f; do [ -s "$out/$f" ] || fail "$f was not built"; done \
    < <("$pk" names "$version" x86_64-unknown-linux-musl)
  echo "ok: packages built with $(nfpm --version | sed -n 's/^GitVersion: *//p')"
else
  echo "skip: nfpm is not on PATH, so no package was built"
fi
echo "ok: package names, release patterns and pinned sums"
