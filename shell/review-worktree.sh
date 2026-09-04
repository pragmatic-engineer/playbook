#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# review-worktree.sh: manage git review worktrees
# Usage:
#   bash review-worktree.sh setup <pr> <head_sha>
#   bash review-worktree.sh teardown <path>

set -u

_die() {
  printf '%s\n' "$*" >&2
  exit 1
}

# Sweep stale linked worktrees. A worktree is stale if:
#   - it has no lock file (unlocked leftover), OR
#   - the lock file exists but ts is missing or older than TTL
# Fresh = lock file present with parseable ts within TTL (pid not checked; subshells die).
_sweep_stale_worktrees() {
  local ROOT="$1"
  local now ttl
  now=$(date +%s)
  ttl="${REVIEW_WT_TTL_SECONDS:-86400}"

  [[ -d "${ROOT}/worktrees" ]] || return 0

  local meta_dir gitdir_file gitdir_path wt_path lock_file lock_reason ts stale

  for meta_dir in "${ROOT}/worktrees"/*/; do
    [[ -d "$meta_dir" ]] || continue
    gitdir_file="${meta_dir}gitdir"
    [[ -f "$gitdir_file" ]] || continue

    gitdir_path="$(cat "$gitdir_file")"
    # gitdir_path is like /path/to/wt/.git; derive the wt directory
    wt_path="$(dirname "$gitdir_path")"

    # Only manage worktrees we created under review-worktrees/
    [[ "$wt_path" == "${ROOT}/review-worktrees/"* ]] || continue

    lock_file="${meta_dir}locked"

    stale=false
    if [[ ! -f "$lock_file" ]]; then
      # Not locked → stale unlocked leftover
      stale=true
    else
      lock_reason="$(cat "$lock_file")"
      ts="$(printf '%s' "$lock_reason" | grep -oE 'ts=[0-9]+' | cut -d= -f2)"
      if [[ -z "$ts" ]]; then
        # No parseable timestamp → treat as stale
        stale=true
      elif (( now - ts > ttl )); then
        stale=true
      fi
    fi

    if $stale; then
      git worktree remove -f -f "$wt_path" 2>/dev/null || true
    fi
  done
  # Prune orphaned admin entries once after the sweep, not per-removal
  git worktree prune
}

cmd_setup() {
  local pr="$1"
  local head_sha="$2"
  local short_sha="${head_sha:0:7}"

  # Absolute path to git common dir (handles both normal repos and worktrees)
  local ROOT
  ROOT="$(git rev-parse --path-format=absolute --git-common-dir)" \
    || _die "not in a git repo"

  local DIR="${ROOT}/review-worktrees/${pr}-${short_sha}"

  # Sweep stale linked worktrees
  _sweep_stale_worktrees "$ROOT"

  # Fetch PR head ref: resolve URL via gh when GH_FETCH_URL is not set
  local fetch_url="${GH_FETCH_URL:-}"
  if [[ -z "$fetch_url" ]]; then
    fetch_url="$(gh repo view --json url -q .url)" \
      || _die "failed to resolve repo URL via gh (not authenticated or not a GitHub repo)"
  fi
  git fetch "$fetch_url" "refs/pull/${pr}/head" \
    || _die "failed to fetch refs/pull/${pr}/head"

  # Warn (not abort) when origin has moved past the requested head_sha since
  # it was resolved, so a caller re-running the review notices before it burns tokens on stale state.
  local origin_sha
  origin_sha="$(git rev-parse FETCH_HEAD)" || _die "failed to resolve FETCH_HEAD after fetch"
  if [[ "$origin_sha" != "$head_sha" ]]; then
    printf 'warning: PR %s has moved since head_sha was resolved (requested %s, origin now at %s); re-run to review the latest commits\n' \
      "$pr" "$short_sha" "${origin_sha:0:7}" >&2
  fi

  # Assert SHA exists after fetch
  git cat-file -e "${head_sha}^{commit}" \
    || _die "head ${head_sha} not found after fetch (force-push?); re-run"

  # Check for live-locked worktree at the exact target path
  if [[ -d "$DIR" ]]; then
    if git worktree list --porcelain | grep -A5 "^worktree ${DIR}$" | grep -q "^locked"; then
      _die "review already in progress for PR ${pr}"
    fi
  fi

  # Ensure parent directory exists
  mkdir -p "$(dirname "$DIR")"

  # Add detached worktree (redirect stdout so "HEAD is now at..." doesn't leak into $())
  git worktree add --detach "$DIR" "$head_sha" >&2 \
    || _die "failed to add worktree at $DIR"

  # Lock it with our traceable reason
  git worktree lock --reason "review pr=${pr} pid=$$ ts=$(date +%s)" "$DIR" \
    || _die "failed to lock worktree at $DIR"

  printf '%s\n' "$DIR"
}

cmd_teardown() {
  local path="$1"
  git worktree unlock "$path" 2>/dev/null || true
  git worktree remove -f -f "$path" 2>/dev/null || true
  git worktree prune
  return 0
}

case "${1:-}" in
  setup)
    [[ $# -ge 3 ]] || _die "usage: $0 setup <pr> <head_sha>"
    cmd_setup "$2" "$3"
    ;;
  teardown)
    [[ $# -ge 2 ]] || _die "usage: $0 teardown <path>"
    cmd_teardown "$2"
    ;;
  *)
    _die "usage: $0 {setup <pr> <head_sha> | teardown <path>}"
    ;;
esac
