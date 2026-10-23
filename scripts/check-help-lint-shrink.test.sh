#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# check-help-lint-shrink.test.sh: offline tests for check-help-lint-shrink.sh.
# Every repository is a temp one, every remote is a local file:// path.
#
# Usage: check-help-lint-shrink.test.sh
set -euo pipefail

cd "$(dirname "$0")/.."

SCRIPT="$PWD/scripts/check-help-lint-shrink.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# The test must not depend on the identity or signing setup of the machine.
export GIT_CONFIG_GLOBAL="$TMP/gitconfig"
export GIT_CONFIG_SYSTEM=/dev/null
git config --global user.name tester
git config --global user.email tester@example.com
git config --global commit.gpgsign false
git config --global init.defaultBranch main

ALLOW_DIR=crates/cli/tests/help_lint_allowlist
ALLOW_FILE=$ALLOW_DIR/user-core.txt
FLAGS=crates/cli/tests/golden/flags.txt
LINT=crates/cli/tests/help_lint.rs

pass=0
fail() { echo "FAIL $*" >&2; exit 1; }
ok() { pass=$((pass + 1)); }

RC=0
OUT=""
ERR=""

# run <ci|local> [script args...]: runs the script in the current
# repository with stdin closed and records RC, OUT and ERR.
run() {
  local mode="$1"
  shift
  local script="$SCRIPT"
  RC=0
  if [ "$mode" = ci ]; then
    CI=1 bash "$script" "$@" <&- >"$TMP/out" 2>"$TMP/err" || RC=$?
  else
    env -u CI bash "$script" "$@" <&- >"$TMP/out" 2>"$TMP/err" || RC=$?
  fi
  OUT="$(cat "$TMP/out")"
  ERR="$(cat "$TMP/err")"
}

# expect <label> <exit code> [stderr text]: an exit-1 row passes the text that
# stderr must contain.
expect() {
  local label="$1" code="$2" text="${3:-}"
  [ "$RC" -eq "$code" ] || fail "$label: exit $RC, want $code; stderr: $ERR"
  if [ -n "$text" ]; then
    case "$ERR" in
      *"$text"*) ;;
      *) fail "$label: stderr lacks '$text': $ERR" ;;
    esac
  fi
  ok
}

write_guarded() {
  mkdir -p "$ALLOW_DIR" "$(dirname "$FLAGS")" "$(dirname "$LINT")"
  printf 'H4 one\nH4 two\n' >"$ALLOW_FILE"
  printf 'flag one\n' >"$FLAGS"
  printf '// RULES-BEGIN\nconst H4_MAX: usize = 70;\n// RULES-END\nfn other() {}\n' >"$LINT"
}

commit_all() {
  git add -A
  git commit -q -m "$1"
}

# new_repo <name>: a repository with the guarded paths on main, as commit M.
new_repo() {
  REPO="$TMP/$1"
  mkdir -p "$REPO"
  cd "$REPO"
  git init -q -b main
  write_guarded
  commit_all base
  M="$(git rev-parse HEAD)"
}

# new_bare_repo <name>: the same, but with no guarded paths.
new_empty_repo() {
  REPO="$TMP/$1"
  mkdir -p "$REPO"
  cd "$REPO"
  git init -q -b main
  echo readme >README
  commit_all base
  M="$(git rev-parse HEAD)"
}

# topic: switches to a branch off M for the edit.
topic() { git switch -q -c topic "$M"; }

# ci_shape: the pull_request merge commit, whose first parent is M.
ci_shape() {
  git switch -q --detach "$M"
  git merge -q --no-ff -m merge topic
}

# new_clone <name>: a bare origin holding M, and a clone of it with branch topic.
new_clone() {
  local name="$1"
  ORIGIN="$TMP/$name-origin.git"
  git init -q --bare -b main "$ORIGIN"
  new_repo "$name-seed"
  git push -q "file://$ORIGIN" main
  cd "$TMP"
  git clone -q "file://$ORIGIN" "$name"
  REPO="$TMP/$name"
  cd "$REPO"
}

# --- CI shape ---------------------------------------------------------------

new_repo ci_single
run ci
expect "CI with one commit" 1 "fetch-depth: 2"

