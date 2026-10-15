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
#   check-plugin.sh version
#     Fails unless the plugin version in plugin.json equals the workspace
#     version in Cargo.toml, and its metadata.minBinaryVersion is the X.Y.0
#     of that minor line. The binary and the plugin ship as one release line
#     (ADR 0038), and scripts/bump-version.sh sets all three.
#
#   check-plugin.sh cargo-version
#     Prints the workspace version in Cargo.toml, the one reader of it that
#     the other scripts use.
#
# Run from anywhere inside the repository.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PLUGIN="plugins/playbook"
MANIFEST="$PLUGIN/.claude-plugin/plugin.json"
cd "$ROOT"

fail() { echo "FAIL $*"; exit 1; }

manifest_version() {
  jq -r '.version // empty' 2>/dev/null
}

check_semver() {
  printf '%s' "$1" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' \
    || fail "version '$1' is not MAJOR.MINOR.PATCH"
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

cargo_version() {
  local v
  v="$(awk '/^\[workspace.package\]/ { on = 1; next } /^\[/ { on = 0 }
    on && /^version = / { gsub(/"/, "", $3); print $3; exit }' Cargo.toml)"
  [ -n "$v" ] || fail "Cargo.toml has no [workspace.package] version"
  check_semver "$v"
  echo "$v"
}

version() {
  [ -f "$MANIFEST" ] || fail "$MANIFEST is missing"
  local plugin_v min_v cargo_v floor
  plugin_v="$(manifest_version < "$MANIFEST")" || fail "$MANIFEST is not valid JSON"
  check_semver "$plugin_v"
  min_v="$(jq -r '.metadata.minBinaryVersion // empty' "$MANIFEST")"
  cargo_v="$(cargo_version)" || { echo "$cargo_v"; exit 1; }
  floor="${cargo_v%.*}.0"
  if [ "$plugin_v" != "$cargo_v" ] || [ "$min_v" != "$floor" ]; then
    fail "Cargo.toml says $cargo_v, $MANIFEST says version $plugin_v and minBinaryVersion ${min_v:-none}. The plugin version must be $cargo_v and minBinaryVersion $floor; run scripts/bump-version.sh <version>."
  fi
  echo "PASS the binary and the plugin are both at $cargo_v, for binaries from $floor"
}

case "${1:-}" in
  payload) payload ;;
  version) version ;;
  cargo-version) cargo_version ;;
  *) echo "usage: check-plugin.sh payload | version | cargo-version"; exit 2 ;;
esac
