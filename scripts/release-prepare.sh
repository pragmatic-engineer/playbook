#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# release-prepare.sh <version>: the changes of a release pull request
# (ADR 0039). It runs scripts/bump-version.sh, then adds the section
# "## playbook <version>" to the top of CHANGELOG.md with the notes that
# scripts/release-notes.sh builds from the commits since the last v tag
# (ADR 0038 decision 8). The notes keep their sections one heading level
# lower, and the compare link is left out.
#
# Running it again for the same version finishes what is missing: the bump
# runs only when the files are below the version, and the section is added
# only when it is not there.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

fail() { echo "FAIL $*" >&2; exit 1; }

version="${1:-}"
[ -n "$version" ] || { echo "usage: release-prepare.sh <version>" >&2; exit 2; }

current="$(jq -r '.version' plugins/playbook/.claude-plugin/plugin.json)"
if [ "$current" = "$version" ] && grep -qx "## playbook $version" CHANGELOG.md; then
  echo "The checkout already carries $version and its changelog section."
  exit 0
fi

# A run that bumped and then failed left the version in place. It only needs
# the changelog section.
if [ "$current" != "$version" ]; then
  scripts/bump-version.sh "$version"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
scripts/release-notes.sh json playbook "$version" > "$tmp/notes.json"
{
  echo "## playbook $version"
  echo
  # One level lower, so the sections sit under the release heading.
  scripts/release-notes.sh markdown "$tmp/notes.json" \
    | grep -v '^\*\*Full changelog\*\*' \
    | sed -E 's/^## /### /'
} > "$tmp/section.md"

# The new section goes before the first release section, after the header.
awk -v section="$tmp/section.md" '
  !done && /^## playbook / {
    while ((getline line < section) > 0) print line
    done = 1
  }
  { print }
  END { if (!done) { print ""; while ((getline line < section) > 0) print line } }
' CHANGELOG.md > "$tmp/CHANGELOG.md" || fail "could not write CHANGELOG.md"
cat "$tmp/CHANGELOG.md" > CHANGELOG.md

echo "Added the changelog section for $version."
