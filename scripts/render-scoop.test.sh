#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# render-scoop.test.sh: offline tests for render-scoop.sh. No network, no scoop.
#
# Usage: render-scoop.test.sh
set -euo pipefail

cd "$(dirname "$0")/.."

command -v jq >/dev/null || { echo "FAIL jq is required" >&2; exit 1; }

SCRIPT="$PWD/scripts/render-scoop.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

pass=0
fail() { echo "FAIL $*" >&2; exit 1; }
ok() { pass=$((pass + 1)); }

hex_of() { printf '%s' "$1" | shasum -a 256 | cut -d' ' -f1; }
WIN_HASH="$(hex_of w)"

# new_case <name>: makes $TMP/<name>, sets CASE.
new_case() { CASE="$TMP/$1"; mkdir -p "$CASE"; }

# sums_full <version> <file>: a SHA256SUMS as the real release makes it.
sums_full() {
  local v="$1" f="$2" t
  {
    printf '%s  playbook-%s-aarch64-apple-darwin\n' "$(hex_of a)" "$v"
    printf '%s  playbook-%s-aarch64-unknown-linux-musl\n' "$(hex_of b)" "$v"
    printf '%s  playbook-%s-x86_64-apple-darwin\n' "$(hex_of c)" "$v"
    printf '%s  playbook-%s-x86_64-pc-windows-msvc.exe\n' "$WIN_HASH" "$v"
    printf '%s *playbook-%s-x86_64-unknown-linux-musl\n' "$(hex_of d)" "$v"
    for t in aarch64-apple-darwin aarch64-unknown-linux-musl x86_64-apple-darwin x86_64-unknown-linux-musl; do
      printf '%s  pbtop-%s-%s.tar.gz\n' "$(hex_of "p$t")" "$v" "$t"
      printf '%s  pbdoc-%s-%s.tar.gz\n' "$(hex_of "d$t")" "$v" "$t"
    done
    printf '%s  pbtop-%s-x86_64-pc-windows-msvc.tar.gz\n' "$(hex_of pw)" "$v"
    printf '%s  playbook_%s_amd64.deb\n' "$(hex_of deb)" "$v"
    printf '%s  playbook-%s-1.x86_64.rpm\n' "$(hex_of rpm)" "$v"
    printf '%s  release-notes.json\n' "$(hex_of n)"
  } > "$f"
}

# expect_fail <code> <message> <version> <sums-file>
expect_fail() {
  local code="$1" msg="$2" out rc=0
  out="$("$SCRIPT" "$3" "$4" "$CASE/out.json" 2>&1)" || rc=$?
  [ "$rc" -eq "$code" ] || fail "expected exit $code, got $rc ($msg): $out"
  grep -q -- "$msg" <<<"$out" || fail "message '$msg' not in: $out"
  [ ! -e "$CASE/out.json" ] || fail "output written on a refused run ($msg)"
  ok
}

# Case: a full-shape sums file renders the manifest for the Windows line only.
new_case full
sums_full 0.39.0 "$CASE/SHA256SUMS"
"$SCRIPT" 0.39.0 "$CASE/SHA256SUMS" "$CASE/out.json"
want_url="https://github.com/pragmatic-engineer/playbook/releases/download/v0.39.0/playbook-0.39.0-x86_64-pc-windows-msvc.exe#/playbook.exe"
[ "$(jq -r .version "$CASE/out.json")" = 0.39.0 ] || fail "version not set"
[ "$(jq -r '.architecture."64bit".hash' "$CASE/out.json")" = "$WIN_HASH" ] || fail "hash is not the Windows line"
[ "$(jq -r '.architecture."64bit".url' "$CASE/out.json")" = "$want_url" ] || fail "url mismatch"
[ "$(jq -r .bin "$CASE/out.json")" = playbook.exe ] || fail "bin not playbook.exe"
[ "$(jq -r .license "$CASE/out.json")" = Apache-2.0 ] || fail "license not Apache-2.0"
ok

