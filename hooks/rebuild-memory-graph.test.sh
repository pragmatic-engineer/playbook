#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# rebuild-memory-graph.test.sh: scenarios for hooks/rebuild-memory-graph.py.
# Each scenario builds its own isolated fake memory store under a scratch
# HOME, runs the hook against it, then asserts on the generated graph.json
# with jq. Never touches the real ~/.claude/memory store.
#
# Run:  bash hooks/rebuild-memory-graph.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOOK="${HERE}/rebuild-memory-graph.py"

if ! command -v jq >/dev/null 2>&1; then
  echo "SKIP: jq not available"
  exit 0
fi

PASS=0
FAIL=0

REAL_HOME="$HOME"
WORK="$(mktemp -d)"
trap 'HOME="$REAL_HOME"; rm -rf "$WORK"' EXIT INT TERM

STORE_N=0

# new_store: point HOME at a fresh scratch directory so MEMORY_DIR
# (HOME/.claude/memory) is fully isolated from the real store.
new_store() {
  STORE_N=$((STORE_N + 1))
  HOME="${WORK}/home-${STORE_N}"
  mkdir -p "${HOME}/.claude/memory"
  export HOME
}

# write_fact <relative-path-under-memory-dir>: writes stdin to that path.
write_fact() {
  local rel="$1" full
  full="${HOME}/.claude/memory/${rel}"
  mkdir -p "$(dirname "$full")"
  cat > "$full"
}

