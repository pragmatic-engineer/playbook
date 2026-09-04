#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# review-worktree.test.sh: self-contained smoke tests for shell/review-worktree.sh
#
# Run:  bash shell/review-worktree.test.sh
# Exit: 0 if all 10 scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HELPER="${SCRIPT_DIR}/review-worktree.sh"
PASS=0
FAIL=0

# Globals set by setup_repo (avoids subshell / variable-propagation issues)
REPO_DIR=""
BARE_DIR=""
HEAD_SHA=""

# ---------------------------------------------------------------------------
# Minimal test harness
# ---------------------------------------------------------------------------
run_scenario() {
  local name="$1"
  local fn="$2"
  if "$fn" 2>&1; then
    echo "PASS: $name"
    (( PASS++ )) || true
  else
    echo "FAIL: $name"
    (( FAIL++ )) || true
  fi
}

# ---------------------------------------------------------------------------
# Shared repo bootstrap: sets REPO_DIR, BARE_DIR, HEAD_SHA, GH_FETCH_URL
# Must be called directly (NOT inside $(...)) so exports reach the caller.
# ---------------------------------------------------------------------------
setup_repo() {
  REPO_DIR="$(mktemp -d)"
  BARE_DIR="${REPO_DIR}.bare"

  # Init the working repo and create an initial commit
  git -C "$REPO_DIR" init -q
  git -C "$REPO_DIR" config user.name  "Test User"
  git -C "$REPO_DIR" config user.email "test@example.com"
  echo "init" > "$REPO_DIR/README"
  git -C "$REPO_DIR" add README
  git -C "$REPO_DIR" commit -q -m "initial"

  HEAD_SHA="$(git -C "$REPO_DIR" rev-parse HEAD)"

  # Create a bare clone that acts as "GitHub"
  git clone --bare -q "$REPO_DIR" "$BARE_DIR"

  # Plant a synthetic pull-request ref in the bare so the helper can fetch it
  git -C "$BARE_DIR" update-ref refs/pull/7/head "$HEAD_SHA"

  # Export so the helper reads GH_FETCH_URL
  export GH_FETCH_URL="file://${BARE_DIR}"
}

# ---------------------------------------------------------------------------
# Scenario A: setup creates a locked worktree
# ---------------------------------------------------------------------------
scenario_a() {
  setup_repo
  local repo="$REPO_DIR" bare="$BARE_DIR" sha="$HEAD_SHA"
  # shellcheck disable=SC2064
  trap "rm -rf \"$repo\" \"$bare\"" EXIT INT TERM

  local wt_path
  wt_path="$(cd "$repo" && bash "$HELPER" setup 7 "$sha" 2>/dev/null)"
  local rc=$?

  [[ $rc -eq 0 ]]         || { echo "  setup exited $rc (expected 0)"; return 1; }
  [[ -d "$wt_path" ]]     || { echo "  printed path not a directory: $wt_path"; return 1; }
  git -C "$repo" worktree list --porcelain | grep -q "^worktree $wt_path$" \
                           || { echo "  worktree not listed"; return 1; }
  git -C "$repo" worktree list --porcelain | grep -A4 "^worktree $wt_path$" | grep -q "^locked" \
                           || { echo "  worktree not locked"; return 1; }
  local head
  head="$(git -C "$wt_path" rev-parse HEAD 2>/dev/null)"
  [[ "$head" == "$sha" ]] || { echo "  HEAD $head != $sha"; return 1; }
}

# ---------------------------------------------------------------------------
# Scenario B: teardown removes the worktree
# ---------------------------------------------------------------------------
scenario_b() {
  setup_repo
  local repo="$REPO_DIR" bare="$BARE_DIR" sha="$HEAD_SHA"
  # shellcheck disable=SC2064
  trap "rm -rf \"$repo\" \"$bare\"" EXIT INT TERM

  local wt_path
  wt_path="$(cd "$repo" && bash "$HELPER" setup 7 "$sha" 2>/dev/null)"
  [[ -d "$wt_path" ]] || { echo "  setup failed, skipping teardown check"; return 1; }

  cd "$repo" && bash "$HELPER" teardown "$wt_path"
  local rc=$?

  [[ $rc -eq 0 ]]       || { echo "  teardown exited $rc (expected 0)"; return 1; }
  [[ ! -d "$wt_path" ]] || { echo "  directory still exists after teardown"; return 1; }
  git -C "$repo" worktree list --porcelain | grep -q "^worktree $wt_path$" \
    && { echo "  worktree still listed after teardown"; return 1; } || true
}

# ---------------------------------------------------------------------------
# Scenario C: second setup of same PR while live-locked → error
# ---------------------------------------------------------------------------
scenario_c() {
  setup_repo
  local repo="$REPO_DIR" bare="$BARE_DIR" sha="$HEAD_SHA"
  # shellcheck disable=SC2064
  trap "rm -rf \"$repo\" \"$bare\"" EXIT INT TERM

  local wt_path
  wt_path="$(cd "$repo" && bash "$HELPER" setup 7 "$sha" 2>/dev/null)"
  [[ -d "$wt_path" ]] || { echo "  first setup failed"; return 1; }

  local stderr_out
  stderr_out="$(cd "$repo" && bash "$HELPER" setup 7 "$sha" 2>&1 1>/dev/null)"
  local rc=$?

  [[ $rc -ne 0 ]] || { echo "  second setup unexpectedly exited 0"; return 1; }
  echo "$stderr_out" | grep -qi "already in progress" \
    || { echo "  stderr did not contain 'already in progress': $stderr_out"; return 1; }
}

