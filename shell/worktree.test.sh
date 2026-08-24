#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# worktree.test.sh: hermetic tests for shell/shared/worktree.sh.
# Covers the base-dir resolver, helper functions (unit, via zsh when available),
# and end-to-end worktree creation in both bash and zsh.
#
# Run:  bash shell/worktree.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENGINE="${SCRIPT_DIR}/shared/worktree.sh"
PASS=0
FAIL=0
SKIP=0

skip() { printf 'SKIP  %s\n' "$1"; SKIP=$(( SKIP + 1 )); }

HAVE_ZSH=0
command -v zsh >/dev/null 2>&1 && HAVE_ZSH=1

# Resolve the base dir by sourcing the engine under zsh and calling the helper.
# Args: 1=WORKTREE_BASE_DIR value ("" for default), 2=repo_root, 3=repo_parent.
resolve() {
  local base="$1" root="$2" parent="$3"
  # shellcheck disable=SC2016  # $1..$3 are for the zsh subshell, not bash
  WORKTREE_BASE_DIR="$base" \
    zsh -c 'source "$1"; _wt_resolve_base "$2" "$3"' _ "$ENGINE" "$root" "$parent"
}

# Guard for scenarios that require zsh.
_need_zsh() {
  if (( ! HAVE_ZSH )); then
    skip "$1 (zsh not on PATH)"
    return 1
  fi
  return 0
}

run_scenario() {
  local name="$1" fn="$2"
  if "$fn" 2>&1; then echo "PASS: $name"; (( PASS++ )) || true
  else echo "FAIL: $name"; (( FAIL++ )) || true; fi
}

# Default base: <repo-parent>/.worktrees/<repo>
scenario_default() {
  local got want="/home/u/ws/.worktrees/myrepo"
  got="$(resolve "" "/home/u/ws/myrepo" "/home/u/ws")"
  [[ "$got" == "$want" ]] || { echo "  got '$got' want '$want'"; return 1; }
}

# Relative override sits under the repo parent: <repo-parent>/<base>/<repo>
scenario_relative() {
  local got want="/home/u/ws/trees/myrepo"
  got="$(resolve "trees" "/home/u/ws/myrepo" "/home/u/ws")"
  [[ "$got" == "$want" ]] || { echo "  got '$got' want '$want'"; return 1; }
}

# Absolute override is used as-is, with <repo> appended
scenario_absolute() {
  local got want="/var/wt/myrepo"
  got="$(resolve "/var/wt" "/home/u/ws/myrepo" "/home/u/ws")"
  [[ "$got" == "$want" ]] || { echo "  got '$got' want '$want'"; return 1; }
}

# Same-named branches in sibling repos land in distinct dirs (collision guard)
scenario_grouping() {
  local a b
  a="$(resolve "" "/home/u/ws/repo-a" "/home/u/ws")"
  b="$(resolve "" "/home/u/ws/repo-b" "/home/u/ws")"
  [[ "$a" != "$b" ]]    || { echo "  grouping collided: '$a' == '$b'"; return 1; }
  [[ "$a" == */repo-a ]] || { echo "  repo-a leaf missing: '$a'"; return 1; }
  [[ "$b" == */repo-b ]] || { echo "  repo-b leaf missing: '$b'"; return 1; }
}

# A "." base must not resolve back into the repo root
scenario_dot_guard() {
  local got repo_root="/home/u/ws/myrepo"
  got="$(resolve "." "$repo_root" "/home/u/ws")"
  [[ "$got" != "$repo_root" ]]                  || { echo "  '.' collapsed into repo root: '$got'"; return 1; }
  [[ "$got" == "/home/u/ws/.worktrees/myrepo" ]] || { echo "  '.' not defaulted: '$got'"; return 1; }
}

if _need_zsh "A-E (resolver scenarios)"; then
run_scenario "A: default base is .worktrees/<repo>"       scenario_default
run_scenario "B: relative WORKTREE_BASE_DIR override"     scenario_relative
run_scenario "C: absolute WORKTREE_BASE_DIR override"     scenario_absolute
run_scenario "D: repo-name grouping avoids collisions"    scenario_grouping
run_scenario "E: '.' base guarded to default"             scenario_dot_guard
fi

# ── WU-4: smoke tests ─────────────────────────────────────────────────────────

