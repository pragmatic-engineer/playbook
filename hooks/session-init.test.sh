#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Behavioral tests for the session-init SessionStart hook, project memory
# section: it prefers the graph slice from shell/memory-context.sh, falls
# back to the legacy MEMORY.md index, and must never emit malformed JSON or
# a non-zero exit, since this hook runs at the start of every session.
#
# Isolated with a fake HOME (never the user's real ~/.claude/memory/) and a
# scratch git repo carrying an origin remote, so the repo slug derivation in
# the hook resolves the same way it would in a real checkout.
#
# Run:  bash hooks/session-init.test.sh
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
HOOK="$HERE/session-init.sh"

PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

REPO_SLUG="acme/widget"
ORIGIN_URL="git@github.com:${REPO_SLUG}.git"

# A throwaway git repo with an origin remote, so `git remote get-url origin`
# resolves to $REPO_SLUG the same way it does in a real checkout. Local-only
# git identity and gpgsign=false so the init commit never touches the real
# user's global git config or signing key.
REPO_DIR="$WORK/repo"
mkdir -p "$REPO_DIR"
(
  cd "$REPO_DIR" || exit 1
  git init --quiet >/dev/null 2>&1
  git config user.email "test@example.com"
  git config user.name "Test User"
  git config commit.gpgsign false
  git remote add origin "$ORIGIN_URL"
  git commit --quiet --allow-empty -m "init" >/dev/null 2>&1
)

# A directory that is not inside any git repo, for the "not a git repo" case.
NONREPO_DIR="$WORK/not-a-repo"
mkdir -p "$NONREPO_DIR"

# run_hook <cwd> <home>: run the hook as SessionStart would, stdin '{}'
# (matches the real-run verification), stderr discarded. Sets OUT and RC.
run_hook() {
  OUT="$(cd "$1" && HOME="$2" bash "$HOOK" <<<'{}' 2>/dev/null)"
  RC=$?
}

# mem_ctx <json>: hookSpecificOutput.additionalContext, or empty.
mem_ctx() {
  printf '%s' "$1" | jq -r '.hookSpecificOutput.additionalContext // empty' 2>/dev/null
}

assert_contains() {  # <haystack> <needle> <name>
  if printf '%s' "$1" | grep -qF -- "$2"; then
    echo "PASS: $3"; PASS=$(( PASS + 1 ))
  else
    echo "FAIL: $3 (expected output to contain: $2)"; FAIL=$(( FAIL + 1 ))
  fi
}

assert_not_contains() {  # <haystack> <needle> <name>
  if printf '%s' "$1" | grep -qF -- "$2"; then
    echo "FAIL: $3 (expected output NOT to contain: $2)"; FAIL=$(( FAIL + 1 ))
  else
    echo "PASS: $3"; PASS=$(( PASS + 1 ))
  fi
}

assert_eq() {  # <actual> <expected> <name>
  if [[ "$1" == "$2" ]]; then
    echo "PASS: $3"; PASS=$(( PASS + 1 ))
  else
    echo "FAIL: $3 (expected '$2', got '$1')"; FAIL=$(( FAIL + 1 ))
  fi
}

assert_valid_json() {  # <text> <name>
  if printf '%s' "$1" | jq -e . >/dev/null 2>&1; then
    echo "PASS: $2"; PASS=$(( PASS + 1 ))
  else
    echo "FAIL: $2 (stdout did not parse as JSON: $1)"; FAIL=$(( FAIL + 1 ))
  fi
}

# 1: slice is injected.
# Arrange: a fake HOME with a graph.json carrying one fact in this repo's scope.
HOME_GRAPH="$WORK/home-graph"
mkdir -p "$HOME_GRAPH/.claude/memory"
cat > "$HOME_GRAPH/.claude/memory/graph.json" <<EOF
{
  "nodes": [
    {"id": "${REPO_SLUG}/f1", "file": "${REPO_SLUG}/f1.md", "scope": "project", "type": "project", "name": "widget-fact-one", "description": "The widget module talks to the sprocket service.", "project": "${REPO_SLUG}"}
  ],
  "edges": []
}
EOF
# Act
run_hook "$REPO_DIR" "$HOME_GRAPH"
CTX1="$(mem_ctx "$OUT")"
# Assert
assert_eq "$RC" "0" "slice injected: hook exits 0"
assert_valid_json "$OUT" "slice injected: stdout is valid JSON"
assert_contains "$CTX1" "widget-fact-one" "slice injected: additionalContext contains the fact name"

# 2: fallback to the index.
# Arrange: a fake HOME with the legacy MEMORY.md index but no graph.json.
HOME_INDEX="$WORK/home-index"
mkdir -p "$HOME_INDEX/.claude/memory/${REPO_SLUG}"
printf -- '- legacy-fact-two: an old style index entry\n' \
  > "$HOME_INDEX/.claude/memory/${REPO_SLUG}/MEMORY.md"
# Act
run_hook "$REPO_DIR" "$HOME_INDEX"
CTX2="$(mem_ctx "$OUT")"
# Assert
assert_eq "$RC" "0" "fallback to index: hook exits 0"
assert_valid_json "$OUT" "fallback to index: stdout is valid JSON"
assert_contains "$CTX2" "legacy-fact-two" "fallback to index: additionalContext contains the index line"

# 3: no store at all.
# Arrange: a fake HOME with no memory dir whatsoever.
HOME_EMPTY="$WORK/home-empty"
mkdir -p "$HOME_EMPTY"
# Act
run_hook "$REPO_DIR" "$HOME_EMPTY"
CTX3="$(mem_ctx "$OUT")"
# Assert
assert_eq "$RC" "0" "no store: hook exits 0"
assert_valid_json "$OUT" "no store: stdout is valid JSON"
assert_not_contains "$CTX3" "Project memory for this repo" "no store: no memory block emitted"

# 4: not a git repo.
# Arrange: the graph-backed HOME from scenario 1, but run from outside any
# git repo, so the slug never resolves and the slice must not be injected.
# Act
run_hook "$NONREPO_DIR" "$HOME_GRAPH"
CTX4="$(mem_ctx "$OUT")"
# Assert
assert_eq "$RC" "0" "not a git repo: hook exits 0"
assert_valid_json "$OUT" "not a git repo: stdout is valid JSON"
assert_not_contains "$CTX4" "Project memory for this repo" "not a git repo: no memory block emitted"
assert_not_contains "$CTX4" "widget-fact-one" "not a git repo: fact from the slice is absent"

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
