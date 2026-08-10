#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# memory-anchors.test.sh: scenarios for hooks/memory-anchors.sh.
# Each scenario builds its own isolated fake memory store and session dir
# under a scratch HOME, feeds the hook a synthetic Edit payload, and asserts
# on stdout. Never touches the real ~/.claude/memory store or ~/.claude/runtime.
#
# Run:  bash hooks/memory-anchors.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOOK="${HERE}/memory-anchors.sh"
RUN_BASH="${BASH:-bash}"

if ! command -v jq >/dev/null 2>&1; then
  echo "SKIP: jq not available"
  exit 0
fi

# The hook strips the git worktree root off an absolute file_path to get a
# repo-relative match key. Use the real root of this checkout so test paths
# are built the same way the real Edit tool builds them (absolute).
GIT_ROOT="$(git -C "$HERE" rev-parse --show-toplevel 2>/dev/null)"

PASS=0
FAIL=0

REAL_HOME="$HOME"
WORK="$(mktemp -d)"
trap 'HOME="$REAL_HOME"; rm -rf "$WORK"' EXIT INT TERM

STORE_N=0

# new_store: point HOME at a fresh scratch directory so both the memory
# store (HOME/.claude/memory) and the session runtime dir
# (HOME/.claude/runtime) are fully isolated from the real ones.
new_store() {
  STORE_N=$((STORE_N + 1))
  HOME="${WORK}/home-${STORE_N}"
  mkdir -p "${HOME}/.claude/memory"
  export HOME
}

# write_graph: writes stdin to HOME/.claude/memory/graph.json.
write_graph() {
  cat > "${HOME}/.claude/memory/graph.json"
}

# edit_path <repo-relative path>: builds an absolute path under the real
# checkout root, matching the shape the Edit tool actually sends.
edit_path() {
  printf '%s/%s' "$GIT_ROOT" "$1"
}

# run_hook <file_path> <session_id>: feeds a synthetic Edit payload on
# stdin and prints the hook's stdout.
run_hook() {
  local fp="$1" sid="$2"
  printf '{"session_id":"%s","tool_name":"Edit","tool_input":{"file_path":"%s"}}' "$sid" "$fp" \
    | "$RUN_BASH" "$HOOK"
}

check() {  # <expect> <actual> <label>
  if [[ "$1" == "$2" ]]; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    printf 'FAIL: %s (expected [%s], got [%s])\n' "$3" "$1" "$2" >&2
  fi
}

check_contains() {  # <haystack> <needle> <label>
  if [[ "$1" == *"$2"* ]]; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    printf 'FAIL: %s (expected to find [%s] in [%s])\n' "$3" "$2" "$1" >&2
  fi
}

check_true() {  # <condition already evaluated as 0/1 via $?> <label>
  if [[ "$1" -eq 0 ]]; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    printf 'FAIL: %s\n' "$2" >&2
  fi
}

BASE_GRAPH='{
  "nodes": [
    {"id": "global/fact-a", "file": "fact-a.md", "scope": "global", "type": "project", "name": "fact-a", "description": "Fact A describes src/a.py"},
    {"id": "global/fact-dir", "file": "fact-dir.md", "scope": "global", "type": "project", "name": "fact-dir", "description": "Fact dir describes everything under src/"},
    {"id": "global/fact-neighbour", "file": "fact-b.md", "scope": "global", "type": "project", "name": "fact-neighbour", "description": "Neighbour reached via depends_on"},
    {"id": "code:src/a.py", "file": "src/a.py", "scope": "code", "type": "code"},
    {"id": "code:src/", "file": "src/", "scope": "code", "type": "code"}
  ],
  "edges": [
    {"from": "global/fact-a", "to": "code:src/a.py", "relation": "anchors"},
    {"from": "global/fact-dir", "to": "code:src/", "relation": "anchors"},
    {"from": "global/fact-a", "to": "global/fact-neighbour", "relation": "depends_on"}
  ]
}'

# --- 1 and 3: anchored file hits, and its depends_on neighbour is named too ---
new_store
printf '%s' "$BASE_GRAPH" | write_graph
out1="$(run_hook "$(edit_path src/a.py)" "s1")"
check_contains "$out1" "fact-a" "scenario 1: anchored file names the matching fact"
check_contains "$out1" "fact-neighbour" "scenario 3: depends_on neighbour is named"

# --- 2: directory anchor hits ---
new_store
printf '%s' "$BASE_GRAPH" | write_graph
out2="$(run_hook "$(edit_path src/deep/b.py)" "s2")"
check_contains "$out2" "fact-dir" "scenario 2: directory anchor names the containing-directory fact"

