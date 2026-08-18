#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# install-backup-prune.test.sh: re-running install.sh must not grow
# CLAUDE_HOME/backups without bound.
#
# Re-running the installer is the documented upgrade path, and each run past the
# first copies the whole previous tree into backups/: 13M over nine installs
# before the cap. The loop sleeps because the stamp has one-second resolution,
# and colliding names would keep the count low for the wrong reason.
#
# Run:  bash shell/install-backup-prune.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1${2:+ -- $2}"; (( FAIL++ )) || true; }

if ! git -C "$REPO_ROOT" rev-parse --git-dir >/dev/null 2>&1; then
    echo "SKIP: not a git checkout, cannot build a tracked-files-only source"
    exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

SRC="$WORK/src"
HOME_DIR="$WORK/home"
CLAUDE_DIR="$HOME_DIR/.claude"
mkdir -p "$SRC" "$CLAUDE_DIR"
git -C "$REPO_ROOT" archive HEAD | tar -x -C "$SRC"

RUNS=7
KEEP=5

# Glob rather than `ls | grep`: an unmatched glob stays literal, so the -d test
# is what filters it out when no backup exists yet.
list_backups() {
    local d
    for d in "$CLAUDE_DIR"/backups/install-*; do
        [ -d "$d" ] || continue
        basename "$d"
    done | sort
}

for _ in $(seq 1 "$RUNS"); do
    PLAYBOOK_SRC="$SRC" CLAUDE_HOME="$CLAUDE_DIR" HOME="$HOME_DIR" \
        bash "$REPO_ROOT/install.sh" --no-setup --skip-plugin >/dev/null 2>&1
    sleep 1
done

# Plain strings, not arrays. macOS ships bash 3.2, which has no mapfile, and
# under `set -u` indexing an empty array there is an unbound-variable error.
# The macOS leg of shell-ci exists to catch exactly this, and it did.
backups="$(list_backups)"
count="$(printf '%s\n' "$backups" | grep -c . || true)"

# Guard against a vacuous pass: if the installer never made a backup at all,
# "at most 5" would hold trivially and this suite would prove nothing.
if [ "$count" -ge 2 ]; then
    pass "re-running install creates backups at all"
else
    fail "re-running install creates backups at all" "only $count after $RUNS runs"
fi

if [ "$count" -le "$KEEP" ]; then
    pass "install backups are capped at $KEEP"
else
    fail "install backups are capped at $KEEP" "$count dirs after $RUNS runs"
fi

# Keeping the OLDEST five would also satisfy a bare count check, and would be
# the wrong five: the newest backup holds the tree the user just replaced, so
# it is the one worth recovering from. Names are timestamped, so lexical order
# is chronological. Run once more and confirm the newest ADVANCES; a prune that
# dropped the newest instead would leave it frozen.
if [ "$count" -ge 1 ]; then
    prev_newest="$(printf '%s\n' "$backups" | tail -1)"
    sleep 1
    PLAYBOOK_SRC="$SRC" CLAUDE_HOME="$CLAUDE_DIR" HOME="$HOME_DIR" \
        bash "$REPO_ROOT/install.sh" --no-setup --skip-plugin >/dev/null 2>&1
    latest="$(list_backups | tail -1)"
    if [ -n "$latest" ] && [ "$latest" \> "$prev_newest" ]; then
        pass "pruning drops the oldest, not the newest"
    else
        fail "pruning drops the oldest, not the newest" "newest stayed at $prev_newest"
    fi
else
    fail "pruning drops the oldest, not the newest" "no backups to order"
fi

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
