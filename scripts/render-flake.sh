#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# render-flake.sh: sets the version and the four binary hashes of flake.nix
# from the SHA256SUMS of a release.
#
#   render-flake.sh <version> <SHA256SUMS> <flake.nix>
#     Rewrites the `releases = { ... };` block of <flake.nix> in place and
#     nothing else. It reads only the lines for playbook-<version>-<target>
#     of the four Nix targets and ignores every other line. A file name may
#     follow the hash after two spaces or after " *". It refuses, with exit 1
#     and a message, an empty file, a missing or duplicate target, a line
#     without a hash, a hash that is not 64 hex digits, a version that is not
#     X.Y.Z, and a flake without the block. A refused run leaves the flake
#     as it was.
#
#   render-flake.sh sri <hex-sha256>
#     Prints the SRI form, sha256-<base64>.
set -euo pipefail

fail() { echo "FAIL $*" >&2; exit 1; }

# The Nix system and the release target of each binary.
SYSTEMS="x86_64-linux aarch64-linux x86_64-darwin aarch64-darwin"
target_of() {
  case "$1" in
    x86_64-linux) echo x86_64-unknown-linux-musl ;;
    aarch64-linux) echo aarch64-unknown-linux-musl ;;
    x86_64-darwin) echo x86_64-apple-darwin ;;
    aarch64-darwin) echo aarch64-apple-darwin ;;
  esac
}

sri() {
  [[ "$1" =~ ^[0-9a-f]{64}$ ]] || fail "bad hash length: '$1' is not 64 lowercase hex digits"
  printf 'sha256-%s' "$(printf '%s' "$1" | xxd -r -p | base64)"
}

if [ "${1:-}" = sri ]; then
  [ $# -eq 2 ] || fail "usage: render-flake.sh sri <hex-sha256>"
  sri "$2"
  echo
  exit 0
fi

[ $# -eq 3 ] || fail "usage: render-flake.sh <version> <SHA256SUMS> <flake.nix>"
version="$1" sums="$2" flake="$3"

[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "bad version: '$version' is not X.Y.Z"
[ -f "$sums" ] || fail "no sums: $sums is not a file"
[ -s "$sums" ] || fail "no sums: $sums is empty"
[ -f "$flake" ] || fail "no flake: $flake is not a file"
grep -q '^ *releases = {$' "$flake" || fail "no releases block in $flake"

lines=""
for system in $SYSTEMS; do
  target="$(target_of "$system")"
  name="playbook-$version-$target"
  # Lines whose last field is exactly the name, after "  " or " *".
  matches="$(awk -v n="$name" '{ f = $NF; sub(/^\*/, "", f); if (f == n || $0 == n) print }' "$sums")"
  [ -n "$matches" ] || fail "missing system: $target"
  [ "$(wc -l <<<"$matches" | tr -d ' ')" -eq 1 ] || fail "duplicate target: $target"
  hash="$(awk '{ print $1 }' <<<"$matches")"
  [ "$hash" != "$name" ] || fail "no hash on the line for $name"
  lines+="        $system = \"$(sri "$hash")\";"$'\n'
done

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
BODY="$lines" awk -v v="$version" '
  /^ *releases = \{$/ { print; printf "        version = \"%s\";\n%s", v, ENVIRON["BODY"]; skip = 1; next }
  skip && /^ *\};$/ { skip = 0 }
  !skip { print }
  END { if (skip) exit 1 }
' "$flake" > "$tmp" || fail "no end of the releases block in $flake"
cat "$tmp" > "$flake"
