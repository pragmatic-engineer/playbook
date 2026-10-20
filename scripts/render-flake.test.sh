#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# render-flake.test.sh: offline tests for render-flake.sh. No network, no Nix.
#
# Usage: render-flake.test.sh
set -euo pipefail

cd "$(dirname "$0")/.."

SCRIPT="$PWD/scripts/render-flake.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

pass=0
fail() { echo "FAIL $*" >&2; exit 1; }
ok() { pass=$((pass + 1)); }

# A distinct 64-digit hex hash for a one-letter seed.
hex_of() { printf '%s' "$1" | shasum -a 256 | cut -d' ' -f1; }
# The SRI form of the same hash, computed without the script under test.
sri_of() { printf 'sha256-%s' "$(hex_of "$1" | xxd -r -p | base64)"; }

T_LINUX_X=x86_64-unknown-linux-musl
T_LINUX_A=aarch64-unknown-linux-musl
T_DARWIN_X=x86_64-apple-darwin
T_DARWIN_A=aarch64-apple-darwin

# new_case <name>: makes $TMP/<name> with a flake.nix, sets CASE.
new_case() {
  CASE="$TMP/$1"
  mkdir -p "$CASE"
  cat > "$CASE/flake.nix" <<'NIX'
{
  outputs = { ... }: let
    # render-flake.sh rewrites the releases block, and nothing else.
    releases = {
      version = "0.0.0";
      x86_64-linux = "sha256-AAAA";
      aarch64-linux = "sha256-AAAA";
      x86_64-darwin = "sha256-AAAA";
      aarch64-darwin = "sha256-AAAA";
    };
  in releases;
}
NIX
}

# sums_full <version> <file>: a SHA256SUMS as the real release makes it, with
# the five playbook assets, the pbtop archives and release-notes.json.
sums_full() {
  local v="$1" f="$2"
  {
    printf '%s  playbook-%s-%s\n' "$(hex_of a)" "$v" "$T_DARWIN_A"
    printf '%s  playbook-%s-%s\n' "$(hex_of b)" "$v" "$T_LINUX_A"
    printf '%s  playbook-%s-%s\n' "$(hex_of c)" "$v" "$T_DARWIN_X"
    printf '%s  playbook-%s-x86_64-pc-windows-msvc.exe\n' "$(hex_of w)" "$v"
    printf '%s *playbook-%s-%s\n' "$(hex_of d)" "$v" "$T_LINUX_X"
    for t in "$T_DARWIN_A" "$T_LINUX_A" "$T_DARWIN_X" "$T_LINUX_X"; do
      printf '%s  pbtop-%s-%s.tar.gz\n' "$(hex_of "p$t")" "$v" "$t"
    done
    printf '%s  release-notes.json\n' "$(hex_of n)"
  } > "$f"
}

# expect_fail <message> <args...>: the script exits 1 and prints <message>.
expect_fail() {
  local msg="$1" out rc=0
  shift
  out="$("$SCRIPT" "$@" 2>&1)" || rc=$?
  [ "$rc" -eq 1 ] || fail "expected exit 1, got $rc for: $* ($out)"
  grep -qF -- "$msg" <<<"$out" || fail "expected message '$msg', got: $out"
  ok
}

# 1. Hex to SRI, exact vector (sha256 of the empty string).
got="$("$SCRIPT" sri e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855)"
[ "$got" = 'sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=' ] || fail "sri vector: $got"
ok

