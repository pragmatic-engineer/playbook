#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# aur-verify.test.sh: offline tests for the parts of aur-verify.sh that need
# no Arch container and no network. The build and install run in the release.
#
# Usage: aur-verify.test.sh
set -euo pipefail

cd "$(dirname "$0")/.."

VERIFY="$PWD/scripts/aur-verify.sh"
RENDER="$PWD/scripts/render-pkgbuild.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

pass=0
fail() { echo "FAIL $*" >&2; exit 1; }
ok() { pass=$((pass + 1)); }

hex_of() { printf '%s' "$1" | shasum -a 256 | cut -d' ' -f1; }

# A rendered PKGBUILD for 1.2.3 with known sums.
mkdir "$TMP/r"
{
  printf '%s  playbook-1.2.3-x86_64-unknown-linux-musl\n' "$(hex_of x)"
  printf '%s  playbook-1.2.3-aarch64-unknown-linux-musl\n' "$(hex_of a)"
} > "$TMP/r/SHA256SUMS"
"$RENDER" 1.2.3 "$TMP/r/SHA256SUMS" "$TMP/r/PKGBUILD"

# The aarch64 asset is the release URL and the aarch64 sum of the PKGBUILD.
got="$("$VERIFY" aarch64-asset "$TMP/r/PKGBUILD")"
want="https://github.com/pragmatic-engineer/playbook/releases/download/v1.2.3/playbook-1.2.3-aarch64-unknown-linux-musl $(hex_of a)"
[ "$got" = "$want" ] || fail "aarch64-asset gave '$got'"
ok

# Refusals: each exits 1 with a message.
expect_fail() {
  local msg="$1" out rc=0
  shift
  out="$("$VERIFY" "$@" 2>&1)" || rc=$?
  [ "$rc" -eq 1 ] || fail "expected exit 1, got $rc for: $* ($out)"
  grep -qF -- "$msg" <<<"$out" || fail "expected '$msg', got: $out"
  ok
}
expect_fail 'usage'
expect_fail 'usage' nonsense
expect_fail 'usage' aarch64-asset
expect_fail 'not a file' aarch64-asset "$TMP/none"
printf 'pkgname=x\n' > "$TMP/empty"
expect_fail 'no aarch64 source' aarch64-asset "$TMP/empty"
expect_fail 'is not a directory' srcinfo "$TMP/none"
expect_fail 'is not a directory' build "$TMP/none" 1.2.3
expect_fail 'bad version' build "$TMP/r" 1.2

# The pinned host keys are the ones aur.archlinux.org publishes on its home
# page (checked on 2026-10-22). A different key means a refresh from that page.
got="$(ssh-keygen -lf packaging/aur/known_hosts | awk '{ print $2 " " $NF }' | sort)"
want="$(printf '%s\n' \
  'SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4 (ED25519)' \
  'SHA256:uTa/0PndEgPZTf76e1DFqXKJEXKsn7m9ivhLQtzGOCI (ECDSA)' \
  'SHA256:5s5cIyReIfNNVGRFdDbe3hdYiI5OelHGpw2rOUud3Q8 (RSA)' | sort)"
[ "$got" = "$want" ] || fail "packaging/aur/known_hosts is not the published AUR host keys: $got"
ok

echo "OK aur-verify: $pass checks"