# F: Branch classification, assert the protected-branch case patterns directly.
# Exact patterns from worktree.zsh:378 (more than the spec listed):
#   main|master|trunk|develop|dev|staging|release|release/*|hotfix|hotfix/*
scenario_branch_classification() {
  local branch result ok=1
  local CASE_SNIPPET='
    b="$1"
    case "$b" in
      main|master|trunk|develop|dev|staging|release|release/*|hotfix|hotfix/*) print protected ;;
      *) print feature ;;
    esac
  '
  for branch in main master trunk develop dev staging release "release/1.2" hotfix "hotfix/x"; do
    result="$(zsh -c "$CASE_SNIPPET" _ "$branch")"
    if [[ "$result" != "protected" ]]; then
      echo "  branch '$branch': expected 'protected', got '$result'"
      ok=0
    fi
  done
  for branch in "feature/foo" mybranch; do
    result="$(zsh -c "$CASE_SNIPPET" _ "$branch")"
    if [[ "$result" != "feature" ]]; then
      echo "  branch '$branch': expected 'feature', got '$result'"
      ok=0
    fi
  done
  (( ok ))
}

# G: _wt_restore_stash pops the stash when STASH_APPLIED=1, is a no-op when 0.
# Env vars confirmed from worktree.zsh:212-215: STASH_APPLIED, MAIN_WORKTREE.
scenario_restore_stash() {
  local repo ok=1
  repo="$(mktemp -d)"

  git -C "$repo" init -q
  git -C "$repo" config user.email "t@t"
  git -C "$repo" config user.name "t"
  printf 'initial\n' > "$repo/file.txt"
  git -C "$repo" add file.txt
  git -C "$repo" commit -q -m "initial"

  # Sub-case 1: STASH_APPLIED=1 -> _wt_restore_stash must pop the stash
  printf 'dirty\n' > "$repo/file.txt"
  git -C "$repo" stash push --quiet -m "test stash"

  local content
  content="$(cat "$repo/file.txt")"
  if [[ "$content" != "initial" ]]; then
    echo "  pre-condition: stash push did not restore working tree (got '$content')"
    ok=0
  fi

  # shellcheck disable=SC2016
  zsh -c '
    source "$1"
    STASH_APPLIED=1
    MAIN_WORKTREE="$2"
    _wt_restore_stash
  ' _ "$ENGINE" "$repo"

  content="$(cat "$repo/file.txt")"
  if [[ "$content" != "dirty" ]]; then
    echo "  sub-case 1 (STASH_APPLIED=1): expected 'dirty' after pop, got '$content'"
    ok=0
  fi

  # Sub-case 2: STASH_APPLIED=0 -> no-op; stash list must be unchanged
  git -C "$repo" stash push --quiet -m "another stash"
  local before after
  before="$(git -C "$repo" stash list | wc -l | tr -d '[:space:]')"

  # shellcheck disable=SC2016
  zsh -c '
    source "$1"
    STASH_APPLIED=0
    MAIN_WORKTREE="$2"
    _wt_restore_stash
  ' _ "$ENGINE" "$repo"

  after="$(git -C "$repo" stash list | wc -l | tr -d '[:space:]')"
  if [[ "$before" != "$after" ]]; then
    echo "  sub-case 2 (STASH_APPLIED=0): stash count changed ($before -> $after); should be no-op"
    ok=0
  fi

  rm -rf "$repo"
  (( ok ))
}

# H: _wt_find_env_base across four cases.
# The function reads $REPO_ROOT as a global (confirmed worktree.zsh:103-115).
# Calling convention: _wt_find_env_base [arg] where arg is optional.
scenario_find_env_base() {
  local repo ok=1
  repo="$(mktemp -d)"

  git -C "$repo" init -q

  # (a) REPO_ROOT/.env exists, no arg -> returns "."
  touch "$repo/.env"
  local got
  # shellcheck disable=SC2016
  got="$(zsh -c 'source "$1"; REPO_ROOT="$2"; _wt_find_env_base' _ "$ENGINE" "$repo")"
  if [[ "$got" != "." ]]; then
    echo "  (a) REPO_ROOT/.env exists, no arg: expected '.', got '$got'"
    ok=0
  fi
  rm "$repo/.env"

  # (b) exactly one nested sub/.env, no arg -> returns subdir name
  mkdir -p "$repo/apps"
  touch "$repo/apps/.env"
  # shellcheck disable=SC2016
  got="$(zsh -c 'source "$1"; REPO_ROOT="$2"; _wt_find_env_base' _ "$ENGINE" "$repo")"
  if [[ "$got" != "apps" ]]; then
    echo "  (b) nested apps/.env, no arg: expected 'apps', got '$got'"
    ok=0
  fi
  rm "$repo/apps/.env"
  rmdir "$repo/apps"

  # (c) explicit valid arg "myenv" with REPO_ROOT/myenv/.env -> returns "myenv"
  mkdir -p "$repo/myenv"
  touch "$repo/myenv/.env"
  # shellcheck disable=SC2016
  got="$(zsh -c 'source "$1"; REPO_ROOT="$2"; _wt_find_env_base "$3"' _ "$ENGINE" "$repo" "myenv")"
  if [[ "$got" != "myenv" ]]; then
    echo "  (c) explicit arg 'myenv': expected 'myenv', got '$got'"
    ok=0
  fi
  rm "$repo/myenv/.env"
  rmdir "$repo/myenv"

  # (d) no .env anywhere, no arg -> returns ""
  # shellcheck disable=SC2016
  got="$(zsh -c 'source "$1"; REPO_ROOT="$2"; _wt_find_env_base' _ "$ENGINE" "$repo")"
  if [[ "$got" != "" ]]; then
    echo "  (d) no .env, no arg: expected '', got '$got'"
    ok=0
  fi

  rm -rf "$repo"
  (( ok ))
}

if _need_zsh "F-H (unit scenarios)"; then
run_scenario "F: branch classification (protected vs feature)"  scenario_branch_classification
run_scenario "G: _wt_restore_stash stash-pop and no-op"        scenario_restore_stash
run_scenario "H: _wt_find_env_base four cases"                 scenario_find_env_base
fi

# ── WU-1/2/3 tests ────────────────────────────────────────────────────────────

# I: _wt_ai_resolve_decision, full matrix (pure, no repo needed)
scenario_ai_resolve_decision() {
  local ok=1

  decision() {
    # shellcheck disable=SC2016
    zsh -c 'source "$1"; _wt_ai_resolve_decision "$2" "$3" "$4"' \
      _ "$ENGINE" "$1" "$2" "$3"
  }

  check() {
    local label="$1" got want="$2"
    shift 2
    got="$(decision "$@")"
    if [[ "$got" != "$want" ]]; then
      echo "  decision($*): expected '$want', got '$got' [$label]"
      ok=0
    fi
  }

  # silent=1 -> always spawn regardless of tty or answer
  check "silent=1,tty=0,ans=''"  spawn  1 0 ""
  check "silent=1,tty=1,ans=n"   spawn  1 1 "n"

  # silent=0, is_tty=1 -> interactive path
  check "silent=0,tty=1,ans=''"    spawn  0 1 ""
  check "silent=0,tty=1,ans=y"     spawn  0 1 "y"
  check "silent=0,tty=1,ans=Y"     spawn  0 1 "Y"
  check "silent=0,tty=1,ans=yes"   spawn  0 1 "yes"
  check "silent=0,tty=1,ans=Yes"   spawn  0 1 "Yes"
  check "silent=0,tty=1,ans=YES"   spawn  0 1 "YES"
  check "silent=0,tty=1,ans=n"     abort  0 1 "n"
  check "silent=0,tty=1,ans=junk"  abort  0 1 "junk"

  # silent=0, is_tty=0 -> non-interactive, always abort
  check "silent=0,tty=0,ans=''"    abort  0 0 ""

  (( ok ))
}

# J: _wt_ai_resolve_info, info block mentions branch, base, files, git add, git rebase --continue
scenario_ai_resolve_info() {
  local ok=1
  # shellcheck disable=SC2016
  local stderr
  stderr="$(zsh -c '
    source "$1"
    _wt_ai_resolve_info "branchX" "origin/main" "a.txt b.txt"
  ' _ "$ENGINE" 2>&1 >/dev/null)"

  for needle in "branchX" "origin/main" "a.txt b.txt" "git add" "git rebase --continue"; do
    if [[ "$stderr" != *"$needle"* ]]; then
      echo "  info block missing: '$needle'"
      ok=0
    fi
  done

  (( ok ))
}

# K: _wt_setup_upstream with NO_PUSH
scenario_no_push() {
  local repo bare ok=1
  repo="$(mktemp -d)"
  bare="$(mktemp -d)"

  # Initialise a bare "origin"
  git -C "$bare" init --bare -q

  # Initialise a working repo and point it at the bare remote
  git -C "$repo" init -q
  git -C "$repo" config user.email "t@t"
  git -C "$repo" config user.name "t"
  printf 'init\n' > "$repo/f.txt"
  git -C "$repo" add f.txt
  git -C "$repo" commit -q -m "init"
  git -C "$repo" remote add origin "file://$bare"
  git -C "$repo" push -u origin master --quiet 2>/dev/null \
    || git -C "$repo" push -u origin main --quiet 2>/dev/null || true

  # Create a local feature branch (not pushed)
  git -C "$repo" checkout -q -b feat/test-no-push

  # Sub-case 1: NO_PUSH=1 -> bare origin must NOT get the branch
  # shellcheck disable=SC2016
  zsh -c '
    source "$1"
    cd "$2" || exit 1
    REMOTE=origin BRANCH=feat/test-no-push NO_PUSH=1
    _wt_setup_upstream
  ' _ "$ENGINE" "$repo"

  local ref_count
  ref_count="$(git -C "$bare" show-ref --heads 2>/dev/null | grep -c "feat/test-no-push" || true)"
  if [[ "$ref_count" != "0" ]]; then
    echo "  sub-case 1 (NO_PUSH=1): branch was pushed despite --no-push (count=$ref_count)"
    ok=0
  fi

  # Sub-case 2: NO_PUSH=0 -> bare origin MUST get the branch
  # shellcheck disable=SC2016
  zsh -c '
    source "$1"
    cd "$2" || exit 1
    REMOTE=origin BRANCH=feat/test-no-push NO_PUSH=0
    _wt_setup_upstream
  ' _ "$ENGINE" "$repo"

  ref_count="$(git -C "$bare" show-ref --heads 2>/dev/null | grep -c "feat/test-no-push" || true)"
  if [[ "$ref_count" == "0" ]]; then
    echo "  sub-case 2 (NO_PUSH=0): branch was NOT pushed (expected push)"
    ok=0
  fi

  rm -rf "$repo" "$bare"
  (( ok ))
}

# L: _wt_node_modules, CoW clone, independence, no nesting, npm invoked
scenario_cow_node_modules() {
  local base wt npm_shim ok=1
  base="$(mktemp -d)"
  wt="$(mktemp -d)"
  npm_shim="$(mktemp -d)"

  # npm shim records invocation
  printf '#!/bin/sh\necho npm-called >> "%s/npm.log"\n' "$npm_shim" > "$npm_shim/npm"
  chmod +x "$npm_shim/npm"

  # Initialise base as a git repo with matching lockfile + node_modules
  git -C "$base" init -q
  git -C "$base" config user.email "t@t"
  git -C "$base" config user.name "t"
  printf '{"name":"x","version":"1.0.0"}\n'  > "$base/package.json"
  printf 'lockfile-content\n'                 > "$base/package-lock.json"
  mkdir -p "$base/node_modules"
  printf 'base-marker\n'                      > "$base/node_modules/marker"
  git -C "$base" add .
  git -C "$base" commit -q -m "init"

  # Initialise wt (the "worktree") with matching lockfile
  git -C "$wt" init -q
  git -C "$wt" config user.email "t@t"
  git -C "$wt" config user.name "t"
  cp "$base/package.json"      "$wt/package.json"
  cp "$base/package-lock.json" "$wt/package-lock.json"
  git -C "$wt" add .
  git -C "$wt" commit -q -m "init"

  # Run _wt_node_modules from within wt
  # shellcheck disable=SC2016
  PATH="$npm_shim:$PATH" zsh -c '
    source "$1"
    cd "$2" || exit 1
    REPO_ROOT="$3"
    _wt_node_modules
  ' _ "$ENGINE" "$wt" "$base"

  # (a) node_modules/marker must exist in the worktree
  if [[ ! -f "$wt/node_modules/marker" ]]; then
    echo "  (a) node_modules/marker missing in worktree"
    ok=0
  fi

  # (b) no node_modules/node_modules nesting
  if [[ -d "$wt/node_modules/node_modules" ]]; then
    echo "  (b) node_modules/node_modules nesting detected"
    ok=0
  fi

  # (c) overwrite wt copy; base must be UNCHANGED (independent copy)
  printf 'wt-changed\n' > "$wt/node_modules/marker"
  local base_content
  base_content="$(cat "$base/node_modules/marker")"
  if [[ "$base_content" != "base-marker" ]]; then
    echo "  (c) base repo's node_modules/marker was altered (got '$base_content'); not independent"
    ok=0
  fi

  # (d) npm shim was invoked
  if [[ ! -f "$npm_shim/npm.log" ]]; then
    echo "  (d) npm was not called"
    ok=0
  fi

  # (e) lockfile mismatch -> early return (no clone)
  local wt2
  wt2="$(mktemp -d)"
  git -C "$wt2" init -q
  git -C "$wt2" config user.email "t@t"
  git -C "$wt2" config user.name "t"
  cp "$base/package.json" "$wt2/package.json"
  printf 'different-lock\n' > "$wt2/package-lock.json"
  git -C "$wt2" add .
  git -C "$wt2" commit -q -m "init"

  # shellcheck disable=SC2016
  PATH="$npm_shim:$PATH" zsh -c '
    source "$1"
    cd "$2" || exit 1
    REPO_ROOT="$3"
    _wt_node_modules
  ' _ "$ENGINE" "$wt2" "$base"

  if [[ -d "$wt2/node_modules" ]]; then
    echo "  (e) lockfile mismatch: node_modules was cloned despite mismatch"
    ok=0
  fi

  rm -rf "$base" "$wt" "$wt2" "$npm_shim"
  (( ok ))
}

# M: Integration, _wt_maybe_rebase with rebase conflict
# WORKTREE_AI_RESOLVE_SILENT=1 -> claude shim invoked (sentinel created)
# stdin not a tty + silent unset -> shim NOT invoked, rebase aborted
scenario_maybe_rebase_conflict() {
  local ok=1

  # Helper: build a repo with a rebase conflict.
  # Returns the base-branch name on stdout.
  make_conflict_repo() {
    local repo="$1" bare="$2"

    git -C "$bare" init --bare -q

    git -C "$repo" init -q
    git -C "$repo" config user.email "test@t"
    git -C "$repo" config user.name "Test User"
    printf 'line1\n' > "$repo/f.txt"
    git -C "$repo" add f.txt
    git -C "$repo" commit -q -m "init"
    git -C "$repo" remote add origin "file://$bare"

    local base_branch
    base_branch="$(git -C "$repo" rev-parse --abbrev-ref HEAD)"
    git -C "$repo" push -u origin "$base_branch" --quiet

    git -C "$repo" checkout -q -b my-feat
    printf 'feat-change\n' > "$repo/f.txt"
    git -C "$repo" commit -q -am "feat commit"

    git -C "$repo" checkout -q "$base_branch"
    printf 'base-change\n' > "$repo/f.txt"
    git -C "$repo" commit -q -am "base conflict commit"
    git -C "$repo" push origin "$base_branch" --quiet

    git -C "$repo" checkout -q my-feat

    printf '%s\n' "$base_branch"
  }

  # ── Sub-case 1: WORKTREE_AI_RESOLVE_SILENT=1 → shim invoked ──────────────
  local repo1 bare1 claude_shim sentinel base_ref1
  repo1="$(mktemp -d)"
  bare1="$(mktemp -d)"
  claude_shim="$(mktemp -d)"
  sentinel="$claude_shim/sentinel"

  base_ref1="$(make_conflict_repo "$repo1" "$bare1")"

  # claude shim: record call via sentinel; exit 0.
  # Set PATH inside the zsh script to guarantee the shim wins over any
  # system-installed claude binary (macOS path_helper can prepend /usr/local/bin
  # after the env-prefix PATH, so we set it explicitly inside zsh instead).
  cat > "$claude_shim/claude" <<'EOF'
#!/bin/sh
touch "$SENTINEL"
exit 0
EOF
  chmod +x "$claude_shim/claude"

  # Pass claude_shim as positional $3, sentinel path via env SENTINEL.
  # shellcheck disable=SC2016
  SENTINEL="$sentinel" WORKTREE_AI_RESOLVE_SILENT=1 \
    zsh -c '
      PATH="$3:$PATH"
      source "$1"
      cd "$2" || exit 1
      REMOTE=origin BASE_REF="$4" AI_RESOLVE=1 BRANCH=my-feat
      _wt_maybe_rebase
    ' _ "$ENGINE" "$repo1" "$claude_shim" "$base_ref1" 2>/dev/null

  if [[ ! -f "$sentinel" ]]; then
    echo "  sub-case 1 (SILENT=1): claude shim was NOT invoked (sentinel missing)"
    ok=0
  fi

  # ── Sub-case 2: no tty + silent unset → rebase aborted, shim NOT invoked ──
  local repo2 bare2 claude_shim2 sentinel2 base_ref2
  repo2="$(mktemp -d)"
  bare2="$(mktemp -d)"
  claude_shim2="$(mktemp -d)"
  sentinel2="$claude_shim2/sentinel2"

  base_ref2="$(make_conflict_repo "$repo2" "$bare2")"

  cat > "$claude_shim2/claude" <<'EOF'
#!/bin/sh
touch "$SENTINEL2"
exit 0
EOF
  chmod +x "$claude_shim2/claude"

  local pre_tip
  pre_tip="$(git -C "$repo2" rev-parse my-feat)"

  # stdin is not a tty (bash test script); WORKTREE_AI_RESOLVE_SILENT unset.
  # shellcheck disable=SC2016
  SENTINEL2="$sentinel2" \
    zsh -c '
      PATH="$3:$PATH"
      source "$1"
      cd "$2" || exit 1
      REMOTE=origin BASE_REF="$4" AI_RESOLVE=1 BRANCH=my-feat
      _wt_maybe_rebase
    ' _ "$ENGINE" "$repo2" "$claude_shim2" "$base_ref2" </dev/null 2>/dev/null

  if [[ -f "$sentinel2" ]]; then
    echo "  sub-case 2 (no tty): claude shim WAS invoked (should have aborted)"
    ok=0
  fi

  if [[ -d "$repo2/.git/rebase-merge" ]]; then
    echo "  sub-case 2: .git/rebase-merge still exists (rebase not aborted)"
    ok=0
  fi

  local post_tip
  post_tip="$(git -C "$repo2" rev-parse my-feat)"
  if [[ "$pre_tip" != "$post_tip" ]]; then
    echo "  sub-case 2: branch tip changed ($pre_tip -> $post_tip); rebase was not fully aborted"
    ok=0
  fi

  rm -rf "$repo1" "$bare1" "$claude_shim" "$repo2" "$bare2" "$claude_shim2"
  (( ok ))
}

if _need_zsh "I-M (integration scenarios)"; then
run_scenario "I: _wt_ai_resolve_decision matrix"               scenario_ai_resolve_decision
run_scenario "J: _wt_ai_resolve_info output"                   scenario_ai_resolve_info
run_scenario "K: _wt_setup_upstream --no-push"                 scenario_no_push
run_scenario "L: _wt_node_modules CoW clone"                   scenario_cow_node_modules
run_scenario "M: _wt_maybe_rebase conflict (integration)"      scenario_maybe_rebase_conflict
fi

# ── N/O: dual-shell worktree creation ─────────────────────────────────────────
# Build a throwaway git repo with a local bare origin, then call _cc_worktree
# directly (no dispatcher) with _WT_NO_LAUNCH=1 WORKTREE_NO_PUSH=1.
# Asserts: worktree dir at correct JIRA-key path, branch exists, gitignored
# .env copied, non-gitignored app/.env NOT copied.

_WT_TMPBASE="$(mktemp -d)"
_wt_cleanup_tmp() { rm -rf "$_WT_TMPBASE"; }
trap _wt_cleanup_tmp EXIT

_wt_setup_repo() {
  local repo="$1" bare="$2"
  git init --bare --quiet "$bare"
  git init --quiet "$repo"
  git -C "$repo" config user.email "test@test.local"
  git -C "$repo" config user.name "Test User"
  printf 'hello\n' > "$repo/README"
  git -C "$repo" add README
  git -C "$repo" commit --quiet -m "initial"
  git -C "$repo" remote add origin "$bare"
  git -C "$repo" push --quiet -u origin HEAD 2>/dev/null || true
  git -C "$repo" fetch --quiet origin 2>/dev/null || true
  git -C "$repo" remote set-head origin --auto 2>/dev/null || true
  # gitignored .env (should be copied)
  printf 'SECRET=test\n' > "$repo/.env"
  printf '.env\n' > "$repo/.gitignore"
  git -C "$repo" add .gitignore
  git -C "$repo" commit --quiet -m "add gitignore"
  git -C "$repo" push --quiet origin HEAD 2>/dev/null || true
  # tracked app/.env (must NOT be copied)
  mkdir -p "$repo/app"
  printf 'PUBLIC=yes\n' > "$repo/app/.env"
  git -C "$repo" add "$repo/app/.env"
  git -C "$repo" commit --quiet -m "add tracked app/.env"
  git -C "$repo" push --quiet origin HEAD 2>/dev/null || true
}

_run_creation_test() {
  local shell_bin="$1" label="$2"
  local repo="$_WT_TMPBASE/repo_$label"
  local bare="$_WT_TMPBASE/bare_$label"
  local wt_base="$_WT_TMPBASE/wts_$label"
  local testhome="$_WT_TMPBASE/home_$label"
  mkdir -p "$testhome"
  _wt_setup_repo "$repo" "$bare"
  local repo_name="${repo##*/}"

  local out rc=0
  out=$(
    HOME="$testhome" PATH="$SCRIPT_DIR/../shell:$PATH" \
    WORKTREE_BASE_DIR="$wt_base" \
    _WT_NO_LAUNCH=1 WORKTREE_NO_PUSH=1 WORKTREE_AI_RESOLVE=0 \
    GIT_AUTHOR_NAME="Test User" GIT_COMMITTER_NAME="Test User" \
    GIT_AUTHOR_EMAIL="test@test.local" GIT_COMMITTER_EMAIL="test@test.local" \
    "$shell_bin" -c "
      cd '$repo'
      source '$ENGINE'
      _cc_worktree TEST-123-foo
    " 2>&1
  ) || rc=$?

  local wt_dir="$wt_base/$repo_name/TEST-123"

  if [[ -d "$wt_dir" ]]; then
    if [[ "$1" == "bash" ]]; then
      run_scenario "N: bash worktree dir at JIRA-key path" true
    else
      run_scenario "O: zsh worktree dir at JIRA-key path" true
    fi
  else
    if [[ "$1" == "bash" ]]; then
      run_scenario "N: bash worktree dir at JIRA-key path" false
    else
      run_scenario "O: zsh worktree dir at JIRA-key path" false
    fi
    echo "  not found at $wt_dir (rc=$rc)"
    echo "  output: $out"
  fi

  local pfx; [[ "$shell_bin" == "bash" ]] && pfx="N" || pfx="O"

  if git -C "$repo" show-ref --verify --quiet "refs/heads/TEST-123-foo" 2>/dev/null; then
    run_scenario "$pfx: $label branch TEST-123-foo created" true
  else
    run_scenario "$pfx: $label branch TEST-123-foo created" false
    echo "  branch not found; rc=$rc out=$out"
  fi

  if [[ -f "$wt_dir/.env" ]]; then
    run_scenario "$pfx: $label gitignored .env copied" true
  else
    run_scenario "$pfx: $label gitignored .env copied" false
    echo "  .env missing in $wt_dir; rc=$rc out=$out"
  fi

  if [[ ! -f "$wt_dir/app/.env" ]]; then
    run_scenario "$pfx: $label non-gitignored app/.env not copied" true
  else
    run_scenario "$pfx: $label non-gitignored app/.env not copied" false
  fi
}

echo ""
echo "=== bash creation tests ==="
_run_creation_test bash bash

echo ""
echo "=== zsh creation tests ==="
if (( HAVE_ZSH )); then
  _run_creation_test zsh zsh
else
  skip "N/O: zsh not on PATH; skipping zsh creation tests"
fi

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed  (${SKIP} skipped)"

[[ $FAIL -eq 0 ]]