# ---------------------------------------------------------------------------
# Scenario D: stale-locked worktree is swept during setup
# ---------------------------------------------------------------------------
scenario_d() {
  setup_repo
  local repo="$REPO_DIR" bare="$BARE_DIR" sha="$HEAD_SHA"
  # shellcheck disable=SC2064
  trap "rm -rf \"$repo\" \"$bare\"" EXIT INT TERM

  # Create a second commit so we have a distinct new SHA for the real setup
  echo "v2" >> "$repo/README"
  git -C "$repo" add README
  git -C "$repo" commit -q -m "v2"
  local new_sha
  new_sha="$(git -C "$repo" rev-parse HEAD)"
  # Push the new object to the bare before updating the ref
  git -C "$repo" push -q "$bare" "HEAD:refs/heads/pr7-v2" 2>/dev/null || true
  git -C "$bare" update-ref refs/pull/7/head "$new_sha"

  # Create the stale worktree under ${ROOT}/review-worktrees/ so the sweep
  # prefix guard matches it; path must be under that directory.
  local ROOT
  ROOT="$(git -C "$repo" rev-parse --path-format=absolute --git-common-dir)"
  mkdir -p "${ROOT}/review-worktrees"
  local stale_path="${ROOT}/review-worktrees/7-${sha:0:7}"
  git -C "$repo" worktree add --detach -q "$stale_path" "$sha"
  local stale_ts
  stale_ts=$(( $(date +%s) - 15 ))
  git -C "$repo" worktree lock --reason "review pr=7 pid=999999 ts=${stale_ts}" "$stale_path"

  export REVIEW_WT_TTL_SECONDS=10
  local new_wt_path
  new_wt_path="$(cd "$repo" && bash "$HELPER" setup 7 "$new_sha" 2>/dev/null)"
  local rc=$?
  unset REVIEW_WT_TTL_SECONDS

  [[ $rc -eq 0 ]] || { echo "  setup exited $rc (expected 0)"; return 1; }
  [[ "$new_wt_path" == /* ]] || { echo "  setup did not print an absolute path: $new_wt_path"; return 1; }

  git -C "$repo" worktree list --porcelain | grep -q "^worktree $stale_path$" \
    && { echo "  stale worktree still present after sweep"; return 1; } || true

}

# ---------------------------------------------------------------------------
# Scenario E: setup with bogus sha → error
# ---------------------------------------------------------------------------
scenario_e() {
  setup_repo
  local repo="$REPO_DIR" bare="$BARE_DIR"
  # shellcheck disable=SC2064
  trap "rm -rf \"$repo\" \"$bare\"" EXIT INT TERM

  cd "$repo" && bash "$HELPER" setup 7 "deadbeef000000000000000000000000deadbeef" 2>/dev/null
  local rc=$?

  [[ $rc -ne 0 ]] || { echo "  setup with bogus sha unexpectedly exited 0"; return 1; }
}

# ---------------------------------------------------------------------------
# Scenario F: teardown is idempotent (twice)
# ---------------------------------------------------------------------------
scenario_f() {
  setup_repo
  local repo="$REPO_DIR" bare="$BARE_DIR" sha="$HEAD_SHA"
  # shellcheck disable=SC2064
  trap "rm -rf \"$repo\" \"$bare\"" EXIT INT TERM

  local wt_path
  wt_path="$(cd "$repo" && bash "$HELPER" setup 7 "$sha" 2>/dev/null)"
  [[ -d "$wt_path" ]] || { echo "  setup failed"; return 1; }

  cd "$repo" && bash "$HELPER" teardown "$wt_path" >/dev/null 2>&1
  cd "$repo" && bash "$HELPER" teardown "$wt_path"
  local rc=$?

  [[ $rc -eq 0 ]] || { echo "  second teardown exited $rc (expected 0)"; return 1; }
}

# ---------------------------------------------------------------------------
# Scenario G: teardown on a path that never existed → succeeds
# ---------------------------------------------------------------------------
scenario_g() {
  setup_repo
  local repo="$REPO_DIR" bare="$BARE_DIR"
  # shellcheck disable=SC2064
  trap "rm -rf \"$repo\" \"$bare\"" EXIT INT TERM

  local ghost_path="${repo}/nonexistent-worktree-path"
  cd "$repo" && bash "$HELPER" teardown "$ghost_path"
  local rc=$?

  [[ $rc -eq 0 ]] || { echo "  teardown on nonexistent path exited $rc (expected 0)"; return 1; }
}

# ---------------------------------------------------------------------------
# Scenario H: fresh live-locked worktree is NOT swept for a different PR
# ---------------------------------------------------------------------------
scenario_h() {
  setup_repo
  local repo="$REPO_DIR" bare="$BARE_DIR" sha="$HEAD_SHA"
  # shellcheck disable=SC2064
  trap "rm -rf \"$repo\" \"$bare\"" EXIT INT TERM

  # Create a second commit for PR 8
  echo "pr8" >> "$repo/README"
  git -C "$repo" add README
  git -C "$repo" commit -q -m "pr8 commit"
  local sha8
  sha8="$(git -C "$repo" rev-parse HEAD)"
  # Push the new object to the bare before updating the ref
  git -C "$repo" push -q "$bare" "HEAD:refs/heads/pr8" 2>/dev/null || true
  git -C "$bare" update-ref refs/pull/8/head "$sha8"

  # Setup PR 7 first (fresh lock)
  local pr7_path
  pr7_path="$(cd "$repo" && bash "$HELPER" setup 7 "$sha" 2>/dev/null)"
  [[ -d "$pr7_path" ]] || { echo "  PR 7 setup failed"; return 1; }

  # Now setup PR 8
  cd "$repo" && bash "$HELPER" setup 8 "$sha8" >/dev/null 2>&1

  # PR 7 worktree must still be present (fresh lock, different PR)
  git -C "$repo" worktree list --porcelain | grep -q "^worktree $pr7_path$" \
    || { echo "  PR 7 worktree was incorrectly swept when setting up PR 8"; return 1; }
}

# ---------------------------------------------------------------------------
# Scenario I: setup from a different CWD prints an absolute path
# ---------------------------------------------------------------------------
scenario_i() {
  setup_repo
  local repo="$REPO_DIR" bare="$BARE_DIR" sha="$HEAD_SHA"
  # shellcheck disable=SC2064
  trap "rm -rf \"$repo\" \"$bare\"" EXIT INT TERM

  # Invoke the helper from /tmp: it must resolve the git repo via GIT_DIR and
  # still print an absolute path.
  local wt_path
  wt_path="$(cd /tmp && GH_FETCH_URL="$GH_FETCH_URL" GIT_DIR="$repo/.git" \
             bash "$HELPER" setup 7 "$sha" 2>/dev/null)"
  local rc=$?

  # Even if the helper fails, what it printed (if anything) must be absolute.
  if [[ -n "$wt_path" ]]; then
    [[ "$wt_path" == /* ]] || { echo "  path is not absolute: $wt_path"; return 1; }
    [[ -d "$wt_path" ]]    || { echo "  absolute path does not exist: $wt_path"; return 1; }
  else
    # Nothing printed → can't verify; treat as failure (helper didn't work from /tmp)
    echo "  helper printed nothing when called from /tmp (rc=$rc)"
    return 1
  fi
}

# Scenario J: origin moved past the requested head_sha, warns but still succeeds
scenario_j() {
  setup_repo
  local repo="$REPO_DIR" bare="$BARE_DIR" sha="$HEAD_SHA"
  # shellcheck disable=SC2064
  trap "rm -rf \"$repo\" \"$bare\"" EXIT INT TERM

  echo "v2" >> "$repo/README"
  git -C "$repo" add README
  git -C "$repo" commit -q -m "v2"
  local new_sha
  new_sha="$(git -C "$repo" rev-parse HEAD)"
  git -C "$repo" push -q "$bare" "HEAD:refs/heads/pr7-moved" 2>/dev/null || true
  git -C "$bare" update-ref refs/pull/7/head "$new_sha"

  local err_file wt_path stderr_out
  err_file="$(mktemp)"
  wt_path="$(cd "$repo" && bash "$HELPER" setup 7 "$sha" 2>"$err_file")"
  stderr_out="$(cat "$err_file")"
  rm -f "$err_file"

  echo "$stderr_out" | grep -qi "has moved" \
    || { echo "  stderr did not warn about the moved origin: $stderr_out"; return 1; }
  [[ -d "$wt_path" ]] || { echo "  setup did not still succeed: $wt_path"; return 1; }
  local head
  head="$(git -C "$wt_path" rev-parse HEAD 2>/dev/null)"
  [[ "$head" == "$sha" ]] || { echo "  checked out $head, expected the originally requested $sha"; return 1; }
}

# ---------------------------------------------------------------------------
# Run all scenarios
# ---------------------------------------------------------------------------
run_scenario "A: setup creates a locked worktree"                   scenario_a
run_scenario "B: teardown removes the worktree"                      scenario_b
run_scenario "C: second setup same PR while live-locked → error"     scenario_c
run_scenario "D: stale-locked worktree swept during setup"           scenario_d
run_scenario "E: setup with bogus sha → error"                       scenario_e
run_scenario "F: teardown is idempotent (twice)"                     scenario_f
run_scenario "G: teardown on never-existed path → success"           scenario_g
run_scenario "H: fresh live-locked worktree NOT swept for diff PR"   scenario_h
run_scenario "I: setup from different CWD prints absolute path"      scenario_i
run_scenario "J: origin moved past head_sha, warns but still succeeds" scenario_j

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
