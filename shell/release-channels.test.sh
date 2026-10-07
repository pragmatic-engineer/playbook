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

m="$(bash "$DIR/pin-marketplace.sh" 9.8.7 "$FIX/marketplace.json")"
check "playbook ref pinned" jq_is "$m" '.plugins[]|select(.name=="playbook")|.source.ref' v9.8.7
check "playbook url kept" jq_is "$m" '.plugins[]|select(.name=="playbook")|.source.url' https://github.com/pragmatic-engineer/playbook.git
check "other plugin untouched" jq_is "$m" '.plugins[]|select(.name=="other")|.source|tojson' '{"source":"url","url":"https://example.com/other.git"}'
m2="$(bash "$DIR/pin-marketplace.sh" 9.8.8 <(printf '%s' "$m"))"
check "repin replaces ref" jq_is "$m2" '.plugins[]|select(.name=="playbook")|.source.ref' v9.8.8
check "missing playbook entry fails" fails bash "$DIR/pin-marketplace.sh" 9.8.7 "$FIX/marketplace-empty.json"

echo "passed=$PASS failed=$FAIL"
[[ $FAIL -eq 0 ]]
