#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Behavioral tests for the post-edit-track PostToolUse hook (python port).
# Verifies it records {path,ts} to edits.jsonl and bumps edit-count for
# Edit/Write/NotebookEdit, honours the notebook_path fallback, and is a
# no-op for other tools.
#
# Run:  bash hooks/post-edit-track.test.sh
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
HOOK="$HERE/post-edit-track.py"

PASS=0
FAIL=0
ok()  { echo "PASS: $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL: $1"; FAIL=$((FAIL+1)); }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM
SID="pet"
DIR="$WORK/.claude/runtime/$SID"

fire() { printf '%s' "$1" | HOME="$WORK" python3 "$HOOK" >/dev/null 2>&1; }

# 1. Edit records a {path,ts} line and bumps edit-count.
fire '{"session_id":"pet","tool_name":"Edit","tool_input":{"file_path":"/tmp/a/x.txt"}}'
if [ -s "$DIR/edits.jsonl" ] && python3 -c 'import json,sys; d=json.loads(open(sys.argv[1]).read().strip().splitlines()[-1]); assert set(d)=={"path","ts"}; assert d["path"].endswith("/x.txt")' "$DIR/edits.jsonl" 2>/dev/null; then
  ok "Edit records a path/ts line"
else
  bad "Edit records a path/ts line"
fi
[ "$(cat "$DIR/edit-count" 2>/dev/null)" = "1" ] && ok "edit-count bumped to 1" || bad "edit-count (got $(cat "$DIR/edit-count" 2>/dev/null))"

# 2. Write also records (append), edit-count now 2.
fire '{"session_id":"pet","tool_name":"Write","tool_input":{"file_path":"/tmp/a/y.txt"}}'
[ "$(wc -l < "$DIR/edits.jsonl" | tr -d ' ')" = "2" ] && ok "Write appends a second line" || bad "Write appends"
[ "$(cat "$DIR/edit-count")" = "2" ] && ok "edit-count now 2" || bad "edit-count now 2"

# 3. NotebookEdit uses the notebook_path fallback.
fire '{"session_id":"pet","tool_name":"NotebookEdit","tool_input":{"notebook_path":"/tmp/a/nb.ipynb"}}'
tail -1 "$DIR/edits.jsonl" | grep -q 'nb.ipynb' && ok "NotebookEdit honours notebook_path" || bad "NotebookEdit notebook_path fallback"

# 4. Non-edit tool is a no-op (count unchanged).
before="$(cat "$DIR/edit-count")"
fire '{"session_id":"pet","tool_name":"Read","tool_input":{"file_path":"/tmp/a/x.txt"}}'
[ "$(cat "$DIR/edit-count")" = "$before" ] && ok "Read is a no-op" || bad "Read is a no-op"

# 5. No session id -> silent no-op, exit 0.
printf '{"tool_name":"Edit","tool_input":{"file_path":"/tmp/z"}}' | python3 "$HOOK" >/dev/null 2>&1
[ "$?" -eq 0 ] && ok "no session id is a silent no-op" || bad "no session id no-op"

TOTAL=$((PASS+FAIL))
echo ""
echo "${PASS}/${TOTAL} cases passed"
[ "$FAIL" -eq 0 ]
