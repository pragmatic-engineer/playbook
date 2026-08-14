#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Behavioral tests for the auto-model-detect UserPromptSubmit hook (python port).
# Verifies it nudges on design/architecture intent and stays silent on slash
# commands, short prompts, and plain non-design prose.
#
# Run:  bash hooks/auto-model-detect.test.sh
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
HOOK="$HERE/auto-model-detect.py"

PASS=0
FAIL=0
ok()  { echo "PASS: $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL: $1"; FAIL=$((FAIL+1)); }

run() { printf '%s' "$1" | python3 "$HOOK" 2>/dev/null; }
emits() { [ -n "$(run "$1")" ]; }

# 1. Design/architecture intent -> nudge (non-empty, valid JSON).
p='{"prompt":"Should we design a new schema and evaluate the tradeoffs between the two approaches?"}'
if emits "$p" && run "$p" | python3 -c 'import sys,json; d=json.load(sys.stdin); assert d["hookSpecificOutput"]["hookEventName"]=="UserPromptSubmit"' 2>/dev/null; then
  ok "nudges on design intent with a UserPromptSubmit object"
else
  bad "nudges on design intent"
fi

# 2. Slash command -> silent.
emits '{"prompt":"/playbook:implement do the whole thing now for me please and thanks"}' && bad "slash command stays silent" || ok "slash command stays silent"

# 3. Very short prompt -> silent.
emits '{"prompt":"design?"}' && bad "short prompt stays silent" || ok "short prompt stays silent"

# 4. Plain non-design prose -> silent.
emits '{"prompt":"please rename this variable to totalCount across the whole file thanks"}' && bad "plain prose stays silent" || ok "plain prose stays silent"

# 5. Empty prompt -> silent, exit 0.
run '{"prompt":""}' >/dev/null; rc=$?
[ "$rc" -eq 0 ] && [ -z "$(run '{"prompt":""}')" ] && ok "empty prompt silent, exit 0" || bad "empty prompt silent, exit 0"

# 6. Architecture keyword variants trigger.
emits '{"prompt":"what is the best architecture for this migration and data model?"}' && ok "architecture/migration keywords trigger" || bad "architecture/migration keywords trigger"

TOTAL=$((PASS+FAIL))
echo ""
echo "${PASS}/${TOTAL} cases passed"
[ "$FAIL" -eq 0 ]
