#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Behavioral tests for the precompact-warn PreCompact hook (python port).
# Verifies it always emits a single valid systemMessage JSON, interpolates the
# trigger, logs a line, and caps the log at 500 entries.
#
# Run:  bash hooks/precompact-warn.test.sh
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
HOOK="$HERE/precompact-warn.py"

PASS=0
FAIL=0
ok()   { echo "PASS: $1"; PASS=$((PASS+1)); }
bad()  { echo "FAIL: $1"; FAIL=$((FAIL+1)); }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

run() { printf '%s' "$1" | HOME="$WORK" python3 "$HOOK" 2>/dev/null; }

# 1. Valid single-object JSON out for a normal payload.
out="$(run '{"trigger":"auto","session_id":"s1"}')"
if printf '%s' "$out" | python3 -c 'import sys,json; d=json.load(sys.stdin); assert "systemMessage" in d' 2>/dev/null; then
  ok "emits a valid systemMessage object"
else
  bad "emits a valid systemMessage object (got: $out)"
fi

# 2. The trigger value is interpolated into the message.
case "$out" in
  *"(auto)"*) ok "interpolates the trigger" ;;
  *) bad "interpolates the trigger (got: $out)" ;;
esac

# 3. Missing trigger falls back to 'auto' in the message, 'unknown' in the log.
out2="$(run '{"session_id":"s2"}')"
case "$out2" in *"(auto)"*) ok "missing trigger defaults to auto in message" ;; *) bad "missing trigger default" ;; esac
if grep -q 'trigger=unknown' "$WORK/.claude/runtime/compactions.log" 2>/dev/null; then
  ok "log records trigger=unknown when absent"
else
  bad "log records trigger=unknown when absent"
fi

# 4. Exit code is always 0 (never break Claude Code).
run '{}' >/dev/null; rc=$?
[ "$rc" -eq 0 ] && ok "exit 0 on empty payload" || bad "exit 0 on empty payload (rc=$rc)"

# 5. Log is capped at 500 lines.
big="$WORK/.claude/runtime/compactions.log"
mkdir -p "$(dirname "$big")"
: > "$big"; for i in $(seq 1 600); do printf 'old line %s\n' "$i" >> "$big"; done
run '{"trigger":"manual"}' >/dev/null
n="$(wc -l < "$big" | tr -d ' ')"
[ "$n" -le 500 ] && ok "log capped at 500 lines (now $n)" || bad "log capped at 500 lines (now $n)"

TOTAL=$((PASS+FAIL))
echo ""
echo "${PASS}/${TOTAL} cases passed"
[ "$FAIL" -eq 0 ]
