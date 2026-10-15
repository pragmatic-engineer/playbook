#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# bump-version.sh <version>: sets the version of a playbook release (ADR 0038).
# The binary and the plugin share one version line, so it writes the same
# MAJOR.MINOR.PATCH to:
#   - the workspace version in Cargo.toml, and the playbook and
#     playbook-code entries in Cargo.lock;
#   - version and metadata.minBinaryVersion in the plugin's plugin.json;
#   - the PLAYBOOK_REF pins in README.md and docs/guides/00-install.md.
# The version must be above the current one, since Claude Code updates the
# plugin only when its version rises. Run it in the release pull request,
# then push the signed tag vX.Y.Z after it merges.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MANIFEST="plugins/playbook/.claude-plugin/plugin.json"
PINS=(README.md docs/guides/00-install.md)
cd "$ROOT"

fail() { echo "FAIL $*" >&2; exit 1; }

new="${1:-}"
[ -n "$new" ] || { echo "usage: bump-version.sh <version>" >&2; exit 2; }
printf '%s' "$new" | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' \
  || fail "version '$new' is not MAJOR.MINOR.PATCH"

# True when $1 is strictly greater than $2 (both MAJOR.MINOR.PATCH).
version_gt() {
  [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)" = "$1" ]
}

cargo_now="$(awk '/^\[workspace.package\]/ { on = 1; next } /^\[/ { on = 0 }
  on && /^version = / { gsub(/"/, "", $3); print $3; exit }' Cargo.toml)"
plugin_now="$(jq -r '.version // empty' "$MANIFEST")"
[ -n "$cargo_now" ] || fail "Cargo.toml has no [workspace.package] version"
version_gt "$new" "$cargo_now" || fail "$new is not above the current version $cargo_now"
# The plugin version never goes down. It can already equal the new version
# when the plugin line ran ahead of the binary, as before 0.32.0.
[ "$new" = "$plugin_now" ] || version_gt "$new" "$plugin_now" \
  || fail "$new is below the plugin version $plugin_now"

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

awk -v v="$new" '/^\[workspace.package\]/ { on = 1 } /^\[/ && !/^\[workspace.package\]/ { on = 0 }
  on && /^version = / { print "version = \"" v "\""; on = 0; next } { print }' Cargo.toml > "$tmp"
cat "$tmp" > Cargo.toml

awk -v v="$new" '/^name = "(playbook|playbook-code)"$/ { hit = 1; print; next }
  hit && /^version = / { print "version = \"" v "\""; hit = 0; next } { hit = 0; print }' Cargo.lock > "$tmp"
cat "$tmp" > Cargo.lock

jq --arg v "$new" '.version = $v | .metadata.minBinaryVersion = $v' "$MANIFEST" > "$tmp"
cat "$tmp" > "$MANIFEST"

for f in "${PINS[@]}"; do
  sed -E "s/PLAYBOOK_REF=v[0-9]+\.[0-9]+\.[0-9]+/PLAYBOOK_REF=v$new/g" "$f" > "$tmp"
  cat "$tmp" > "$f"
done

echo "Set the version to $new in Cargo.toml, Cargo.lock, $MANIFEST and ${PINS[*]}."