# run_hook_for <path>: <path> may be relative (resolved under the memory
# dir) or absolute (used as-is, for the outside-memory-dir scenario).
run_hook_for() {
  local target="$1" fp
  case "$target" in
    /*) fp="$target" ;;
    *)  fp="${HOME}/.claude/memory/${target}" ;;
  esac
  printf '{"tool_input":{"file_path":"%s"}}' "$fp" | python3 "$HOOK" >/dev/null 2>&1
}

GRAPH() { printf '%s' "${HOME}/.claude/memory/graph.json"; }

check() {  # <expect> <actual> <label>
  if [[ "$1" == "$2" ]]; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    printf 'FAIL: %s (expected [%s], got [%s])\n' "$3" "$1" "$2" >&2
  fi
}

check_true() {  # <jq boolean filter> <graph-file> <label>
  if jq -e "$1" "$2" >/dev/null 2>&1; then
    PASS=$((PASS + 1))
  else
    FAIL=$((FAIL + 1))
    printf 'FAIL: %s\n' "$3" >&2
  fi
}

# --- 1: scalar link produces exactly one edge with the right shape ---
new_store
write_fact "scalar-fact.md" <<'EOF'
---
name: scalar-fact
type: reference
links:
  relates_to: other-fact
---

Body text.
EOF
run_hook_for "scalar-fact.md"
g="$(GRAPH)"
check "1" "$(jq '.edges|length' "$g")" "scalar link: edge count"
check "global/scalar-fact" "$(jq -r '.edges[0].from' "$g")" "scalar link: from"
check "global/other-fact" "$(jq -r '.edges[0].to' "$g")" "scalar link: to"
check "relates_to" "$(jq -r '.edges[0].relation' "$g")" "scalar link: relation"

# --- 2: inline list [a, b, c] produces three edges, one per target ---
new_store
write_fact "list-fact.md" <<'EOF'
---
name: list-fact
type: reference
links:
  relates_to: [alpha, beta, gamma]
---

Body text.
EOF
run_hook_for "list-fact.md"
g="$(GRAPH)"
check "3" "$(jq '.edges|length' "$g")" "inline list: edge count"
check_true '.edges | any(.from=="global/list-fact" and .to=="global/alpha" and .relation=="relates_to")' "$g" "inline list: alpha edge"
check_true '.edges | any(.from=="global/list-fact" and .to=="global/beta" and .relation=="relates_to")' "$g" "inline list: beta edge"
check_true '.edges | any(.from=="global/list-fact" and .to=="global/gamma" and .relation=="relates_to")' "$g" "inline list: gamma edge"

# --- 3: single-element inline list [a] produces one edge, id has no brackets ---
new_store
write_fact "single-fact.md" <<'EOF'
---
name: single-fact
type: reference
links:
  relates_to: [solo]
---

Body text.
EOF
run_hook_for "single-fact.md"
g="$(GRAPH)"
check "1" "$(jq '.edges|length' "$g")" "single-element list: edge count"
check "global/solo" "$(jq -r '.edges[0].to' "$g")" "single-element list: target id has no brackets"

# --- 4: quoted items ["a", 'b'] parse to clean names ---
new_store
write_fact "quoted-fact.md" <<'EOF'
---
name: quoted-fact
type: reference
links:
  relates_to: ["quoted-a", 'quoted-b']
---

Body text.
EOF
run_hook_for "quoted-fact.md"
g="$(GRAPH)"
check "2" "$(jq '.edges|length' "$g")" "quoted items: edge count"
check_true '.edges | any(.to=="global/quoted-a")' "$g" "quoted items: double-quoted item clean"
check_true '.edges | any(.to=="global/quoted-b")' "$g" "quoted items: single-quoted item clean"

# --- 5: empty inline list [] produces no edges and does not crash ---
new_store
write_fact "empty-list-fact.md" <<'EOF'
---
name: empty-list-fact
type: reference
links:
  relates_to: []
---

Body text.
EOF
run_hook_for "empty-list-fact.md"
g="$(GRAPH)"
check "0" "$(jq '.edges|length' "$g")" "empty list: no edges"
check "1" "$(jq '.nodes|length' "$g")" "empty list: node still written, no crash"

# --- 6: nested block list under a relation produces one edge per item ---
new_store
write_fact "nested-fact.md" <<'EOF'
---
name: nested-fact
type: reference
links:
  relates_to:
    - item-one
    - item-two
---

Body text.
EOF
run_hook_for "nested-fact.md"
g="$(GRAPH)"
check "2" "$(jq '.edges|length' "$g")" "nested block list: edge count"
check_true '.edges | any(.to=="global/item-one" and .relation=="relates_to")' "$g" "nested block list: item-one edge"
check_true '.edges | any(.to=="global/item-two" and .relation=="relates_to")' "$g" "nested block list: item-two edge"

# --- 7: anchors still produce code: nodes and anchors edges, unchanged ---
new_store
write_fact "anchor-fact.md" <<'EOF'
---
name: anchor-fact
type: reference
anchors:
  - src/index.ts
  - src/other.ts
---

Body text.
EOF
run_hook_for "anchor-fact.md"
g="$(GRAPH)"
check "2" "$(jq '.edges|length' "$g")" "anchors: edge count"
check_true '.nodes | any(.id=="code:src/index.ts" and .type=="code")' "$g" "anchors: code node for src/index.ts"
check_true '.nodes | any(.id=="code:src/other.ts" and .type=="code")' "$g" "anchors: code node for src/other.ts"
check_true '.edges | any(.from=="global/anchor-fact" and .to=="code:src/index.ts" and .relation=="anchors")' "$g" "anchors: edge to src/index.ts"
check_true '.edges | any(.from=="global/anchor-fact" and .to=="code:src/other.ts" and .relation=="anchors")' "$g" "anchors: edge to src/other.ts"

# --- 8: a dangling target still emits its edge (surfaced, not dropped) ---
new_store
write_fact "dangling-fact.md" <<'EOF'
---
name: dangling-fact
type: reference
links:
  relates_to: does-not-exist
---

Body text.
EOF
run_hook_for "dangling-fact.md"
g="$(GRAPH)"
check "1" "$(jq '.edges|length' "$g")" "dangling target: edge still emitted"
check "global/does-not-exist" "$(jq -r '.edges[0].to' "$g")" "dangling target: to id"
check_true '(.nodes | map(.id) | index("global/does-not-exist")) == null' "$g" "dangling target: target node genuinely absent"

# --- 9: project-scoped fact gets owner/repo/name ids, global fact gets global/name ---
new_store
write_fact "acme/widget/proj-fact.md" <<'EOF'
---
name: proj-fact
type: reference
links:
  relates_to: sibling-fact
---

Body text.
EOF
write_fact "global-fact.md" <<'EOF'
---
name: global-fact
type: reference
---

Body text.
EOF
run_hook_for "acme/widget/proj-fact.md"
g="$(GRAPH)"
check_true '.nodes | any(.id=="acme/widget/proj-fact" and .project=="acme/widget")' "$g" "project scope: node id and project"
check_true '.nodes | any(.id=="global/global-fact")' "$g" "global scope: node id"
check_true '.edges | any(.from=="acme/widget/proj-fact" and .to=="acme/widget/sibling-fact")' "$g" "project scope: target id keeps owner/repo prefix"

# --- 10: writing a file outside the memory dir is a no-op, leaves graph untouched ---
new_store
write_fact "out-fact-1.md" <<'EOF'
---
name: out-fact-1
type: reference
---

Body text.
EOF
run_hook_for "out-fact-1.md"
g="$(GRAPH)"
check "1" "$(jq '.nodes|length' "$g")" "outside write: baseline graph has one node"
# Add a second fact directly on disk (not via a hook-triggering write). If the
# hook were to rebuild on the next call, this would show up as a second node.
write_fact "out-fact-2.md" <<'EOF'
---
name: out-fact-2
type: reference
---

Body text.
EOF
run_hook_for "${HOME}/outside-memory-dir.md"
check "1" "$(jq '.nodes|length' "$g")" "outside write: graph untouched, second fact not picked up"

# --- 11: malformed or absent frontmatter does not crash the hook ---
new_store
write_fact "no-frontmatter.md" <<'EOF'
Just a note with no frontmatter at all.
EOF
write_fact "malformed-frontmatter.md" <<'EOF'
---
name: malformed
This has no closing delimiter.
EOF
run_hook_for "no-frontmatter.md"
rc=$?
g="$(GRAPH)"
check "0" "$rc" "malformed frontmatter: hook exits cleanly"
check_true 'true' "$g" "malformed frontmatter: graph.json is valid JSON"
check_true '.nodes | any(.id=="global/no-frontmatter" and .type=="reference" and .name=="no-frontmatter")' "$g" "malformed frontmatter: absent-frontmatter fact falls back to defaults"
check_true '.nodes | any(.id=="global/malformed-frontmatter" and .type=="reference")' "$g" "malformed frontmatter: unclosed frontmatter falls back to defaults"

# --- 12: project fact links to a global fact, resolves cross-scope ---
new_store
write_fact "acme/widget/local-fact.md" <<'EOF'
---
name: local-fact
type: reference
links:
  relates_to: [global-thing]
---

Body text.
EOF
write_fact "global-thing.md" <<'EOF'
---
name: global-thing
type: reference
---

Body text.
EOF
run_hook_for "acme/widget/local-fact.md"
g="$(GRAPH)"
check "1" "$(jq '.edges|length' "$g")" "cross-scope resolve: edge count"
check_true '.edges | any(.from=="acme/widget/local-fact" and .to=="global/global-thing" and .relation=="relates_to")' "$g" "cross-scope resolve: project fact reaches global target"

# --- 13: own scope wins over a same-named global fact ---
new_store
write_fact "acme/widget/proj-source.md" <<'EOF'
---
name: proj-source
type: reference
links:
  relates_to: [dup]
---

Body text.
EOF
write_fact "acme/widget/dup.md" <<'EOF'
---
name: dup
type: reference
---

Body text.
EOF
write_fact "dup.md" <<'EOF'
---
name: dup
type: reference
---

Body text.
EOF
run_hook_for "acme/widget/proj-source.md"
g="$(GRAPH)"
check_true '.edges | any(.from=="acme/widget/proj-source" and .to=="acme/widget/dup" and .relation=="relates_to")' "$g" "own scope wins: edge targets project dup"
check_true '(.edges | any(.from=="acme/widget/proj-source" and .to=="global/dup")) | not' "$g" "own scope wins: edge does not fall through to global dup"

# --- 14: a project link to a target that exists nowhere still dangles ---
new_store
write_fact "acme/widget/missing-source.md" <<'EOF'
---
name: missing-source
type: reference
links:
  relates_to: nope
---

Body text.
EOF
run_hook_for "acme/widget/missing-source.md"
g="$(GRAPH)"
check "1" "$(jq '.edges|length' "$g")" "project dangling: edge still emitted"
check "acme/widget/nope" "$(jq -r '.edges[0].to' "$g")" "project dangling: same-scope id used"
check_true '(.nodes | map(.id) | index("acme/widget/nope")) == null' "$g" "project dangling: target node genuinely absent"

# --- 15: a global source resolves in global, unaffected by the two-pass rework ---
new_store
write_fact "global-source.md" <<'EOF'
---
name: global-source
type: reference
links:
  relates_to: global-target
---

Body text.
EOF
write_fact "global-target.md" <<'EOF'
---
name: global-target
type: reference
---

Body text.
EOF
run_hook_for "global-source.md"
g="$(GRAPH)"
check "1" "$(jq '.edges|length' "$g")" "global source unaffected: edge count"
check "global/global-source" "$(jq -r '.edges[0].from' "$g")" "global source unaffected: from"
check "global/global-target" "$(jq -r '.edges[0].to' "$g")" "global source unaffected: to"

# --- 16: anchors are unchanged when the same fact also carries links ---
new_store
write_fact "acme/widget/combo-fact.md" <<'EOF'
---
name: combo-fact
type: reference
links:
  relates_to: [combo-target]
anchors:
  - src/combo.ts
---

Body text.
EOF
write_fact "acme/widget/combo-target.md" <<'EOF'
---
name: combo-target
type: reference
---

Body text.
EOF
run_hook_for "acme/widget/combo-fact.md"
g="$(GRAPH)"
check "2" "$(jq '.edges|length' "$g")" "anchors regression pin: edge count"
check_true '.nodes | any(.id=="code:acme/widget/src/combo.ts" and .type=="code" and .project=="acme/widget")' "$g" "anchors regression pin: code node unaffected"
check_true '.edges | any(.from=="acme/widget/combo-fact" and .to=="code:acme/widget/src/combo.ts" and .relation=="anchors")' "$g" "anchors regression pin: anchors edge unaffected"
check_true '.edges | any(.from=="acme/widget/combo-fact" and .to=="acme/widget/combo-target" and .relation=="relates_to")' "$g" "anchors regression pin: link edge still resolves"

TOTAL=$((PASS + FAIL))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
