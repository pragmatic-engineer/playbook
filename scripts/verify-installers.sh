#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# verify-installers.sh <version>: install the published release the way
# `cargo binstall` and mise users do, in a scratch environment, and check it
# (ADR 0039). Run from a checkout of the release tag, on Linux x86_64.
#
# For each tool it installs playbook, runs `playbook --version`, and compares
# the installed file with the SHA256SUMS line of the release and with its
# build attestation. cargo binstall may not compile, so a missing or renamed
# asset fails instead of falling back to a build.
#
# Both tools are pinned by version and SHA-256, and they run with an empty
# HOME. Nothing outside the scratch folders is written.
#
# Needs: curl, tar, sha256sum and gh (for the attestation check).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO="${GH_REPO:-pragmatic-engineer/playbook}"
TARGET=x86_64-unknown-linux-musl

# Bump a version and its sum together, from the release page of the tool.
BINSTALL_VERSION=1.26.0
BINSTALL_SHA256=dd7ff9451d8ba2e437a8d861889b630746f82f09d931df2d893b0c3f28656f87
MISE_VERSION=2026.9.12
MISE_SHA256=b4058dece685259910d3aba5782445996eea79dbdb3cf952a6eb81aadf0373ff

fail() { echo "::error::$*" >&2; exit 1; }

version="${1:-}"
printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || { echo "usage: verify-installers.sh <X.Y.Z>" >&2; exit 2; }
[ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] || fail "verify-installers.sh runs on Linux x86_64"

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/home" "$scratch/tools"
export HOME="$scratch/home"

fetch() { # <url> <sha256> <out>
  curl -fsSL --retry 3 -o "$3" "$1"
  echo "$2  $3" | sha256sum -c - >/dev/null || fail "$1 does not match its pinned SHA-256"
}

fetch "https://github.com/cargo-bins/cargo-binstall/releases/download/v${BINSTALL_VERSION}/cargo-binstall-x86_64-unknown-linux-musl.tgz" \
  "$BINSTALL_SHA256" "$scratch/binstall.tgz"
tar -xzf "$scratch/binstall.tgz" -C "$scratch/tools" cargo-binstall
fetch "https://github.com/jdx/mise/releases/download/v${MISE_VERSION}/mise-v${MISE_VERSION}-linux-x64.tar.gz" \
  "$MISE_SHA256" "$scratch/mise.tar.gz"
tar -xzf "$scratch/mise.tar.gz" -C "$scratch/tools" mise/bin/mise

# The checksum line of the release binary, from the release's own SHA256SUMS.
asset="playbook-$version-$TARGET"
curl -fsSL --retry 3 -o "$scratch/SHA256SUMS" "https://github.com/$REPO/releases/download/v$version/SHA256SUMS"
want="$(awk -v a="$asset" '$2 == a { print $1 }' "$scratch/SHA256SUMS")"
printf '%s' "$want" | grep -Eq '^[0-9a-f]{64}$' || fail "SHA256SUMS of v$version has no line for $asset"

# check <label> <installed file>
check() {
  local label="$1" bin="$2" out
  [ -x "$bin" ] || fail "$label installed no executable at $bin"
  out="$("$bin" --version)"
  [ "${out##* }" = "$version" ] || fail "$label: playbook --version says '$out', not $version"
  [ "$(sha256sum "$bin" | cut -d' ' -f1)" = "$want" ] \
    || fail "$label: the installed file does not match its SHA256SUMS line"
  gh attestation verify "$bin" --repo "$REPO" --source-ref "refs/heads/${DEFAULT_BRANCH:-main}" \
    --signer-workflow "$REPO/.github/workflows/release.yml" >/dev/null \
    || fail "$label: the installed file has no valid build attestation"
  echo "$label: $out, checksum and attestation match"
}

# cargo binstall reads the metadata in this checkout. The crates.io name
# `playbook` belongs to another project, so the crate comes from the manifest
# and never from the registry.
(cd "$ROOT" && "$scratch/tools/cargo-binstall" --manifest-path crates/cli/Cargo.toml \
  --no-confirm --strategies crate-meta-data --root "$scratch/binstall" playbook)
check "cargo binstall" "$scratch/binstall/bin/playbook"

export MISE_DATA_DIR="$scratch/mise/data" MISE_CONFIG_DIR="$scratch/mise/config" MISE_CACHE_DIR="$scratch/mise/cache"
mkdir -p "$MISE_DATA_DIR" "$MISE_CONFIG_DIR" "$MISE_CACHE_DIR"
(cd "$scratch" && "$scratch/tools/mise/bin/mise" install "github:$REPO@$version")
check "mise" "$MISE_DATA_DIR/installs/github-${REPO//\//-}/$version/playbook"
echo "cargo binstall and mise installed playbook $version from the release."
