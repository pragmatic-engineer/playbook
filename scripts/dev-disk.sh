#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# dev-disk.sh: keeps cargo target folders small while many worktrees of this
# repo build. A development helper for contributors and their agents. It is
# not part of the playbook binary or the plugin, and no release ships it.
#
# Usage:
#   eval "$(scripts/dev-disk.sh env)"      lean build settings and a private
#                                          target folder for this worktree
#   scripts/dev-disk.sh clean [--dry-run]  remove idle target folders of the
#                                          linked worktrees of this repo
#   scripts/dev-disk.sh warn               warn when free space is low
#
# Settings (environment variables):
#   DEV_DISK_WARN_FREE_GB  warn below this many GB free (default 15)
#   DEV_DISK_IDLE_MINUTES  a target folder must be unchanged this long to be
#                          removed (default 30)
set -euo pipefail

WARN_FREE_GB="${DEV_DISK_WARN_FREE_GB:-15}"
IDLE_MINUTES="${DEV_DISK_IDLE_MINUTES:-30}"
KIB_PER_GIB=1048576
TARGET_NAME="target"
HASH_LENGTH=8

# Prints a warning on stderr when the volume of this worktree has less than
# WARN_FREE_GB free.
warn_low_space() {
  local top free_kib
  top="$(git rev-parse --show-toplevel)"
  free_kib="$(df -Pk "$top" | awk 'NR == 2 { print $4 }')"
  if [ "$((free_kib / KIB_PER_GIB))" -lt "$WARN_FREE_GB" ]; then
    echo "dev-disk: only $((free_kib / KIB_PER_GIB)) GB free, below DEV_DISK_WARN_FREE_GB ($WARN_FREE_GB). Run scripts/dev-disk.sh clean, or cargo clean in finished worktrees." >&2
  fi
}

# Prints the export line for a lean build in a private target folder next to
# this worktree. Incremental data and debug info are the two big parts of a
# target folder; a panic keeps its file and line without debug info.
print_env() {
  local top hash dir
  top="$(git rev-parse --show-toplevel)"
  hash="$(printf '%s' "$top" | shasum | cut -c1-"$HASH_LENGTH")"
  dir="$(dirname "$top")/cargo-$(basename "$top")-$hash"
  warn_low_space
  printf "export CARGO_TARGET_DIR='%s' CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0\n" "$dir"
}

# Succeeds when a cargo build holds a lock under the target folder $1.
build_running() {
  local lock
  while IFS= read -r lock; do
    perl -e 'use Fcntl ":flock"; open(my $f, "<", $ARGV[0]) or exit 0; exit(flock($f, LOCK_EX | LOCK_NB) ? 0 : 1)' "$lock" || return 0
  done < <(find "$1" -maxdepth 3 -name .cargo-lock -type f 2>/dev/null)
  return 1
}

# Succeeds when a running process names the folder $1 on its command line.
named_by_a_process() {
  ps -axww -o args= | grep -F -q -- "$1"
}

# Removes the target folder of each linked worktree of this repo that is a
# cargo target folder, is not a link, has not changed for IDLE_MINUTES, has no
# build holding its lock and is named by no running process. The main
# worktree is never touched.
clean_targets() {
  local dry_run="$1" main path target
  main="$(git worktree list --porcelain | awk '/^worktree / { print substr($0, 10); exit }')"
  while IFS= read -r path; do
    [ "$path" = "$main" ] && continue
    target="$path/$TARGET_NAME"
    [ -d "$target" ] || continue
    if [ -L "$target" ] || [ ! -f "$target/CACHEDIR.TAG" ]; then
      echo "kept $target: not a cargo target folder"
    elif [ -n "$(find "$target" -mmin "-$IDLE_MINUTES" -print -quit)" ]; then
      echo "kept $target: changed in the last $IDLE_MINUTES minutes"
    elif build_running "$target"; then
      echo "kept $target: a cargo build holds its lock"
    elif named_by_a_process "$target"; then
      echo "kept $target: a running process names it"
    elif [ "$dry_run" = 1 ]; then
      echo "would remove $target ($(du -sh "$target" | cut -f1))"
    else
      echo "removing $target ($(du -sh "$target" | cut -f1))"
      rm -rf -- "$target"
    fi
  done < <(git worktree list --porcelain | awk '/^worktree / { print substr($0, 10) }')
}

case "${1:-}" in
  env) print_env ;;
  warn) warn_low_space ;;
  clean)
    case "${2:-}" in
      "") clean_targets 0 ;;
      --dry-run) clean_targets 1 ;;
      *) echo "usage: dev-disk.sh clean [--dry-run]" >&2; exit 2 ;;
    esac
    ;;
  *) echo "usage: dev-disk.sh env | clean [--dry-run] | warn" >&2; exit 2 ;;
esac