new_repo ci_add
topic
echo 'H4 three' >>"$ALLOW_FILE"
commit_all add
ci_shape
run ci
expect "an added allowlist line" 1 "$ALLOW_FILE"

new_repo ci_remove
topic
printf 'H4 one\n' >"$ALLOW_FILE"
commit_all remove
ci_shape
run ci
expect "a removed allowlist line" 0

new_repo ci_new_file
topic
echo 'H4 x' >"$ALLOW_DIR/setup.txt"
commit_all newfile
ci_shape
run ci
expect "a new file in an existing allowlist directory" 1 "$ALLOW_DIR/setup.txt"

new_repo ci_delete_all
topic
git rm -q -r "$ALLOW_DIR"
commit_all delete
ci_shape
run ci
expect "every allowlist file deleted" 0

new_repo ci_flags
topic
echo 'flag two' >>"$FLAGS"
commit_all flags
ci_shape
run ci
expect "an edited flags.txt" 1 "flags.txt"
run ci --allow-flags-change
expect "an edited flags.txt with the flag" 0

new_empty_repo ci_flags_new
topic
write_guarded
commit_all create
ci_shape
run ci
expect "the S7a case: everything created in one PR" 0

new_repo ci_second_golden
topic
echo other >crates/cli/tests/golden/other.txt
commit_all second
ci_shape
run ci
expect "a second golden file" 0

new_repo ci_rules
topic
sed -i.bak 's/70/80/' "$LINT"
rm -f "$LINT.bak"
commit_all rules
ci_shape
run ci
expect "a changed rule constant" 1 "RULES"
run ci --allow-rules-change
expect "a changed rule constant with the flag" 0

new_repo ci_outside
topic
echo 'fn more() {}' >>"$LINT"
commit_all outside
ci_shape
run ci
expect "a change outside the rule block" 0

new_repo ci_markers
topic
printf 'const H4_MAX: usize = 70;\n' >"$LINT"
commit_all markers
ci_shape
run ci
expect "the markers removed" 1 "RULES-BEGIN"
run ci --allow-rules-change
expect "the markers removed with the flag" 0

new_repo ci_root_copy
topic
echo 'H4 three' >>"$ALLOW_FILE"
commit_all add
ci_shape
mkdir -p "$TMP/copy"
cp "$SCRIPT" "$TMP/copy/guard.sh"
SCRIPT_BACKUP="$SCRIPT"
SCRIPT="$TMP/copy/guard.sh"
cd crates/cli
run ci
SCRIPT="$SCRIPT_BACKUP"
expect "the script run from a copy in a subdirectory" 1 "$ALLOW_FILE"

# --- Local shape ------------------------------------------------------------

new_clone local_add
topic
echo 'H4 three' >>"$ALLOW_FILE"
commit_all add
run local
expect "a local branch that adds an allowlist line" 1 "$ALLOW_FILE"

new_clone local_noop
run local
expect "HEAD equal to origin/main" 0

new_empty_repo local_no_remote
run local
expect "no remote" 0
case "$OUT" in
  *skipped*) ok ;;
  *) fail "no remote: stdout lacks the skip line: $OUT" ;;
esac

new_empty_repo local_offline
git remote add origin "$TMP/does-not-exist.git"
run local
expect "a remote that cannot be fetched" 0
case "$OUT" in
  *skipped*) ok ;;
  *) fail "offline: stdout lacks the skip line: $OUT" ;;
esac

# The shallow clone: origin moves on after the clone, then a depth-1 fetch
# leaves HEAD and origin/main with no common commit.
ORIGIN="$TMP/shallow-origin.git"
git init -q --bare -b main "$ORIGIN"
new_repo shallow_seed
git push -q "file://$ORIGIN" main
cd "$TMP"
git clone -q --depth 1 "file://$ORIGIN" shallow
cd "$TMP/shallow_seed"
echo more >README
commit_all more
git push -q "file://$ORIGIN" main
cd "$TMP/shallow"
git fetch -q --depth 1 origin main
run local
expect "a shallow clone" 1 "unshallow"

echo "check-help-lint-shrink.test.sh: $pass checks passed"
