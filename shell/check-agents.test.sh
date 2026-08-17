#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# check-agents.test.sh: scenarios for shell/check-agents.sh. Builds scratch
# agent fixture directories and asserts a well-formed agent passes, each
# specific contract violation fails, and _TEMPLATE.md is skipped, then
# checks the repo's own agents/ passes for real.
#
# Run:  bash shell/check-agents.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="${SCRIPT_DIR}/check-agents.sh"
REPO_ROOT="$(git -C "$SCRIPT_DIR" rev-parse --show-toplevel)"

PASS=0
FAIL=0

WORK="$(mktemp -d)"
# shellcheck disable=SC2064
trap "rm -rf '${WORK}'" EXIT INT TERM

# HOME isolation so no global gitconfig/hooks bleed into the scratch agent
# fixtures.
export HOME="${WORK}/home"
mkdir -p "$HOME"

# write_valid_agent <dir> <stem>: a fixture that satisfies every rule.
write_valid_agent() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Grep, Glob
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
2. **Ground every claim.** Quote exact code with file:line citations.
3. **Zero AI attribution.** No AI or Claude attribution anywhere.
EOF
}

# write_agent_unquoted_colon <dir> <stem>: a description holding an unquoted
# colon-space. YAML reads "prompt: a lens" as a nested mapping and rejects the
# whole document, so at runtime every frontmatter field is dropped, including
# the tools allowlist. This is the exact shape that shipped in 0.9.0 and 0.9.1.
write_agent_unquoted_colon() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent. Each spawn takes a focus from the orchestrator's prompt: a single named lens. Not for general-purpose work.
tools: Read, Grep, Glob
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
2. **Ground every claim.** Quote exact code with file:line citations.
3. **Zero AI attribution.** No AI or Claude attribution anywhere.
EOF
}

# write_agent_quoted_colon <dir> <stem>: the same description, quoted. A colon
# inside a quoted scalar is inert, so this must PASS. Without this pair the
# rule could be satisfied by banning colons outright, which would reject every
# legitimate "/playbook:deep-review" reference.
write_agent_quoted_colon() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: "A structurally read-only fixture agent. Each spawn takes a focus from the orchestrator's prompt: a single named lens. Not for general-purpose work."
tools: Read, Grep, Glob
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
2. **Ground every claim.** Quote exact code with file:line citations.
3. **Zero AI attribution.** No AI or Claude attribution anywhere.
EOF
}

# write_agent_no_model <dir> <stem>: valid frontmatter minus the model key.
write_agent_no_model() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Grep, Glob
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
EOF
}

# write_agent_strict_write_violation <dir> <stem>: strict tier, description
# says "structurally read-only", tools carries Write. Must fail.
write_agent_strict_write_violation() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Write, Glob
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
EOF
}

# write_agent_strict_bash_violation <dir> <stem>: strict tier, description
# says "structurally read-only", tools carries Bash. Must fail: the strict
# tier allows no Bash at all, unlike the loose tier.
write_agent_strict_bash_violation() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Bash, Read, Grep
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
EOF
}

# write_agent_loose_readonly_bash <dir> <stem>: loose tier, description says
# plain "read-only" (not "structurally"), tools carries Bash only. Must
# pass: the loose tier allows Bash for non-mutating shell like git log.
write_agent_loose_readonly_bash() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: An isolated read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Bash, Read, Grep
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
2. **Ground every claim.** Quote exact code with file:line citations.
3. **Zero AI attribution.** No AI or Claude attribution anywhere.
EOF
}

# write_agent_loose_readonly_bash_write <dir> <stem>: same loose-tier
# description as write_agent_loose_readonly_bash, but Write is added to
# tools alongside Bash. Must fail: the loose tier still forbids Write.
write_agent_loose_readonly_bash_write() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: An isolated read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Bash, Write, Read, Grep
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
EOF
}

