#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# render-scoop.sh: renders the scoop manifest of a release.
#
#   render-scoop.sh <version> <SHA256SUMS> <out.json>
#     Fills packaging/scoop/playbook.json.tmpl with the version and the hash
#     of playbook-<version>-x86_64-pc-windows-msvc.exe, read from the
#     SHA256SUMS of the release. Every other line is ignored. A file name may
#     follow the hash after two spaces or after " *". Makes no network call.
#
#     Exit 0 rendered. Exit 1 usage or a version that is not X.Y.Z. Exit 2
#     a bad sums file: not a file, empty, no Windows line, more than one, or
#     a hash that is not 64 lowercase hex digits. A refused run writes
#     nothing.
set -euo pipefail

die() { echo "render-scoop: $*" >&2; exit "$1"; }

[ $# -eq 3 ] || die 1 "usage: render-scoop.sh <version> <SHA256SUMS> <out.json>"
version="$1" sums="$2" out="$3"

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die 1 "bad version: '$version' is not X.Y.Z"
[ -f "$sums" ] || die 2 "$sums is not a file"
[ -s "$sums" ] || die 2 "$sums is empty"

here="$(cd "$(dirname "$0")/.." && pwd)"
tmpl="$here/packaging/scoop/playbook.json.tmpl"
[ -f "$tmpl" ] || die 2 "template missing: $tmpl"

name="playbook-$version-x86_64-pc-windows-msvc.exe"
matches="$(awk -v n="$name" '{ f = $NF; sub(/^\*/, "", f); if (f == n) print }' "$sums")"
[ -n "$matches" ] || die 2 "no Windows line for $name in $sums"
[ "$(wc -l <<<"$matches" | tr -d ' ')" -eq 1 ] || die 2 "more than one Windows line for $name"
hash="$(awk '{ print $1 }' <<<"$matches")"
[[ "$hash" =~ ^[0-9a-f]{64}$ ]] || die 2 "hash for $name is not 64 lowercase hex digits"

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
sed -e "s/@VERSION@/$version/g" -e "s/@HASH@/$hash/g" "$tmpl" > "$tmp"
cat "$tmp" > "$out"
