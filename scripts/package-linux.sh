#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# package-linux.sh: build the .deb and .rpm packages of a release (ADR 0039).
#
# Usage:
#   package-linux.sh names <version> <target>
#       print the deb and rpm file names, one per line
#   package-linux.sh install-nfpm <dir>
#       download the pinned nfpm for this machine into <dir>, after checking
#       its SHA-256
#   package-linux.sh build <version> <target> <bindir> <outdir>
#       build both packages from <bindir>/playbook and <bindir>/pbtop, the
#       static musl binaries of <target>, with the nfpm on PATH. The shell
#       completions come from that playbook binary, so it must run here.
#
# Targets are the two Linux musl targets of the release. The names are part
# of the release contract: SHA256SUMS lists them, and the publish and verify
# jobs of release.yml and docs/guides/00-install.md use them.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# The pinned nfpm. Bump the version and both sums together, from the
# checksums.txt of the release at https://github.com/goreleaser/nfpm/releases.
NFPM_VERSION=2.48.0
NFPM_SHA256_X86_64=6d7f9a28b2f397934403950fcbba83f598b08d02edbe79a0815b3b4897341750
NFPM_SHA256_ARM64=a9a75213763783b7669719e505474c748c5ad7dc66591a195619deb50f85c2f4

fail() { echo "FAIL $*" >&2; exit 1; }

# arch_of <target>: sets DEB_ARCH and RPM_ARCH
arch_of() {
  case "$1" in
    x86_64-unknown-linux-musl) DEB_ARCH=amd64 RPM_ARCH=x86_64 ;;
    aarch64-unknown-linux-musl) DEB_ARCH=arm64 RPM_ARCH=aarch64 ;;
    *) fail "no package for target '$1'" ;;
  esac
}

check_version() {
  printf '%s' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || fail "'$1' is not MAJOR.MINOR.PATCH"
}

names() {
  local version="${1:-}" target="${2:-}"
  [ -n "$version" ] && [ -n "$target" ] || fail "usage: package-linux.sh names <version> <target>"
  check_version "$version"
  arch_of "$target"
  printf 'playbook_%s_%s.deb\n' "$version" "$DEB_ARCH"
  printf 'playbook-%s.%s.rpm\n' "$version" "$RPM_ARCH"
}

install_nfpm() {
  local dir="${1:-}" machine want url tmp
  [ -n "$dir" ] || fail "usage: package-linux.sh install-nfpm <dir>"
  [ "$(uname -s)" = Linux ] || fail "install-nfpm runs on Linux only"
  machine="$(uname -m)"
  case "$machine" in
    x86_64) machine=x86_64 want="$NFPM_SHA256_X86_64" ;;
    aarch64 | arm64) machine=arm64 want="$NFPM_SHA256_ARM64" ;;
    *) fail "no nfpm for $machine" ;;
  esac
  url="https://github.com/goreleaser/nfpm/releases/download/v${NFPM_VERSION}/nfpm_${NFPM_VERSION}_Linux_${machine}.tar.gz"
  tmp="$(mktemp -d)"
  curl -fsSL --retry 3 -o "$tmp/nfpm.tar.gz" "$url"
  echo "$want  $tmp/nfpm.tar.gz" | sha256sum -c - >/dev/null || fail "nfpm $NFPM_VERSION does not match its pinned SHA-256"
  mkdir -p "$dir"
  tar -xzf "$tmp/nfpm.tar.gz" -C "$dir" nfpm
  rm -rf "$tmp"
  "$dir/nfpm" --version | grep -q "$NFPM_VERSION" || fail "the nfpm in $dir is not $NFPM_VERSION"
}

build() {
  local version="${1:-}" target="${2:-}" bindir="${3:-}" outdir="${4:-}" f sh comps
  [ -n "$version" ] && [ -n "$target" ] && [ -d "$bindir" ] && [ -n "$outdir" ] \
    || fail "usage: package-linux.sh build <version> <target> <bindir> <outdir>"
  check_version "$version"
  arch_of "$target"
  for b in playbook pbtop; do
    [ -x "$bindir/$b" ] || fail "$bindir/$b is missing or not executable"
  done
  command -v nfpm >/dev/null || fail "nfpm is not on PATH; run install-nfpm first"
  mkdir -p "$outdir"
  # BIN_DIR is made absolute, because nfpm reads the config from the repo root.
  bindir="$(cd "$bindir" && pwd)"
  outdir="$(cd "$outdir" && pwd)"
  comps="$(mktemp -d)"
  for sh in bash zsh fish; do
    "$bindir/playbook" completions "$sh" > "$comps/$sh" || fail "playbook completions $sh failed"
    [ -s "$comps/$sh" ] || fail "playbook completions $sh printed nothing"
  done
  while IFS= read -r f; do
    (cd "$ROOT" && NFPM_ARCH="$DEB_ARCH" NFPM_VERSION="$version" BIN_DIR="$bindir" COMPLETIONS_DIR="$comps" \
      nfpm package --config scripts/nfpm.yaml --packager "${f##*.}" --target "$outdir/$f")
    [ -s "$outdir/$f" ] || fail "nfpm made no $f"
    echo "built $f"
  done < <(names "$version" "$target")
  rm -rf "$comps"
}

case "${1:-}" in
  names) shift; names "$@" ;;
  install-nfpm) shift; install_nfpm "$@" ;;
  build) shift; build "$@" ;;
  *) echo "usage: package-linux.sh <names|install-nfpm|build> ..." >&2; exit 2 ;;
esac