# write_agent_write_capable <dir> <stem>: a write-capable agent (Edit, Write,
# Bash) whose description claims neither read-only tier. Must PASS: an agent
# that legitimately writes code is allowed to hold write tools. This pins that
# the lint does not forbid write tools outright, only when a read-only claim
# contradicts them. The implementer agent is the real example.
write_agent_write_capable() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A write-capable fixture executor used to test check-agents.sh. Holds Edit, Write, and Bash on purpose. Not for general-purpose work.
tools: Read, Grep, Glob, Edit, Write, Bash, Skill
model: sonnet
effort: high
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
2. **Ground every claim.** Quote exact code with file:line citations.
3. **Zero AI attribution.** No AI or Claude attribution anywhere.
EOF
}

# write_agent_bad_model <dir> <stem>: model tier outside the allowed set.
write_agent_bad_model() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Grep, Glob
model: gpt
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
EOF
}

# write_agent_bad_effort <dir> <stem>: effort level outside the allowed set.
write_agent_bad_effort() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Grep, Glob
model: sonnet
effort: extreme
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
EOF
}

# write_agent_missing_no_dash <dir> <stem>: guardrails heading present, no
# no-dash clause anywhere in it.
write_agent_missing_no_dash() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Grep, Glob
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **Stay in scope.** Do the work asked for and nothing more.
EOF
}

# write_agent_no_opening_delim <dir> <stem>: the file does not start with the
# opening --- frontmatter delimiter at all.
write_agent_no_opening_delim() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
This fixture has no frontmatter block, so the first line is plain prose
instead of the required opening delimiter.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
EOF
}

# write_agent_no_closing_delim <dir> <stem>: opening delimiter present, no
# closing --- delimiter anywhere after it.
write_agent_no_closing_delim() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Grep, Glob
model: sonnet
effort: medium

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
EOF
}

# write_agent_name_mismatch <dir> <stem>: the name value is set to something
# other than the filename stem.
write_agent_name_mismatch() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: not-${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Grep, Glob
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
EOF
}

# write_agent_missing_name <dir> <stem>: valid frontmatter minus the name key.
write_agent_missing_name() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Grep, Glob
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
EOF
}

# write_agent_missing_description <dir> <stem>: valid frontmatter minus the
# description key.
write_agent_missing_description() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
tools: Read, Grep, Glob
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
EOF
}

# write_agent_missing_tools <dir> <stem>: valid frontmatter minus the tools key.
write_agent_missing_tools() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
EOF
}

# write_agent_missing_guardrails_heading <dir> <stem>: valid frontmatter, the
# guardrails section is titled differently so the required heading is absent.
# The no dash clause stays elsewhere in the body so this fixture isolates the
# heading rule alone.
write_agent_missing_guardrails_heading() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Grep, Glob
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
EOF
}

# write_agent_unknown_tool <dir> <stem>: valid otherwise, tools carries a
# typo'd tool name not in the allowlist.
write_agent_unknown_tool() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Grepp, Glob
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
2. **Ground every claim.** Quote exact code with file:line citations.
3. **Zero AI attribution.** No AI or Claude attribution anywhere.
EOF
}

# write_agent_missing_grounding <dir> <stem>: guardrails heading present with
# a no-dash and an attribution clause, no grounding clause anywhere in it.
write_agent_missing_grounding() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Grep, Glob
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
2. **Zero AI attribution.** No AI or Claude attribution anywhere.
EOF
}

# write_agent_missing_attribution <dir> <stem>: guardrails heading present
# with a no-dash and a grounding clause, no attribution clause anywhere.
write_agent_missing_attribution() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Grep, Glob
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios.

## Non-negotiable guardrails

1. **No dashes in prose.** No em dashes or en dashes anywhere. Use commas, colons, or separate sentences.
2. **Ground every claim.** Quote exact code with file:line citations.
EOF
}