# --- 4: no match is silent ---
new_store
printf '%s' "$BASE_GRAPH" | write_graph
out4="$(run_hook "$(edit_path other/unrelated.py)" "s4")"
check "" "$out4" "scenario 4: unanchored path emits nothing"

# --- 5: never blocks (malformed payload, missing file_path, missing graph) ---
new_store
printf '%s' "$BASE_GRAPH" | write_graph
out5a="$(printf 'not-json-at-all' | "$RUN_BASH" "$HOOK")"
rc5a=$?
check "0" "$rc5a" "scenario 5: malformed payload exits 0"
check "" "$out5a" "scenario 5: malformed payload emits nothing"

out5b="$(printf '{"session_id":"s5b","tool_name":"Edit","tool_input":{}}' | "$RUN_BASH" "$HOOK")"
rc5b=$?
check "0" "$rc5b" "scenario 5: missing file_path exits 0"
check "" "$out5b" "scenario 5: missing file_path emits nothing"

new_store  # fresh store, no graph.json written at all
out5c="$(run_hook "$(edit_path src/a.py)" "s5c")"
rc5c=$?
check "0" "$rc5c" "scenario 5: missing graph exits 0"
check "" "$out5c" "scenario 5: missing graph emits nothing"

# --- 6: cache is built once, reused on the second edit ---
new_store
printf '%s' "$BASE_GRAPH" | write_graph
sid6="s6"
run_hook "$(edit_path src/a.py)" "$sid6" >/dev/null
idx6="${HOME}/.claude/runtime/${sid6}/memory-anchor-index.tsv"
if [[ -s "$idx6" ]]; then
  PASS=$((PASS + 1))
else
  FAIL=$((FAIL + 1))
  printf 'FAIL: scenario 6: index file was built on first edit\n' >&2
fi
# Plant a marker in the built index. If the second edit rebuilds the index,
# the marker is wiped; if it reuses the cache, the marker survives.
printf 'MARKERLINE\n' >> "$idx6"
run_hook "$(edit_path src/dir-two.py)" "$sid6" >/dev/null
if grep -qF "MARKERLINE" "$idx6"; then
  PASS=$((PASS + 1))
else
  FAIL=$((FAIL + 1))
  printf 'FAIL: scenario 6: index was rebuilt on the second edit instead of reused\n' >&2
fi

# --- 7: stale cache behaviour is pinned (build once, accept staleness) ---
# Deliberately no directory anchor here (unlike BASE_GRAPH): a directory
# anchor would legitimately match the new file too, which would make this
# assertion ambiguous about what staleness is actually being proven.
new_store
GRAPH_BEFORE='{
  "nodes": [
    {"id": "global/fact-a", "file": "fact-a.md", "scope": "global", "type": "project", "name": "fact-a", "description": "Fact A describes src/a.py"},
    {"id": "code:src/a.py", "file": "src/a.py", "scope": "code", "type": "code"}
  ],
  "edges": [
    {"from": "global/fact-a", "to": "code:src/a.py", "relation": "anchors"}
  ]
}'
printf '%s' "$GRAPH_BEFORE" | write_graph
sid7="s7"
run_hook "$(edit_path src/a.py)" "$sid7" >/dev/null  # builds the cache
GRAPH_WITH_NEW_FACT='{
  "nodes": [
    {"id": "global/fact-a", "file": "fact-a.md", "scope": "global", "type": "project", "name": "fact-a", "description": "Fact A describes src/a.py"},
    {"id": "global/fact-new", "file": "fact-new.md", "scope": "global", "type": "project", "name": "fact-new", "description": "Added to the graph after the cache was built"},
    {"id": "code:src/a.py", "file": "src/a.py", "scope": "code", "type": "code"},
    {"id": "code:src/new-file.py", "file": "src/new-file.py", "scope": "code", "type": "code"}
  ],
  "edges": [
    {"from": "global/fact-a", "to": "code:src/a.py", "relation": "anchors"},
    {"from": "global/fact-new", "to": "code:src/new-file.py", "relation": "anchors"}
  ]
}'
printf '%s' "$GRAPH_WITH_NEW_FACT" | write_graph
out7="$(run_hook "$(edit_path src/new-file.py)" "$sid7")"
check "" "$out7" "scenario 7: fact added after cache build is not surfaced this session (pinned staleness)"

# --- 8: output is valid JSON whenever it emits anything ---
printf '%s' "$out1" | jq -e . >/dev/null 2>&1
check_true "$?" "scenario 8: additionalContext output is valid JSON"
check_contains "$out1" '"hookEventName":"PreToolUse"' "scenario 8: hookEventName is PreToolUse"

TOTAL=$((PASS + FAIL))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
