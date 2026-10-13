#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# Bump one pinned internal git dependency in Cargo.toml to a released tag.
#
#   .github/scripts/bump-internal-crate.sh <crate> <owner/repo> <tag>
#
# Rewrites only the `tag = "..."` of that crate's git dependency, then runs
# `cargo update -p <crate>` so Cargo.lock moves for that crate alone. Before
# touching anything it checks that <tag> is the latest release of <owner/repo>
# and that the release carries a SHA256SUMS asset. When Cargo.toml already
# pins <tag> it prints a message and exits 0 with no change.
#
# BUMP_ROOT overrides the directory holding Cargo.toml (used by the tests).
# Exit codes: 0 done or already current, 1 refused, 2 bad usage.
# shellcheck shell=bash
set -euo pipefail

die() { echo "bump-internal-crate: $*" >&2; exit "${2:-1}"; }

[ "$#" -eq 3 ] || die "usage: $0 <crate> <owner/repo> <tag>" 2
crate=$1
repo=$2
tag=$3

[[ $crate =~ ^[A-Za-z0-9_-]+$ ]] || die "bad crate name: $crate" 2
[[ $repo =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || die "repo must be owner/name: $repo" 2
[[ $tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "tag must look like v1.2.3: $tag" 2

root=${BUMP_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}
manifest="$root/crates/cli/Cargo.toml"
[ -f "$manifest" ] || manifest="$root/Cargo.toml"
[ -f "$manifest" ] || die "no Cargo.toml in $root"

line=$(grep -E "^${crate}[[:space:]]*=[[:space:]]*\{.*git[[:space:]]*=" "$manifest" || true)
[ -n "$line" ] || die "no git dependency named $crate in $manifest"
[ "$(printf '%s\n' "$line" | wc -l)" -eq 1 ] || die "more than one $crate entry in $manifest"
case $line in
  *"github.com/$repo\""*|*"github.com/$repo.git\""*) ;;
  *) die "$crate does not point at github.com/$repo" ;;
esac
current=$(printf '%s\n' "$line" | sed -nE 's/.*tag[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p')
[ -n "$current" ] || die "$crate has no exact tag pin"

if [ "$current" = "$tag" ]; then
  echo "$crate is already pinned to $tag. Nothing to do."
  exit 0
fi

latest=$(gh release view --repo "$repo" --json tagName --jq .tagName) \
  || die "cannot read the latest release of $repo"
[ "$latest" = "$tag" ] || die "$tag is not the latest release of $repo (latest is $latest)"

assets=$(gh release view "$tag" --repo "$repo" --json assets --jq '.assets[].name') \
  || die "cannot read the assets of $repo $tag"
printf '%s\n' "$assets" | grep -qx 'SHA256SUMS' || die "$repo $tag has no SHA256SUMS asset"

tmp=$(mktemp)
sed -E "/^${crate}[[:space:]]*=/s/tag[[:space:]]*=[[:space:]]*\"[^\"]+\"/tag = \"${tag}\"/" "$manifest" >"$tmp"
cat "$tmp" >"$manifest"
rm -f "$tmp"

(cd "$root" && cargo update -p "$crate")
echo "Bumped $crate from $current to $tag."
