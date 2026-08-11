#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# common.test.sh: tests for hooks/lib/common.py (all 11 helpers).
#
# Drives common.py by spawning python3 with inline heredocs so each test
# gets a fresh import and its own HOOK_INPUT. Requires python3 >= 3.9.
#
# Run:  bash hooks/lib/common.test.sh
# Exit: 0 if all cases pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIBDIR="$SCRIPT_DIR"

pass=0
fail=0

ok()  { printf 'PASS: %s\n' "$1"; (( pass++ )) || true; }
err() { printf 'FAIL: %s\n' "$1"; (( fail++ )) || true; }

check() {
  local label="$1" got="$2" want="$3"
  if [[ "$got" == "$want" ]]; then
    ok "$label"
  else
    err "$label (got='$got' want='$want')"
  fi
}

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT INT TERM

# ── Helper 1: field() ─────────────────────────────────────────────────────────

out="$(HOOK_INPUT='{"session_id":"abc","tool_input":{"file_path":"/tmp/x"}}' \
  PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
print(common.field('.session_id'))
PYEOF
)"
check "field: string value" "$out" "abc"

out="$(HOOK_INPUT='{"session_id":"abc","tool_input":{"file_path":"/tmp/x"}}' \
  PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
print(common.field('.tool_input.file_path'))
PYEOF
)"
check "field: nested path" "$out" "/tmp/x"

out="$(HOOK_INPUT='{"session_id":"abc"}' \
  PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
print(repr(common.field('.missing')))
PYEOF
)"
check "field: missing key returns empty string" "$out" "''"

out="$(HOOK_INPUT='{"flag":true}' \
  PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
print(common.field('.flag'))
PYEOF
)"
check "field: boolean true" "$out" "true"

out="$(HOOK_INPUT='{"n":42}' \
  PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
print(common.field('.n'))
PYEOF
)"
check "field: integer number" "$out" "42"

out="$(HOOK_INPUT='{"obj":{"k":"v"}}' \
  PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
print(common.field('.obj'))
PYEOF
)"
check "field: object returns compact JSON" "$out" '{"k":"v"}'

# ── Helper 2: session_id() ────────────────────────────────────────────────────

out="$(HOOK_INPUT='{"session_id":"sid-xyz"}' \
  PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
print(common.session_id())
PYEOF
)"
check "session_id: extracts session_id" "$out" "sid-xyz"

out="$(HOOK_INPUT='{}' \
  PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
print(repr(common.session_id()))
PYEOF
)"
check "session_id: empty when missing" "$out" "''"

# ── Helper 3: session_dir() ───────────────────────────────────────────────────

out="$(HOOK_INPUT='{}' HOME="$SCRATCH" \
  PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
print(repr(common.session_dir()))
PYEOF
)"
check "session_dir: empty when no session_id" "$out" "''"

out="$(HOOK_INPUT='{"session_id":"testsid"}' HOME="$SCRATCH" \
  PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
print(common.session_dir())
PYEOF
)"
expected_dir="$SCRATCH/.claude/runtime/testsid"
check "session_dir: returns expected path" "$out" "$expected_dir"
if [[ -d "$expected_dir" ]]; then
  ok "session_dir: created directory on demand"
else
  err "session_dir: directory was not created ($expected_dir)"
fi

# ── Helper 4: abspath() ───────────────────────────────────────────────────────

out="$(PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
print(repr(common.abspath('')))
PYEOF
)"
check "abspath: empty input returns empty string" "$out" "''"

real_scratch="$(cd "$SCRATCH" && pwd -P)"
out="$(SCRATCH="$SCRATCH" PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
print(common.abspath(os.environ['SCRATCH']))
PYEOF
)"
check "abspath: directory resolves realpath" "$out" "$real_scratch"

out="$(SCRATCH="$SCRATCH" PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
p = os.path.join(os.environ['SCRATCH'], 'leaf.txt')
print(common.abspath(p))
PYEOF
)"
check "abspath: non-existent file resolves parent + basename" "$out" "$real_scratch/leaf.txt"