# Case: the manifest has no install script, persist, autoupdate or alias.
for key in pre_install post_install installer uninstaller persist autoupdate checkver; do
  jq -e --arg k "$key" 'has($k) | not' "$CASE/out.json" >/dev/null || fail "manifest has $key"
done
ok

# Case: rendering twice gives the same bytes.
"$SCRIPT" 0.39.0 "$CASE/SHA256SUMS" "$CASE/out2.json"
cmp -s "$CASE/out.json" "$CASE/out2.json" || fail "second run differs"
ok

# Case: an existing output file is replaced.
printf 'stale' > "$CASE/out2.json"
"$SCRIPT" 0.39.0 "$CASE/SHA256SUMS" "$CASE/out2.json"
cmp -s "$CASE/out.json" "$CASE/out2.json" || fail "stale output kept"
ok

# Cases: boundary versions render.
for v in 0.0.0 999.999.999; do
  new_case "boundary-$v"
  sums_full "$v" "$CASE/SHA256SUMS"
  "$SCRIPT" "$v" "$CASE/SHA256SUMS" "$CASE/out.json"
  [ "$(jq -r .version "$CASE/out.json")" = "$v" ] || fail "boundary $v"
  ok
done

# Case: an empty sums file is refused.
new_case empty
: > "$CASE/SHA256SUMS"
expect_fail 2 "empty" 0.39.0 "$CASE/SHA256SUMS"

# Case: a sums file without the Windows line is refused.
new_case nowin
sums_full 0.39.0 "$CASE/SHA256SUMS"
grep -v 'windows-msvc.exe' "$CASE/SHA256SUMS" > "$CASE/s2" && mv "$CASE/s2" "$CASE/SHA256SUMS"
expect_fail 2 "no Windows line" 0.39.0 "$CASE/SHA256SUMS"

# Case: the Windows line of another version does not count.
new_case otherver
sums_full 0.38.0 "$CASE/SHA256SUMS"
expect_fail 2 "no Windows line" 0.39.0 "$CASE/SHA256SUMS"

# Cases: a bad hash is refused (63 chars, 65 chars, uppercase).
name="playbook-0.39.0-x86_64-pc-windows-msvc.exe"
for kind in short long upper; do
  new_case "hash-$kind"
  case "$kind" in
    short) h="${WIN_HASH%?}" ;;
    long) h="${WIN_HASH}0" ;;
    upper) h="$(tr 'a-f' 'A-F' <<<"$WIN_HASH")" ;;
  esac
  printf '%s  %s\n' "$h" "$name" > "$CASE/SHA256SUMS"
  expect_fail 2 "not 64 lowercase hex" 0.39.0 "$CASE/SHA256SUMS"
done

# Case: a duplicated Windows line is refused.
new_case dup
sums_full 0.39.0 "$CASE/SHA256SUMS"
printf '%s  %s\n' "$(hex_of z)" "$name" >> "$CASE/SHA256SUMS"
expect_fail 2 "more than one Windows line" 0.39.0 "$CASE/SHA256SUMS"

# Case: a missing sums file is refused.
new_case missing
expect_fail 2 "not a file" 0.39.0 "$CASE/none"

# Cases: a version that is not X.Y.Z is refused with exit 1.
new_case badver
sums_full 0.39.0 "$CASE/SHA256SUMS"
for v in "" v0.39.0 0.39 0.39.0-rc.1; do
  expect_fail 1 "bad version" "$v" "$CASE/SHA256SUMS"
done

# Case: a wrong argument count prints usage and exits 1.
rc=0
out="$("$SCRIPT" 0.39.0 2>&1)" || rc=$?
[ "$rc" -eq 1 ] || fail "usage exit: $rc"
grep -q usage <<<"$out" || fail "usage text: $out"
ok

echo "ok: $pass checks"
