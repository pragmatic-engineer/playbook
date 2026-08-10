#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Behavioral tests for the memory-capture Stop hook: it must fire exactly
# once per capture-due marker, list a bounded set of edited paths when
# available, and never break Stop when the marker or edit log is absent.
#
# Isolated with a fake HOME per scenario (never the real ~/.claude/runtime/),
# so no real session state is touched.
#
# Run: bash hooks/memory-capture.test.sh
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
HOOK="$HERE/memory-capture.sh"

PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

SID="test-session-abc"

# run_hook <home>: run the hook as Stop would, stdin carries the session_id
# field, which is all this hook reads from a Stop payload. Stderr discarded.
# Sets OUT and RC.
run_hook() {
  OUT="$(HOME="$1" bash "$HOOK" <<<"{\"session_id\":\"${SID}\"}" 2>/dev/null)"
  RC=$?
}

# session_dir_for <home>: matches RUNTIME_ROOT in hooks/lib/common.sh.
session_dir_for() {
  printf '%s/.claude/runtime/%s' "$1" "$SID"
}

assert_eq() {  # <actual> <expected> <name>
  if [[ "$1" == "$2" ]]; then
    echo "PASS: $3"; PASS=$(( PASS + 1 ))
  else
    echo "FAIL: $3 (expected '$2', got '$1')"; FAIL=$(( FAIL + 1 ))
  fi
}

assert_contains() {  # <haystack> <needle> <name>
  if printf '%s' "$1" | grep -qF -- "$2"; then
    echo "PASS: $3"; PASS=$(( PASS + 1 ))
  else
    echo "FAIL: $3 (expected output to contain: $2)"; FAIL=$(( FAIL + 1 ))
  fi
}

assert_valid_json() {  # <text> <name>
  if printf '%s' "$1" | jq -e . >/dev/null 2>&1; then
    echo "PASS: $2"; PASS=$(( PASS + 1 ))
  else
    echo "FAIL: $2 (stdout did not parse as JSON: $1)"; FAIL=$(( FAIL + 1 ))
  fi
}

# 1: marker present fires once, and the marker is cleared afterwards.
HOME1="$WORK/home1"
dir1="$(session_dir_for "$HOME1")"
mkdir -p "$dir1"
: > "$dir1/capture-due"
run_hook "$HOME1"
DECISION1="$(printf '%s' "$OUT" | jq -r '.decision // empty' 2>/dev/null)"
REASON1="$(printf '%s' "$OUT" | jq -r '.reason // empty' 2>/dev/null)"
assert_eq "$RC" "0" "marker present: hook exits 0"
assert_valid_json "$OUT" "marker present: stdout is valid JSON"
assert_eq "$DECISION1" "block" "marker present: decision is block"
assert_eq "$([[ -n "$REASON1" ]] && echo yes || echo no)" "yes" "marker present: reason is non empty"
assert_eq "$([[ -f "$dir1/capture-due" ]] && echo yes || echo no)" "no" "marker present: marker cleared after firing"

# 2: second call, same session, is silent because the marker was consumed.
run_hook "$HOME1"
assert_eq "$RC" "0" "second call: hook exits 0"
assert_eq "$OUT" "" "second call: no output"

# 3: no marker at all, no output.
HOME3="$WORK/home3"
dir3="$(session_dir_for "$HOME3")"
mkdir -p "$dir3"
run_hook "$HOME3"
assert_eq "$RC" "0" "no marker: hook exits 0"
assert_eq "$OUT" "" "no marker: no output"

# 4: edited paths are named in the reason.
HOME4="$WORK/home4"
dir4="$(session_dir_for "$HOME4")"
mkdir -p "$dir4"
: > "$dir4/capture-due"
printf '{"path":"/repo/src/one.sh","ts":1}\n' >> "$dir4/edits.jsonl"
printf '{"path":"/repo/src/two.sh","ts":2}\n' >> "$dir4/edits.jsonl"
run_hook "$HOME4"
REASON4="$(printf '%s' "$OUT" | jq -r '.reason // empty' 2>/dev/null)"
assert_valid_json "$OUT" "edited paths: stdout is valid JSON"
assert_contains "$REASON4" "/repo/src/one.sh" "edited paths: reason names first path"
assert_contains "$REASON4" "/repo/src/two.sh" "edited paths: reason names second path"

# 5: path list is capped when there are many edits, rather than pasting a
# long edit log into the turn.
HOME5="$WORK/home5"
dir5="$(session_dir_for "$HOME5")"
mkdir -p "$dir5"
: > "$dir5/capture-due"
i=1
while [[ $i -le 50 ]]; do
  printf '{"path":"/repo/src/file%d.sh","ts":%d}\n' "$i" "$i" >> "$dir5/edits.jsonl"
  i=$(( i + 1 ))
done
run_hook "$HOME5"
REASON5="$(printf '%s' "$OUT" | jq -r '.reason // empty' 2>/dev/null)"
LISTED5=$(printf '%s' "$REASON5" | grep -c '^- ')
assert_valid_json "$OUT" "capped list: stdout is valid JSON"
assert_eq "$([[ $LISTED5 -le 5 ]] && echo yes || echo no)" "yes" "capped list: at most a handful of paths listed"
assert_contains "$REASON5" "more" "capped list: reason notes there are more not shown"

# 6: missing edits.jsonl is harmless, still a valid block with a reason.
HOME6="$WORK/home6"
dir6="$(session_dir_for "$HOME6")"
mkdir -p "$dir6"
: > "$dir6/capture-due"
run_hook "$HOME6"
DECISION6="$(printf '%s' "$OUT" | jq -r '.decision // empty' 2>/dev/null)"
REASON6="$(printf '%s' "$OUT" | jq -r '.reason // empty' 2>/dev/null)"
assert_eq "$RC" "0" "missing edits log: hook exits 0"
assert_valid_json "$OUT" "missing edits log: stdout is valid JSON"
assert_eq "$DECISION6" "block" "missing edits log: decision is still block"
assert_eq "$([[ -n "$REASON6" ]] && echo yes || echo no)" "yes" "missing edits log: reason still non empty"

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
