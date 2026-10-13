#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# Builds one marketplace plugin archive from an allowlist file.
#
#   build-plugin-archive.sh <tree-ish> <allowlist> <out.zip>
#
# The allowlist holds one git pathspec per line. The archive is cut with
# `git archive`, so only tracked files can ship. A listed
# `.claude-plugin/hooks.plugin.json` is renamed to `.claude-plugin/plugin.json`
# inside the zip, because a plugin root holds one manifest and git archive
# cannot rename. Entries are stamped with the commit time and sorted, so the
# same commit yields the same bytes. release.yml and tests/plugin_archive.rs
# both run this script, so the test builds exactly what the release ships.
set -euo pipefail

if [ "$#" -ne 3 ]; then
    echo "usage: $0 <tree-ish> <allowlist> <out.zip>" >&2
    exit 2
fi
tree="$1"
list="$2"
out="$3"

specs="$(sed -e 's/\r$//' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' "$list" | grep -v '^\(#\|$\)' || true)"
if [ -z "$specs" ]; then
    echo "::error::$list lists no paths." >&2
    exit 1
fi

stage="$(mktemp -d)"
tmp_zip=""
trap 'rm -rf "$stage"; [ -z "$tmp_zip" ] || rm -f "$tmp_zip"' EXIT

printf '%s\n' "$specs" | tr '\n' '\0' \
    | xargs -0 git archive --format=tar "$tree" -- \
    | tar -x -C "$stage"

if [ -f "$stage/.claude-plugin/hooks.plugin.json" ]; then
    mv "$stage/.claude-plugin/hooks.plugin.json" "$stage/.claude-plugin/plugin.json"
fi

# Stamp every file with the commit time (or now for a bare tree), so the same
# commit gives the same bytes. BSD date takes -r, GNU date takes -d.
stamp="$(git log -1 --format=%ct "$tree" 2>/dev/null || true)"
stamp="${stamp:-$(date +%s)}"
touch_time="$(date -u -r "$stamp" +%Y%m%d%H%M.%S 2>/dev/null || date -u -d "@$stamp" +%Y%m%d%H%M.%S)"
find "$stage" -exec touch -t "$touch_time" {} +

mkdir -p "$(dirname "$out")"
out_abs="$(cd "$(dirname "$out")" && pwd)/$(basename "$out")"
tmp_zip="${out_abs}.tmp"
# zip adds to an archive that already exists, so never start from a stale one.
rm -f "$tmp_zip"
(cd "$stage" && find . -type f | sed 's|^\./||' | LC_ALL=C sort | zip -X -q "$tmp_zip" -@)
mv -f "$tmp_zip" "$out_abs"
