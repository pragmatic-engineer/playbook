#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# pin-marketplace.sh <version> <marketplace.json>: print the marketplace file
# with the playbook plugin source pinned to the release tag via `ref`.
set -euo pipefail

version="${1:?usage: pin-marketplace.sh <version> <marketplace.json>}"
file="${2:?usage: pin-marketplace.sh <version> <marketplace.json>}"

jq --arg ref "v${version}" '
  if any(.plugins[]; .name == "playbook") then
    (.plugins[] | select(.name == "playbook") | .source.ref) = $ref
  else error("no playbook plugin entry") end
' "$file"
