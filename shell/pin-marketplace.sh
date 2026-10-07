#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# pin-marketplace.sh <version> <marketplace.json> <sha256>: print the
# marketplace file with the playbook plugin source set to the release's
# trimmed plugin archive (needs Claude Code 2.1.224 or later).
set -euo pipefail

usage="usage: pin-marketplace.sh <version> <marketplace.json> <sha256>"
version="${1:?$usage}"
file="${2:?$usage}"
sha256="${3:?$usage}"

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+.][0-9A-Za-z.+-]+)?$ ]] || { echo "bad version: ${version}" >&2; exit 1; }
[[ "$sha256" =~ ^[0-9a-f]{64}$ ]] || { echo "bad sha256: ${sha256}" >&2; exit 1; }

url="https://github.com/pragmatic-engineer/playbook/releases/download/v${version}/playbook-plugin-${version}.zip"

jq --arg url "$url" --arg sha "$sha256" '
  if any(.plugins[]; .name == "playbook") then
    (.plugins[] | select(.name == "playbook") | .source) =
      {source: "archive", url: $url, sha256: $sha}
  else error("no playbook plugin entry") end
' "$file"
