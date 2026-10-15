#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# check-plugin.sh: guards for the playbook plugin under plugins/playbook.
#
#   check-plugin.sh payload
#     Fails when the plugin directory holds anything but markdown and the
#     manifest. The marketplace serves this directory as it is, so a script, a
#     binary or a stray config file would ship to every user.
#
#   check-plugin.sh version <base-ref>
#     Fails when the plugin changed against <base-ref> without a version bump,
#     or when the version has no "## playbook <version>" section at the top of
#     CHANGELOG.md. Claude Code updates the plugin only when the version
#     changes, so a content change that keeps the version never reaches users.
#
#   check-plugin.sh mirror
#     Fails when metadata.breakingChanges in the manifest differs from the
#     plugin entries in breaking-changes.json (ADR 0035 decision 4.6). The
#     marketplace copies only the plugin folder, so the binary reads the
#     plugin's breaking changes from the manifest. No plugin entries and no
#     key is a match.
#
# Run from anywhere inside the repository.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLUGIN="plugins/playbook"
MANIFEST="$PLUGIN/.claude-plugin/plugin.json"
CHANGELOG="CHANGELOG.md"
RECORD="breaking-changes.json"
cd "$ROOT"

fail() { echo "FAIL $*"; exit 1; }

manifest_version() {
  jq -r '.version // empty' 2>/dev/null
}

check_semver() {
  printf '%s' "$1" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' \
    || fail "version '$1' is not MAJOR.MINOR.PATCH"
}

# True when $1 is strictly greater than $2 (both MAJOR.MINOR.PATCH).
version_gt() {
  [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)" = "$1" ]
}

payload() {
  [ -f "$MANIFEST" ] || fail "$MANIFEST is missing"
  jq -e '.name == "playbook"' "$MANIFEST" >/dev/null || fail "$MANIFEST must name the plugin playbook"
  check_semver "$(manifest_version < "$MANIFEST")" || exit 1

  # One flat folder each for agents, commands and output styles, and
  # markdown directly in a skill folder or in its references folder.
  local flat='^(agents|commands|output-styles)/[^/]+\.md$'
  local skill='^skills/[^/]+/(references/)?[^/]+\.md$'
  local bad=0 mode rel file count=0
  while IFS=$' \t' read -r mode _sha _stage file; do
    rel="${file#"$PLUGIN"/}"
    count=$((count + 1))
    if [ "$file" != "$MANIFEST" ] && ! [[ "$rel" =~ $flat ]] && ! [[ "$rel" =~ $skill ]]; then
      echo "FAIL not allowed in the plugin payload: $file"; bad=1
    elif [ "$mode" != "100644" ]; then
      echo "FAIL $file must be a plain file (mode $mode)"; bad=1
    fi
  done < <(git -c core.quotePath=false ls-files -s "$PLUGIN")
  [ "$bad" -eq 0 ] || exit 1
  echo "PASS the plugin payload holds $count files, all plain markdown or the manifest"
}

version() {
  local base="${1:?usage: check-plugin.sh version <base-ref>}"
  [ -f "$MANIFEST" ] || fail "$MANIFEST is missing"
  local head_v base_v
  head_v="$(manifest_version < "$MANIFEST")" || fail "$MANIFEST is not valid JSON"
  check_semver "$head_v"

  # The top changelog section must always match the manifest.
  local top
  top="$(sed -n 's/^## playbook \([0-9][0-9.]*\)$/\1/p' "$CHANGELOG" | head -1)"
  [ -n "$top" ] || fail "$CHANGELOG has no '## playbook <version>' section"
  [ "$top" = "$head_v" ] || fail "$CHANGELOG starts at playbook $top but $MANIFEST says $head_v"

  if ! git rev-parse --verify --quiet "$base^{commit}" >/dev/null; then
    fail "base ref '$base' is not available; fetch it first"
  fi
  if git diff --quiet "$base" -- "$PLUGIN"; then
    echo "PASS the plugin is unchanged against $base, version $head_v"
    return 0
  fi
  if git cat-file -e "$base:$MANIFEST" 2>/dev/null; then
    base_v="$(git show "$base:$MANIFEST" | manifest_version)" || fail "$MANIFEST at $base is not valid JSON"
    version_gt "$head_v" "$base_v" \
      || fail "the plugin changed but its version is $head_v (base has $base_v); bump the version in $MANIFEST"
  fi
  echo "PASS the plugin changed and its version is $head_v with a changelog section"
}

mirror() {
  [ -f "$MANIFEST" ] || fail "$MANIFEST is missing"
  [ -f "$RECORD" ] || fail "$RECORD is missing"
  local want got
  want="$(jq -cS '[.entries[] | select(.component == "plugin")]' "$RECORD")" \
    || fail "$RECORD is not valid JSON"
  got="$(jq -cS '.metadata.breakingChanges // []' "$MANIFEST")" \
    || fail "$MANIFEST is not valid JSON"
  if [ "$want" != "$got" ]; then
    echo "FAIL metadata.breakingChanges in $MANIFEST must list the plugin entries of $RECORD, in the same order"
    echo "  expected: $want"
    echo "  found:    $got"
    exit 1
  fi
  echo "PASS $MANIFEST mirrors the $(jq 'length' <<<"$want") plugin entries of $RECORD"
}

case "${1:-}" in
  payload) payload ;;
  version) version "${2:-}" ;;
  mirror) mirror ;;
  *) echo "usage: check-plugin.sh payload | version <base-ref> | mirror"; exit 2 ;;
esac