# ── Helper 5: atomic_append() ─────────────────────────────────────────────────

append_file="$SCRATCH/append/test.log"
SCRATCH="$SCRATCH" PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
f = os.path.join(os.environ['SCRATCH'], 'append', 'test.log')
common.atomic_append(f, 'line one')
common.atomic_append(f, 'line two')
PYEOF
if [[ -f "$append_file" ]]; then
  got_lines="$(wc -l < "$append_file" | tr -d ' ')"
  check "atomic_append: two lines written" "$got_lines" "2"
  first_line="$(head -1 "$append_file")"
  check "atomic_append: first line content" "$first_line" "line one"
else
  err "atomic_append: file not created"
  err "atomic_append: first line content"
fi

# ── Helper 6: emit_pre_context() ─────────────────────────────────────────────

out="$(HOOK_INPUT='{}' PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
common.emit_pre_context('PreToolUse', 'hello')
PYEOF
)"
expected='{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"hello"}}'
check "emit_pre_context: exact JSON output" "$out" "$expected"

# ── Helper 7: emit_pre_deny() ─────────────────────────────────────────────────

out="$(HOOK_INPUT='{}' PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
common.emit_pre_deny('not allowed')
PYEOF
)"
expected='{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"not allowed"}}'
check "emit_pre_deny: exact JSON output" "$out" "$expected"

# ── Helper 8: emit_prompt_context() ───────────────────────────────────────────

out="$(HOOK_INPUT='{}' PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
common.emit_prompt_context('context text')
PYEOF
)"
expected='{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"context text"}}'
check "emit_prompt_context: exact JSON output" "$out" "$expected"

# ── Helper 9: emit_system_message() ───────────────────────────────────────────

out="$(HOOK_INPUT='{}' PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
common.emit_system_message('system msg')
PYEOF
)"
check "emit_system_message: exact JSON output" "$out" '{"systemMessage":"system msg"}'

# Non-ASCII must stay raw UTF-8 (ensure_ascii=False), matching jq -cn, not \uXXXX.
out="$(HOOK_INPUT='{}' PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
common.emit_system_message('⚠ warn')
PYEOF
)"
check "emit_system_message: non-ASCII stays raw UTF-8" "$out" "$(printf '{"systemMessage":"\xe2\x9a\xa0 warn"}')"

# ── Helper 10: incr_counter() ────────────────────────────────────────────────

counter_file="$SCRATCH/counter/cnt"
out="$(SCRATCH="$SCRATCH" PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
import tempfile
f = os.path.join(os.environ['SCRATCH'], 'counter', 'cnt')
os.makedirs(os.path.dirname(f), exist_ok=True)
print(common.incr_counter(f))
PYEOF
)"
check "incr_counter: missing file starts at 1" "$out" "1"

out="$(SCRATCH="$SCRATCH" PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
f = os.path.join(os.environ['SCRATCH'], 'counter', 'cnt')
print(common.incr_counter(f))
PYEOF
)"
check "incr_counter: second call returns 2" "$out" "2"

lock_dir="${counter_file}.lock"
if [[ ! -d "$lock_dir" ]]; then
  ok "incr_counter: lock directory removed after call"
else
  err "incr_counter: lock directory still exists"
fi

# ── Helper 11: repo_slug() ────────────────────────────────────────────────────

out="$(PYLIB="$LIBDIR" python3 - <<'PYEOF'
import sys, os
sys.path.insert(0, os.environ['PYLIB'])
import common
print(common.repo_slug())
PYEOF
)"
if [[ -n "$out" && "$out" == *"/"* ]]; then
  ok "repo_slug: returns owner/repo slug"
else
  err "repo_slug: expected owner/repo format, got '$out'"
fi

# ── Summary ───────────────────────────────────────────────────────────────────

total=$(( pass + fail ))
printf '\n%d/%d cases passed\n' "$pass" "$total"
[[ "$fail" -eq 0 ]]
