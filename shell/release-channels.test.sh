#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# release-channels.test.sh: tests for render-formula.sh and pin-marketplace.sh.
#
# Run:  bash shell/release-channels.test.sh
set -u

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIX="$DIR/fixtures/release-channels"
PASS=0
FAIL=0

# check <name> <command...>: passes when the command succeeds.
check() {
  local name="$1"; shift
  if "$@"; then echo "PASS: $name"; PASS=$((PASS + 1))
  else echo "FAIL: $name"; FAIL=$((FAIL + 1)); fi
}
fails() { ! "$@" >/dev/null 2>&1; }
has() { grep -qF -- "$2" <<<"$1"; }
count_is() { [[ "$(grep -cF -- "$2" <<<"$1")" == "$3" ]]; }
jq_is() { [[ "$(jq -r "$2" <<<"$1")" == "$3" ]]; }

out="$(bash "$DIR/render-formula.sh" 9.8.7 "$FIX/SHA256SUMS")"
check "formula renders" test -n "$out"
check "no placeholder left" bash -c '! grep -q "@" <<<"$1"' _ "$out"
check "four url lines for the version" count_is "$out" 'download/v9.8.7/playbook-9.8.7-' 4
check "aarch64 darwin sha" has "$out" "sha256 \"$(printf '1%.0s' {1..64})\""
check "x86_64 darwin sha" has "$out" "sha256 \"$(printf '2%.0s' {1..64})\""
check "aarch64 linux sha" has "$out" "sha256 \"$(printf '3%.0s' {1..64})\""
check "x86_64 linux sha" has "$out" "sha256 \"$(printf '4%.0s' {1..64})\""
check "windows sha not used" bash -c '! grep -q 5555 <<<"$1"' _ "$out"
for pair in aarch64-apple-darwin:1 x86_64-apple-darwin:2 aarch64-unknown-linux-musl:3 x86_64-unknown-linux-musl:4; do
  check "url and sha adjacent for ${pair%%:*}" bash -c \
    'grep -A1 "${2}\"" <<<"$1" | grep -q "sha256 \"${3}${3}${3}${3}"' _ "$out" "${pair%%:*}" "${pair##*:}"
done
check "missing version in sums fails" fails bash "$DIR/render-formula.sh" 9.8.6 "$FIX/SHA256SUMS"
check "v-prefixed version rejected by the guard" bash -c \
  'bash "$1/render-formula.sh" v9.8.7 "$2" 2>&1 | grep -q "bad version"' _ "$DIR" "$FIX/SHA256SUMS"
check "quote in version rejected" fails bash "$DIR/render-formula.sh" '9.8.7"x' "$FIX/SHA256SUMS"

SHA="$(printf 'a%.0s' {1..64})"
SHA2="$(printf 'b%.0s' {1..64})"
URL="https://github.com/pragmatic-engineer/playbook/releases/download/v9.8.7/playbook-plugin-9.8.7.zip"
m="$(bash "$DIR/pin-marketplace.sh" 9.8.7 "$FIX/marketplace.json" "$SHA")"
pb='.plugins[]|select(.name=="playbook")'
check "playbook source is the archive" jq_is "$m" "$pb|.source.source" archive
check "playbook archive url names the version" jq_is "$m" "$pb|.source.url" "$URL"
check "playbook archive sha256 set" jq_is "$m" "$pb|.source.sha256" "$SHA"
check "playbook source has exactly three fields" jq_is "$m" "$pb|.source|keys|join(\",\")" "sha256,source,url"
check "no git ref left behind" jq_is "$m" "$pb|.source|has(\"ref\")" false
check "other playbook fields kept" jq_is "$m" "$pb|.category" workflow
check "other plugin untouched" jq_is "$m" '.plugins[]|select(.name=="other")|.source|tojson' '{"source":"url","url":"https://example.com/other.git"}'
m2="$(bash "$DIR/pin-marketplace.sh" 9.8.8 <(printf '%s' "$m") "$SHA2")"
check "repin replaces url" jq_is "$m2" "$pb|.source.url" "${URL//9.8.7/9.8.8}"
check "repin replaces sha256" jq_is "$m2" "$pb|.source.sha256" "$SHA2"
check "missing playbook entry fails" fails bash "$DIR/pin-marketplace.sh" 9.8.7 "$FIX/marketplace-empty.json" "$SHA"
check "missing sha256 fails" fails bash "$DIR/pin-marketplace.sh" 9.8.7 "$FIX/marketplace.json"
check "short sha256 fails" fails bash "$DIR/pin-marketplace.sh" 9.8.7 "$FIX/marketplace.json" abc123
check "uppercase sha256 fails" fails bash "$DIR/pin-marketplace.sh" 9.8.7 "$FIX/marketplace.json" "${SHA^^}"
check "v-prefixed version fails" fails bash "$DIR/pin-marketplace.sh" v9.8.7 "$FIX/marketplace.json" "$SHA"

echo "passed=$PASS failed=$FAIL"
[[ $FAIL -eq 0 ]]
