#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Behavioral tests for the session-clean-exit Stop/SessionEnd hook (python port).
# Verifies it refreshes last-clean-ts every turn, writes the clean-exit marker
# only on a real (non-'other') reason, and queues an auto-learn flag when the
# session made enough edits in a repo.
#
# Run:  bash hooks/session-clean-exit.test.sh
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
HOOK="$HERE/session-clean-exit.py"

PASS=0
FAIL=0
ok()  { echo "PASS: $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL: $1"; FAIL=$((FAIL+1)); }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

# A scratch repo with an origin so the auto-learn git-root check resolves.
REPO="$WORK/proj"
mkdir -p "$REPO"
(
  cd "$REPO" || exit 1
  git init --quiet
  git config user.email test@example.com
  git config user.name "Test User"
  git config commit.gpgsign false
  git remote add origin https://github.com/acme/widget.git
) >/dev/null 2>&1

SID="cxtest"
DIR="$WORK/.claude/runtime/$SID"
mkdir -p "$DIR"

fire() { # $1=reason  $2=edit-count seed
  printf '%s' "$2" > "$DIR/edit-count"
  ( cd "$REPO" && printf '{"session_id":"%s","reason":"%s"}' "$SID" "$1" | HOME="$WORK" python3 "$HOOK" >/dev/null 2>&1 )
}

# 1. reason 'other' -> refresh ts, but NO clean-exit marker.
rm -f "$DIR/clean-exit"
fire other 9
[ -f "$DIR/last-clean-ts" ] && ok "last-clean-ts refreshed" || bad "last-clean-ts refreshed"
[ ! -f "$DIR/clean-exit" ] && ok "reason 'other' writes no clean-exit marker" || bad "reason 'other' wrote a marker"

# 2. Real reason -> clean-exit marker with the reason.
fire logout 9
[ "$(cat "$DIR/clean-exit" 2>/dev/null)" = "logout" ] && ok "clean-exit marker holds the reason" || bad "clean-exit marker (got $(cat "$DIR/clean-exit" 2>/dev/null))"

# 3. Enough edits -> auto-learn flag with the right shape.
flag="$(ls "$WORK/.claude/runtime/to-learn/"*.json 2>/dev/null | head -1)"
if [ -n "$flag" ] && python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert set(d)=={"repo_root","edits","session_id","ts"}; assert d["edits"]==9; assert d["session_id"]=="cxtest"' "$flag" 2>/dev/null; then
  ok "auto-learn flag queued with repo_root/edits/session_id/ts"
else
  bad "auto-learn flag shape"
fi

# 4. Below the edit threshold -> no flag.
WORK2="$(mktemp -d)"
DIR2="$WORK2/.claude/runtime/lo"; mkdir -p "$DIR2"; printf '2' > "$DIR2/edit-count"
( cd "$REPO" && printf '{"session_id":"lo","reason":"clear"}' | HOME="$WORK2" python3 "$HOOK" >/dev/null 2>&1 )
[ -z "$(ls "$WORK2/.claude/runtime/to-learn/"*.json 2>/dev/null)" ] && ok "below threshold queues no flag" || bad "below threshold queued a flag"
rm -rf "$WORK2"

# 5. AUTO_LEARN_NUDGE=0 disables the queue even above threshold.
WORK3="$(mktemp -d)"
DIR3="$WORK3/.claude/runtime/off"; mkdir -p "$DIR3"; printf '9' > "$DIR3/edit-count"
( cd "$REPO" && printf '{"session_id":"off","reason":"clear"}' | HOME="$WORK3" AUTO_LEARN_NUDGE=0 python3 "$HOOK" >/dev/null 2>&1 )
[ -z "$(ls "$WORK3/.claude/runtime/to-learn/"*.json 2>/dev/null)" ] && ok "AUTO_LEARN_NUDGE=0 disables the queue" || bad "AUTO_LEARN_NUDGE=0 disable"
rm -rf "$WORK3"

TOTAL=$((PASS+FAIL))
echo ""
echo "${PASS}/${TOTAL} cases passed"
[ "$FAIL" -eq 0 ]
