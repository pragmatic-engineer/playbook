#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# memory-context.test.sh: scenarios for shell/memory-context.sh. Builds small
# synthetic graph.json fixtures under a scratch dir; never reads or writes
# the user's real memory store.
#
# Run:  bash shell/memory-context.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="${SCRIPT_DIR}/memory-context.sh"

PASS=0
FAIL=0

WORK="$(mktemp -d)"
# shellcheck disable=SC2064
trap "rm -rf '${WORK}'" EXIT INT TERM

# HOME isolation so a bug that falls back to the default graph path would
# hit an empty scratch tree, never the user's real ~/.claude/memory/graph.json.
export HOME="${WORK}/home"
mkdir -p "$HOME"

# run_ctx <args...>: run memory-context.sh via bash (the script ships at
# mode 644, not executable, matching the house scripts under shell/), capture
# stdout in OUT and its exit code in RC. Stderr is discarded; no scenario
# here expects any.
run_ctx() {
  OUT="$(bash "$SCRIPT" "$@" 2>/dev/null)"
  RC=$?
}

# edges_section <text>: the slice between the "Edges:" and "Anchors:"
# markers, exclusive of both, or empty if "Edges:" is absent.
edges_section() {
  printf '%s\n' "$1" | awk '/^Edges:/{f=1;next} /^Anchors:/{f=0} f'
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

# 1: only relevant scopes appear.
# Arrange: a graph with a global fact, a repo A fact, and a repo B fact.
GRAPH1="${WORK}/scopes.json"
cat > "$GRAPH1" <<'EOF'
{
  "nodes": [
    {"id": "global/g1", "file": "g1.md", "scope": "global", "type": "user", "name": "global-fact-one", "description": "A global fact for scenario one."},
    {"id": "ownerA/repoA/a1", "file": "ownerA/repoA/a1.md", "scope": "project", "type": "project", "name": "repo-a-fact", "description": "A fact that belongs to repo A.", "project": "ownerA/repoA"},
    {"id": "ownerB/repoB/b1", "file": "ownerB/repoB/b1.md", "scope": "project", "type": "project", "name": "repo-b-fact", "description": "A fact that belongs to repo B.", "project": "ownerB/repoB"}
  ],
  "edges": []
}
EOF
# Act
run_ctx --repo ownerA/repoA --graph "$GRAPH1"
# Assert
assert_contains "$OUT" "global-fact-one" "scopes: global fact present"
assert_contains "$OUT" "repo-a-fact" "scopes: repo A fact present"
assert_not_contains "$OUT" "repo-b-fact" "scopes: repo B fact absent"

# 2: anchor index.
# Arrange: a fact anchored to src/auth/login.py via a code node.
GRAPH2="${WORK}/anchor-index.json"
cat > "$GRAPH2" <<'EOF'
{
  "nodes": [
    {"id": "global/login-fact", "file": "login-fact.md", "scope": "global", "type": "user", "name": "login-handling", "description": "How login is implemented."},
    {"id": "code:src/auth/login.py", "file": "src/auth/login.py", "scope": "code", "type": "code"}
  ],
  "edges": [
    {"from": "global/login-fact", "to": "code:src/auth/login.py", "relation": "anchors"}
  ]
}
EOF
# Act
run_ctx --repo owner/repo --graph "$GRAPH2"
# Assert
assert_contains "$OUT" "src/auth/login.py: login-handling" "anchor index: path maps to fact name"

# 3: edges included.
# Arrange: a fact that depends_on another fact.
GRAPH3="${WORK}/edges.json"
cat > "$GRAPH3" <<'EOF'
{
  "nodes": [
    {"id": "global/prereq", "file": "prereq.md", "scope": "global", "type": "user", "name": "prerequisite-fact", "description": "The fact that must come first."},
    {"id": "global/dependent", "file": "dependent.md", "scope": "global", "type": "user", "name": "dependent-fact", "description": "The fact that depends on the other."}
  ],
  "edges": [
    {"from": "global/dependent", "to": "global/prereq", "relation": "depends_on"}
  ]
}
EOF
# Act
run_ctx --repo owner/repo --graph "$GRAPH3"
# Assert
assert_contains "$OUT" "dependent-fact depends_on prerequisite-fact" "edges: prerequisite is named"

# 4: missing graph is not an error.
# Arrange: a graph path that does not exist.
GRAPH4="${WORK}/does/not/exist/graph.json"
# Act
run_ctx --repo owner/repo --graph "$GRAPH4"
# Assert
assert_eq "$RC" "0" "missing graph: exits 0"
assert_eq "$OUT" "" "missing graph: prints nothing"

# 5: unknown repo yields globals only.
# Arrange: a graph with a global fact and a fact for a repo we will not ask for.
GRAPH5="${WORK}/unknown-repo.json"
cat > "$GRAPH5" <<'EOF'
{
  "nodes": [
    {"id": "global/g1", "file": "g1.md", "scope": "global", "type": "user", "name": "global-only-fact", "description": "Visible from any repo."},
    {"id": "ownerA/repoA/a1", "file": "ownerA/repoA/a1.md", "scope": "project", "type": "project", "name": "repo-a-fact", "description": "Belongs only to repo A.", "project": "ownerA/repoA"}
  ],
  "edges": []
}
EOF
# Act
run_ctx --repo owner-with-no-facts/some-repo --graph "$GRAPH5"
# Assert
assert_contains "$OUT" "global-only-fact" "unknown repo: globals still present"
assert_not_contains "$OUT" "repo-a-fact" "unknown repo: other repo's facts absent"

# 6: anchor edges are not rendered as typed edges.
# Arrange: one fact with a depends_on edge and an anchors edge.
GRAPH6="${WORK}/anchors-not-typed.json"
cat > "$GRAPH6" <<'EOF'
{
  "nodes": [
    {"id": "global/f1", "file": "f1.md", "scope": "global", "type": "user", "name": "fact-one", "description": "First fact."},
    {"id": "global/f2", "file": "f2.md", "scope": "global", "type": "user", "name": "fact-two", "description": "Second fact, depended on."},
    {"id": "code:src/thing.py", "file": "src/thing.py", "scope": "code", "type": "code"}
  ],
  "edges": [
    {"from": "global/f1", "to": "global/f2", "relation": "depends_on"},
    {"from": "global/f1", "to": "code:src/thing.py", "relation": "anchors"}
  ]
}
EOF
# Act
run_ctx --repo owner/repo --graph "$GRAPH6"
EDGES="$(edges_section "$OUT")"
# Assert
assert_contains "$EDGES" "fact-one depends_on fact-two" "anchors split: typed edge still renders"
assert_not_contains "$EDGES" "anchors" "anchors split: anchors relation absent from typed-edges section"
assert_contains "$OUT" "src/thing.py: fact-one" "anchors split: anchor still renders in the anchor index"

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