# 2 and 6a. A full-shape SHA256SUMS: only the four Nix hashes are used, each in
# its own slot, with both the "  " and " *" separators.
new_case full
sums_full 1.2.3 "$CASE/SHA256SUMS"
"$SCRIPT" 1.2.3 "$CASE/SHA256SUMS" "$CASE/flake.nix"
grep -qF 'version = "1.2.3";' "$CASE/flake.nix" || fail "version not rendered"
grep -qF "x86_64-linux = \"$(sri_of d)\";" "$CASE/flake.nix" || fail "x86_64-linux hash"
grep -qF "aarch64-linux = \"$(sri_of b)\";" "$CASE/flake.nix" || fail "aarch64-linux hash"
grep -qF "x86_64-darwin = \"$(sri_of c)\";" "$CASE/flake.nix" || fail "x86_64-darwin hash"
grep -qF "aarch64-darwin = \"$(sri_of a)\";" "$CASE/flake.nix" || fail "aarch64-darwin hash"
[ "$(grep -c 'sha256-' "$CASE/flake.nix")" -eq 4 ] || fail "expected 4 hashes"
grep -qF '# render-flake.sh rewrites' "$CASE/flake.nix" || fail "the rest of the file changed"
ok

# 9. Idempotent: a second run leaves the file byte-identical.
cp "$CASE/flake.nix" "$CASE/first.nix"
"$SCRIPT" 1.2.3 "$CASE/SHA256SUMS" "$CASE/flake.nix"
cmp -s "$CASE/first.nix" "$CASE/flake.nix" || fail "second run changed the file"
ok

# 3. Empty file.
new_case empty
: > "$CASE/SHA256SUMS"
expect_fail 'no sums' 1.2.3 "$CASE/SHA256SUMS" "$CASE/flake.nix"

# 4. A missing system, and the flake is left untouched.
new_case missing
sums_full 1.2.3 "$CASE/full"
grep -v "$T_DARWIN_A" "$CASE/full" > "$CASE/SHA256SUMS"
cp "$CASE/flake.nix" "$CASE/before.nix"
expect_fail "missing system: $T_DARWIN_A" 1.2.3 "$CASE/SHA256SUMS" "$CASE/flake.nix"
cmp -s "$CASE/before.nix" "$CASE/flake.nix" || fail "a refused run changed the flake"
ok

# 5. A duplicate target.
new_case dup
sums_full 1.2.3 "$CASE/SHA256SUMS"
printf '%s  playbook-1.2.3-%s\n' "$(hex_of z)" "$T_LINUX_X" >> "$CASE/SHA256SUMS"
expect_fail 'duplicate target' 1.2.3 "$CASE/SHA256SUMS" "$CASE/flake.nix"

# 6. A selected line without a hash.
new_case nohash
sums_full 1.2.3 "$CASE/full"
grep -v "$T_LINUX_X" "$CASE/full" | grep -v pbtop > "$CASE/SHA256SUMS"
printf 'playbook-1.2.3-%s\n' "$T_LINUX_X" >> "$CASE/SHA256SUMS"
expect_fail 'no hash' 1.2.3 "$CASE/SHA256SUMS" "$CASE/flake.nix"

# 7. A hash of the wrong length.
new_case short
sums_full 1.2.3 "$CASE/full"
sed "s/^[0-9a-f]\{64\}  playbook-1.2.3-$T_LINUX_X\$/abcd  playbook-1.2.3-$T_LINUX_X/" \
  "$CASE/full" | sed "s/^[0-9a-f]\{64\} \*playbook-1.2.3-$T_LINUX_X\$/abcd  playbook-1.2.3-$T_LINUX_X/" > "$CASE/SHA256SUMS"
expect_fail 'bad hash length' 1.2.3 "$CASE/SHA256SUMS" "$CASE/flake.nix"

# 8. Bad versions, each its own test.
new_case ver
sums_full 1.2.3 "$CASE/SHA256SUMS"
expect_fail 'bad version' 1.2 "$CASE/SHA256SUMS" "$CASE/flake.nix"
expect_fail 'bad version' v1.2.3 "$CASE/SHA256SUMS" "$CASE/flake.nix"

# A flake without the releases block is refused.
new_case noblock
sums_full 1.2.3 "$CASE/SHA256SUMS"
echo '{ }' > "$CASE/flake.nix"
expect_fail 'no releases block' 1.2.3 "$CASE/SHA256SUMS" "$CASE/flake.nix"

echo "OK render-flake: $pass checks"
