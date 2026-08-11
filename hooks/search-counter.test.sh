#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Behavioral tests for the search-counter PreToolUse hook (python port).
# Verifies Grep/Glob each count 1, Read counts a unique path once (dedup),
# the threshold nudge fires at 4, and the global tool counter bumps every call.
#
# Run:  bash hooks/search-counter.test.sh
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
HOOK="$HERE/search-counter.py"

PASS=0
FAIL=0
ok()  { echo "PASS: $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL: $1"; FAIL=$((FAIL+1)); }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM
SID="sctest"

fire() { printf '%s' "$1" | HOME="$WORK" python3 "$HOOK" 2>/dev/null; }
scount() { cat "$WORK/.claude/runtime/$SID/search-count" 2>/dev/null; }
tcount() { cat "$WORK/.claude/runtime/$SID/tool-count" 2>/dev/null; }

grep_payload='{"session_id":"sctest","tool_name":"Grep"}'

# 1. Four Grep calls -> search-count 4 and a nudge on the 4th.
last=""
for _ in 1 2 3 4; do last="$(fire "$grep_payload")"; done
[ "$(scount)" = "4" ] && ok "Grep bumps search-count to 4" || bad "Grep bumps search-count (got $(scount))"
case "$last" in *"has reached 4"*) ok "threshold nudge fires at 4" ;; *) bad "threshold nudge at 4 (got: $last)" ;; esac
[ "$(tcount)" = "4" ] && ok "tool-count tracks every call" || bad "tool-count (got $(tcount))"

# 2. No nudge on a non-threshold count (e.g. 5).
mid="$(fire "$grep_payload")"
[ -z "$mid" ] && ok "no nudge on count 5" || bad "no nudge on count 5 (got: $mid)"

# 3. Read dedup: same path twice bumps search-count once.
WORK2="$(mktemp -d)"
f1() { printf '{"session_id":"dd","tool_name":"Read","tool_input":{"file_path":"/etc/hosts"}}' | HOME="$WORK2" python3 "$HOOK" >/dev/null 2>&1; }
f1; f1
c="$(cat "$WORK2/.claude/runtime/dd/search-count" 2>/dev/null)"
[ "$c" = "1" ] && ok "Read of same path counts once" || bad "Read dedup (got $c)"
rm -rf "$WORK2"

# 4. No session id -> no-op, exit 0.
printf '{"tool_name":"Grep"}' | python3 "$HOOK" >/dev/null 2>&1; rc=$?
[ "$rc" -eq 0 ] && ok "no session id is a silent no-op" || bad "no session id no-op (rc=$rc)"

TOTAL=$((PASS+FAIL))
echo ""
echo "${PASS}/${TOTAL} cases passed"
[ "$FAIL" -eq 0 ]
