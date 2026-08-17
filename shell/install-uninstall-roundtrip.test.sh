#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# install-uninstall-roundtrip.test.sh: install.sh and uninstall.sh must agree
# on what the toolkit owns.
#
# They cannot agree by construction: install.sh derives its copy set at runtime
# (every top-level entry minus a short skip list) while uninstall.sh carries a
# hardcoded SHIPPED array. Every file added to the repo root is therefore
# installed immediately and uninstalled only if somebody remembers. Six had
# already slipped through when this suite was written (.claude-plugin,
# Cargo.lock, Cargo.toml, ruff.toml, src, tests), all surviving an uninstall.
#
# Rather than re-parse either list, which would rot alongside them, this runs
# the real pair end to end against a scratch HOME and inspects what is left.
# Source is `git archive HEAD`, so it sees tracked files only: a plain copy of
# the working tree would drag in target/ and other untracked build output and
# report failures that no user could ever hit.
#
# Run:  bash shell/install-uninstall-roundtrip.test.sh
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

# --force bypasses the git-repo guard. The scratch HOME can sit inside a git
# working tree depending on where mktemp points, and that guard exists to stop
# a raw rm from stranding index entries, which is irrelevant here.
run_install() {
    PLAYBOOK_SRC="$SRC" CLAUDE_HOME="$CLAUDE_DIR" HOME="$HOME_DIR" \
        bash "$REPO_ROOT/install.sh" --no-setup --skip-plugin >/dev/null 2>&1
}
run_uninstall() {
    CLAUDE_HOME="$CLAUDE_DIR" HOME="$HOME_DIR" \
        bash "$REPO_ROOT/uninstall.sh" --yes --force >/dev/null 2>&1
}

entries() { ls -A "$CLAUDE_DIR" 2>/dev/null | sort; }

# Documented as preserved by uninstall.sh's own help text. Anything else left
# behind is a leak.
PRESERVED="$(printf '%s\n' .settings.base.json backups settings.json | sort)"

run_install
install_rc=$?
after_install="$(entries)"

# Guard against a vacuous pass. If the install did nothing, every "was it
# removed" assertion below would be trivially satisfied and the suite would go
# green while testing nothing.
if [ "$install_rc" -ne 0 ]; then
    fail "install.sh exits 0" "exit $install_rc"
elif [ "$(printf '%s\n' "$after_install" | grep -c .)" -lt 10 ]; then
    fail "install.sh populates CLAUDE_HOME" "only $(printf '%s\n' "$after_install" | grep -c .) entries"
else
    pass "install.sh populates CLAUDE_HOME"
fi

run_uninstall
uninstall_rc=$?
after_uninstall="$(entries)"

if [ "$uninstall_rc" -eq 0 ]; then
    pass "uninstall.sh exits 0"
else
    fail "uninstall.sh exits 0" "exit $uninstall_rc"
fi

# The assertion that matters: nothing the installer wrote may outlive the
# uninstaller, except the entries it documents as preserved.
leaked="$(comm -23 <(printf '%s\n' "$after_uninstall" | grep -v '^$') <(printf '%s\n' "$PRESERVED"))"
if [ -z "$leaked" ]; then
    pass "uninstall removes everything install added"
else
    fail "uninstall removes everything install added" \
        "stranded: $(printf '%s' "$leaked" | tr '\n' ' ')"
fi

# And the preserved entries really are preserved, so the fix above cannot be
# "delete more aggressively until the first assertion passes".
missing=""
for keep in settings.json .settings.base.json; do
    [ -e "$CLAUDE_DIR/$keep" ] || missing="$missing $keep"
done
if [ -z "$missing" ]; then
    pass "uninstall preserves user-owned settings"
else
    fail "uninstall preserves user-owned settings" "removed:$missing"
fi

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
