#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# check-intents.sh: the contract between this repo and the playbook binary.
#
# plugins/intents.json lists the intents the skills, commands and agents rely on, each
# named like a tool map entry (for example `pr-reply`). The binary prints its
# own list with `playbook tools map --json`. This script:
#   1. fails when intents.json is not valid: sorted, unique ids in kebab case
#      with a boolean `required` field;
#   2. when a playbook binary is given, warns about each listed intent the
#      binary does not offer, and fails only for an intent marked required.
#
# Usage: [INTENTS_STRICT=1] check-intents.sh [path-to-playbook-binary]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FILE="$ROOT/plugins/intents.json"
fail=0

jq -e 'type == "array" and length > 0' "$FILE" >/dev/null || { echo "FAIL intents.json must be a non-empty array"; exit 1; }
if ! jq -e 'all(.[]; (.intent | type == "string" and test("^[a-z][a-z0-9]*(-[a-z0-9]+)*$")) and (.required | type == "boolean"))' "$FILE" >/dev/null; then
  echo "FAIL every entry needs a kebab-case intent and a boolean required"; exit 1
fi
if [ "$(jq -r '[.[].intent] | . == (sort | unique)' "$FILE")" != "true" ]; then
  echo "FAIL intents must be sorted and unique"; exit 1
fi
echo "PASS intents.json lists $(jq length "$FILE") intents"

bin="${1:-}"
if [ -z "$bin" ]; then
  echo "SKIP no playbook binary given, offered intents not compared"
  exit 0
fi
if ! map="$("$bin" tools map --json 2>/dev/null)" || ! printf '%s' "$map" | jq -e 'type == "array"' >/dev/null 2>&1; then
  # Against the binary built from this repository the map must exist, so
  # INTENTS_STRICT=1 turns the missing map into a failure.
  if [ "${INTENTS_STRICT:-}" = "1" ]; then
    echo "FAIL $bin gave no tools map"; exit 1
  fi
  echo "WARN $("$bin" --version 2>&1 | head -1) has no tools map yet, offered intents not compared"
  exit 0
fi
offered="$(printf '%s' "$map" | jq -r '.[].intent' | sort -u)"
while IFS=$'\t' read -r intent required; do
  if printf '%s\n' "$offered" | grep -qx "$intent"; then
    continue
  fi
  if [ "$required" = "true" ]; then
    echo "FAIL required intent $intent is not in the tool map"; fail=1
  else
    echo "WARN intent $intent is not in the tool map"
  fi
done < <(jq -r '.[] | [.intent, (.required | tostring)] | @tsv' "$FILE")
exit "$fail"