# write_agent_no_dash_outside_guardrails <dir> <stem>: the no-dash wording
# sits only in the intro prose, above the guardrails heading. The section
# itself carries grounding and attribution clauses but no no-dash clause.
# Pins finding 5: an unscoped match against the whole file would wrongly
# pass this fixture.
write_agent_no_dash_outside_guardrails() {
  local dir="$1" stem="$2"
  mkdir -p "$dir"
  cat > "$dir/${stem}.md" <<EOF
---
name: ${stem}
description: A structurally read-only fixture agent used to test check-agents.sh. Not for general-purpose work.
tools: Read, Grep, Glob
model: sonnet
effort: medium
---

You are a fixture agent used only for check-agents.sh test scenarios. This
intro names em dash and en dash on purpose, proving that wording outside
the guardrails section below must not satisfy the no dashes rule.

## Non-negotiable guardrails

1. **Ground every claim.** Quote exact code with file:line citations.
2. **Zero AI attribution.** No AI or Claude attribution anywhere.
EOF
}

# write_broken_template <dir>: a _TEMPLATE.md that would fail every rule if
# the check did not skip it (no frontmatter, no guardrails heading).
write_broken_template() {
  local dir="$1"
  mkdir -p "$dir"
  cat > "$dir/_TEMPLATE.md" <<'EOF'
Not a real agent. No frontmatter, no guardrails, nothing valid here at all.
EOF
}

run_scenario() {  # <expect: pass|fail> <name> <dir> [expected-stderr-pattern]
  local expect="$1" name="$2" dir="$3" pattern="${4:-}" rc out
  out="$(bash "$CHECK" "$dir" 2>&1 >/dev/null)"
  rc=$?
  if [[ "$expect" == pass ]]; then
    if [[ $rc -eq 0 ]]; then echo "PASS: $name"; (( PASS++ )) || true
    else echo "FAIL: $name (expected exit 0, got $rc)"; (( FAIL++ )) || true; fi
    return
  fi
  if [[ $rc -eq 0 ]]; then
    echo "FAIL: $name (expected non-zero, got 0)"; (( FAIL++ )) || true
    return
  fi
  if [[ -n "$pattern" ]] && ! printf '%s\n' "$out" | grep -q "$pattern"; then
    echo "FAIL: $name (expected stderr matching '$pattern', got: $out)"; (( FAIL++ )) || true
    return
  fi
  echo "PASS: $name"; (( PASS++ )) || true
}

# 1: the repo's own agents/ passes.
# Arrange: no fixture, target the real repo agents directory directly.
# Act + Assert: run_scenario invokes the check and asserts exit 0.
run_scenario pass "the real agents dir passes" "${REPO_ROOT}/agents"

# 2: a valid new agent passes.
# Arrange: scratch dir with one well-formed agent fixture.
VALID="${WORK}/valid"
write_valid_agent "$VALID" "sample"
# Act + Assert
run_scenario pass "a valid new agent passes" "$VALID"

# 2b: a write-capable agent (Edit/Write/Bash, no read-only claim) passes.
# Arrange: scratch dir with a write-capable fixture.
WRITEABLE="${WORK}/writeable"
write_agent_write_capable "$WRITEABLE" "implementer-like"
# Act + Assert
run_scenario pass "a write-capable agent passes (no read-only claim)" "$WRITEABLE"

# 3: missing frontmatter key fails.
# Arrange: scratch dir with a fixture that has no model key.
NO_MODEL="${WORK}/no-model"
write_agent_no_model "$NO_MODEL" "sample"
# Act + Assert
run_scenario fail "missing frontmatter key fails (no model)" "$NO_MODEL" "missing required frontmatter key 'model'"

# 4: strict-tier read-only violation fails (Write).
# Arrange: scratch dir, description says "structurally read-only", Write in tools.
STRICT_WRITE="${WORK}/strict-write-violation"
write_agent_strict_write_violation "$STRICT_WRITE" "sample"
# Act + Assert
run_scenario fail "strict tier read-only violation fails (Write in tools)" "$STRICT_WRITE" "or Bash, found: Write"

# 5: strict-tier read-only violation fails (Bash).
# Arrange: scratch dir, description says "structurally read-only", Bash in tools.
STRICT_BASH="${WORK}/strict-bash-violation"
write_agent_strict_bash_violation "$STRICT_BASH" "sample"
# Act + Assert
run_scenario fail "strict tier read-only violation fails (Bash in tools)" "$STRICT_BASH" "or Bash, found: Bash"

