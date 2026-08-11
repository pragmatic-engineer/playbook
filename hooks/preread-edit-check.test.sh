#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Behavioral tests for the preread-edit-check PreToolUse hook (python port).
# Verifies it nudges when the Read target was edited recently, stays silent
# when there is no match or the edit is outside the 30 minute window, and
# never blocks.
#
# Run:  bash hooks/preread-edit-check.test.sh
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
HOOK="$HERE/preread-edit-check.py"

PASS=0
FAIL=0
ok()  { echo "PASS: $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL: $1"; FAIL=$((FAIL+1)); }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM
SID="pec"
DIR="$WORK/.claude/runtime/$SID"
mkdir -p "$DIR"
NOW="$(date +%s)"

seed() { printf '{"path":"%s","ts":%s}\n' "$1" "$2" > "$DIR/edits.jsonl"; }
read_hook() { printf '{"session_id":"pec","tool_input":{"file_path":"%s"}}' "$1" | HOME="$WORK" python3 "$HOOK" 2>/dev/null; }

# 1. Recent edit (2 min ago) -> reminder mentioning the age.
seed "/tmp/x/file.py" "$((NOW-120))"
out="$(read_hook /tmp/x/file.py)"
case "$out" in *"2m ago"*) ok "recent edit nudges with age" ;; *) bad "recent edit nudge (got: $out)" ;; esac
printf '%s' "$out" | python3 -c 'import sys,json; d=json.load(sys.stdin); assert d["hookSpecificOutput"]["hookEventName"]=="PreToolUse"' 2>/dev/null \
  && ok "emits a valid PreToolUse object" || bad "valid PreToolUse object"

# 2. Edit older than the window (31 min) -> silent.
seed "/tmp/x/file.py" "$((NOW-1860))"
[ -z "$(read_hook /tmp/x/file.py)" ] && ok "edit outside window stays silent" || bad "outside window stays silent"

# 3. Different path -> silent.
seed "/tmp/x/other.py" "$((NOW-60))"
[ -z "$(read_hook /tmp/x/file.py)" ] && ok "unrelated path stays silent" || bad "unrelated path stays silent"

# 4. Seconds-scale age renders as Ns ago.
seed "/tmp/x/file.py" "$((NOW-10))"
case "$(read_hook /tmp/x/file.py)" in *"s ago"*) ok "seconds-scale age renders" ;; *) bad "seconds-scale age" ;; esac

# 5. No edits.jsonl -> silent, exit 0.
rm -f "$DIR/edits.jsonl"
read_hook /tmp/x/file.py >/dev/null; rc=$?
[ "$rc" -eq 0 ] && [ -z "$(read_hook /tmp/x/file.py)" ] && ok "no edits file is a silent no-op" || bad "no edits file no-op"

TOTAL=$((PASS+FAIL))
echo ""
echo "${PASS}/${TOTAL} cases passed"
[ "$FAIL" -eq 0 ]