# 6: bad model tier fails.
# Arrange: scratch dir with model: gpt.
BAD_MODEL="${WORK}/bad-model"
write_agent_bad_model "$BAD_MODEL" "sample"
# Act + Assert
run_scenario fail "bad model tier fails (gpt)" "$BAD_MODEL" "'gpt' is not one of"

# 7: effort out of range fails.
# Arrange: scratch dir with effort: extreme.
BAD_EFFORT="${WORK}/bad-effort"
write_agent_bad_effort "$BAD_EFFORT" "sample"
# Act + Assert
run_scenario fail "effort out of range fails (extreme)" "$BAD_EFFORT" "'extreme' is not one of"

# 8: missing no-dash guardrail fails.
# Arrange: scratch dir with a guardrails heading but no no-dash clause.
NO_DASH_MISSING="${WORK}/no-dash-missing"
write_agent_missing_no_dash "$NO_DASH_MISSING" "sample"
# Act + Assert
run_scenario fail "missing no-dash guardrail fails" "$NO_DASH_MISSING" "missing no-dash guardrail clause"

# 9: _TEMPLATE.md is skipped.
# Arrange: scratch dir with one valid agent plus a _TEMPLATE.md that would
# fail every rule if it were not skipped.
TEMPLATE_SKIP="${WORK}/template-skip"
write_valid_agent "$TEMPLATE_SKIP" "sample"
write_broken_template "$TEMPLATE_SKIP"
# Act + Assert
run_scenario pass "_TEMPLATE.md is skipped" "$TEMPLATE_SKIP"

# 10: loose-tier read-only agent with Bash passes.
# Arrange: scratch dir, description says plain "read-only", Bash only in tools.
LOOSE_BASH="${WORK}/loose-bash-ok"
write_agent_loose_readonly_bash "$LOOSE_BASH" "sample"
# Act + Assert
run_scenario pass "loose tier read-only agent with Bash passes" "$LOOSE_BASH"

# 11: loose-tier read-only agent with Bash and Write fails.
# Arrange: scratch dir, same loose-tier description, Write added alongside Bash.
LOOSE_BASH_WRITE="${WORK}/loose-bash-write-violation"
write_agent_loose_readonly_bash_write "$LOOSE_BASH_WRITE" "sample"
# Act + Assert
run_scenario fail "loose tier read-only agent with Bash and Write fails" "$LOOSE_BASH_WRITE" "or NotebookEdit, found: Write"

# 12: missing opening --- delimiter fails.
# Arrange: scratch dir with a fixture that has no opening delimiter at all.
NO_OPEN_DELIM="${WORK}/no-open-delim"
write_agent_no_opening_delim "$NO_OPEN_DELIM" "sample"
# Act + Assert
run_scenario fail "missing opening delimiter fails" "$NO_OPEN_DELIM" "missing opening --- frontmatter delimiter"

# 13: missing closing --- delimiter fails.
# Arrange: scratch dir with an opening delimiter and no closing delimiter.
NO_CLOSE_DELIM="${WORK}/no-close-delim"
write_agent_no_closing_delim "$NO_CLOSE_DELIM" "sample"
# Act + Assert
run_scenario fail "missing closing delimiter fails" "$NO_CLOSE_DELIM" "missing closing --- frontmatter delimiter"

# 14: name value does not match the filename fails.
# Arrange: scratch dir with name set to a different value than the stem.
NAME_MISMATCH="${WORK}/name-mismatch"
write_agent_name_mismatch "$NAME_MISMATCH" "sample"
# Act + Assert
run_scenario fail "name mismatch fails" "$NAME_MISMATCH" "does not match filename"

# 15: missing name key fails.
# Arrange: scratch dir with a fixture that has no name key.
NO_NAME="${WORK}/no-name"
write_agent_missing_name "$NO_NAME" "sample"
# Act + Assert
run_scenario fail "missing frontmatter key fails (no name)" "$NO_NAME" "missing required frontmatter key 'name'"

# 16: missing description key fails.
# Arrange: scratch dir with a fixture that has no description key.
NO_DESCRIPTION="${WORK}/no-description"
write_agent_missing_description "$NO_DESCRIPTION" "sample"
# Act + Assert
run_scenario fail "missing frontmatter key fails (no description)" "$NO_DESCRIPTION" "missing required frontmatter key 'description'"

# 17: missing tools key fails.
# Arrange: scratch dir with a fixture that has no tools key.
NO_TOOLS="${WORK}/no-tools"
write_agent_missing_tools "$NO_TOOLS" "sample"
# Act + Assert
run_scenario fail "missing frontmatter key fails (no tools)" "$NO_TOOLS" "missing required frontmatter key 'tools'"

# 18: missing the Non-negotiable guardrails heading fails.
# Arrange: scratch dir with a fixture whose guardrails section is not
# titled "## Non-negotiable guardrails".
NO_GUARDRAILS_HEADING="${WORK}/no-guardrails-heading"
write_agent_missing_guardrails_heading "$NO_GUARDRAILS_HEADING" "sample"
# Act + Assert
run_scenario fail "missing guardrails heading fails" "$NO_GUARDRAILS_HEADING" "missing '## Non-negotiable guardrails' heading"

# 19: an unknown tool name fails.
# Arrange: scratch dir with a fixture whose tools list carries a typo'd name.
UNKNOWN_TOOL="${WORK}/unknown-tool"
write_agent_unknown_tool "$UNKNOWN_TOOL" "sample"
# Act + Assert
run_scenario fail "unknown tool name fails" "$UNKNOWN_TOOL" "Grepp"

# 20: a missing grounding clause fails.
# Arrange: scratch dir with a guardrails section that has no grounding clause.
MISSING_GROUNDING="${WORK}/missing-grounding"
write_agent_missing_grounding "$MISSING_GROUNDING" "sample"
# Act + Assert
run_scenario fail "missing grounding clause fails" "$MISSING_GROUNDING" "missing grounding guardrail clause"

# 21: a missing attribution clause fails.
# Arrange: scratch dir with a guardrails section that has no attribution clause.
MISSING_ATTRIBUTION="${WORK}/missing-attribution"
write_agent_missing_attribution "$MISSING_ATTRIBUTION" "sample"
# Act + Assert
run_scenario fail "missing attribution clause fails" "$MISSING_ATTRIBUTION" "missing attribution guardrail clause"

# 22: a no-dash clause outside the guardrails section fails.
# Arrange: scratch dir where the no-dash wording sits only above the
# guardrails heading. Pins finding 5's scoping.
NO_DASH_OUTSIDE="${WORK}/no-dash-outside-guardrails"
write_agent_no_dash_outside_guardrails "$NO_DASH_OUTSIDE" "sample"
# Act + Assert
run_scenario fail "no-dash clause outside guardrails section fails" "$NO_DASH_OUTSIDE" "missing no-dash guardrail clause"

# 23: an unquoted colon-space in a frontmatter value fails.
# Arrange: scratch dir whose description holds "prompt: a single named lens".
# YAML parses that as a nested mapping and rejects the document, so every
# field including tools is dropped at runtime. Four real agents shipped this
# way in 0.9.0 and 0.9.1 while this validator reported "all valid".
UNQUOTED_COLON="${WORK}/unquoted-colon"
write_agent_unquoted_colon "$UNQUOTED_COLON" "sample"
# Act + Assert
run_scenario fail "unquoted colon-space in frontmatter fails" "$UNQUOTED_COLON" "colon-space"

# 24: the same colon-space inside a QUOTED value passes.
# Arrange: identical description, wrapped in double quotes.
# Without this the rule could be satisfied by banning colons outright, which
# would reject every legitimate "/playbook:deep-review" mention in a
# description and make the validator useless.
QUOTED_COLON="${WORK}/quoted-colon"
write_agent_quoted_colon "$QUOTED_COLON" "sample"
# Act + Assert
run_scenario pass "quoted colon-space in frontmatter passes" "$QUOTED_COLON"

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
