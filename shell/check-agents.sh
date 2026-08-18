#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# check-agents.sh: validate every agents/*.md definition (excluding the
# _TEMPLATE.md skeleton) against the house agent contract: real frontmatter,
# the required keys, a name that matches the filename, an allowed model
# tier and effort level, a known tool name allowlist, a read-only tool
# allowlist, and the non-negotiable guardrail invariants every agent must
# carry (heading, no dashes, grounding, zero AI attribution), all matched
# inside the guardrails section.
#
# Run:  bash shell/check-agents.sh [AGENTS_DIR]
# Exit: 0 if every agent definition is valid, non-zero (offenders on stderr)
#       otherwise.
set -u

die() { echo "check-agents: $*" >&2; exit 1; }

AGENTS_DIR="${1:-}"
if [[ -z "$AGENTS_DIR" ]]; then
  REPO_ROOT="$(git -C . rev-parse --show-toplevel 2>/dev/null)" \
    || die "not inside a git repository and no AGENTS_DIR argument given"
  AGENTS_DIR="${REPO_ROOT}/agents"
fi
[[ -d "$AGENTS_DIR" ]] || die "agents directory not found: $AGENTS_DIR"

ALLOWED_MODELS="haiku sonnet opus"
ALLOWED_EFFORTS="low medium high xhigh max"
# Two read-only tiers, matched strictest first. "Structurally read-only"
# agents (agents/reviewer.md) hold no Bash at all. Plain "read-only" agents
# (agents/auditor.md) may hold Bash for non-mutating shell like git log.
FORBIDDEN_TOOLS_STRICT="Edit Write NotebookEdit Bash"
FORBIDDEN_TOOLS_LOOSE="Edit Write NotebookEdit"
# Every tools entry, in every agent regardless of tier, must be one of
# these. Derived from the tools agents/*.md and commands/*.md frontmatter
# actually use, plus the forbidden set above.
ALLOWED_TOOLS="Agent AskUserQuestion Bash Edit Glob Grep NotebookEdit Read Skill TodoWrite WebFetch WebSearch Write"

violations=()

add_violation() { violations+=("$1"); }

# in_words <needle> <space separated haystack>: whole word membership test.
in_words() {
  local needle="$1" haystack="$2" word
  for word in $haystack; do
    [[ "$word" == "$needle" ]] && return 0
  done
  return 1
}

# trim <string>: strip leading and trailing whitespace.
trim() {
  local value="$1"
  value="${value#"${value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  printf '%s' "$value"
}

# frontmatter_value <body> <key>: print the trimmed, unquoted value of the
# first "<key>: value" line in <body>. Returns non-zero if the key is absent.
# The one place frontmatter parsing lives, so no rule below repeats sed/grep.
frontmatter_value() {
  local body="$1" key="$2" line value
  line="$(printf '%s\n' "$body" | grep -m1 "^${key}:")" || return 1
  value="$(trim "${line#*:}")"
  if [[ "${#value}" -ge 2 && "${value:0:1}" == '"' && "${value: -1}" == '"' ]]; then
    value="${value:1:$(( ${#value} - 2 ))}"
  fi
  printf '%s' "$value"
}

# frontmatter_body <file>: print the body slice between the opening and
# closing --- frontmatter delimiters on stdout. Returns 0 on success, 1 if
# the opening delimiter is missing, 2 if the closing delimiter is missing.
# Never calls add_violation itself: callers invoke this through a command
# substitution subshell, where an array mutation would not survive back to
# the caller, so the exit status alone carries which delimiter is missing.
frontmatter_body() {
  local file="$1" first_line=""
  IFS= read -r first_line < "$file"
  [[ "$first_line" == "---" ]] || return 1
  local closing_line
  closing_line="$(awk 'NR>1 && $0=="---"{print NR; exit}' "$file")"
  [[ -n "$closing_line" ]] || return 2
  sed -n "2,$(( closing_line - 1 ))p" "$file"
}

# check_required_keys <file> <name> <body>: the five required frontmatter
# keys, the name to filename match, and the model and effort enums.
check_required_keys() {
  local file="$1" name="$2" body="$3"
  local name_value model_value effort_value

  if name_value="$(frontmatter_value "$body" name)"; then
    [[ "$name_value" == "$name" ]] \
      || add_violation "$file: name '$name_value' does not match filename '$name'"
  else
    add_violation "$file: missing required frontmatter key 'name'"
  fi

  frontmatter_value "$body" description >/dev/null \
    || add_violation "$file: missing required frontmatter key 'description'"

  frontmatter_value "$body" tools >/dev/null \
    || add_violation "$file: missing required frontmatter key 'tools'"

  if model_value="$(frontmatter_value "$body" model)"; then
    in_words "$model_value" "$ALLOWED_MODELS" \
      || add_violation "$file: model '$model_value' is not one of: $ALLOWED_MODELS"
  else
    add_violation "$file: missing required frontmatter key 'model'"
  fi

  if effort_value="$(frontmatter_value "$body" effort)"; then
    in_words "$effort_value" "$ALLOWED_EFFORTS" \
      || add_violation "$file: effort '$effort_value' is not one of: $ALLOWED_EFFORTS"
  else
    add_violation "$file: missing required frontmatter key 'effort'"
  fi
}

# check_tools <file> <body>: the tool name allowlist, which every agent's
# tools list must satisfy regardless of tier, plus the two read-only tiers
# read off the description wording.
check_tools() {
  local file="$1" body="$2"
  local description_value tools_value
  description_value="$(frontmatter_value "$body" description)"
  tools_value="$(frontmatter_value "$body" tools)"
  [[ -n "$tools_value" ]] || return 0

  local old_ifs="$IFS" tok
  IFS=','
  # shellcheck disable=SC2086
  set -- $tools_value
  IFS="$old_ifs"

  local unknown=""
  for tok in "$@"; do
    tok="$(trim "$tok")"
    in_words "$tok" "$ALLOWED_TOOLS" \
      || unknown="${unknown}${unknown:+, }${tok}"
  done
  [[ -z "$unknown" ]] \
    || add_violation "$file: tools lists unknown tool name(s) not in the allowlist: $unknown"

  [[ -n "$description_value" ]] || return 0

  local forbidden="" tier_reason=""
  if printf '%s' "$description_value" | grep -qi "structurally read-only"; then
    forbidden="$FORBIDDEN_TOOLS_STRICT"
    tier_reason="structurally read-only, tools must not include Edit, Write, NotebookEdit, or Bash"
  elif printf '%s' "$description_value" | grep -qi "read-only"; then
    forbidden="$FORBIDDEN_TOOLS_LOOSE"
    tier_reason="read-only, tools must not include Edit, Write, or NotebookEdit"
  fi
  [[ -n "$forbidden" ]] || return 0

  local offending=""
  for tok in "$@"; do
    tok="$(trim "$tok")"
    if in_words "$tok" "$forbidden"; then
      offending="${offending}${offending:+, }${tok}"
    fi
  done
  [[ -z "$offending" ]] \
    || add_violation "$file: description declares the agent $tier_reason, found: $offending"
}

# guardrails_section <file>: print the file content from the
# "## Non-negotiable guardrails" heading (inclusive) to end of file. Prints
# nothing if the heading is absent, so the section scoped rules below fail
# closed instead of matching prose anywhere else in the file.
guardrails_section() {
  local file="$1"
  awk '
    index($0, "## Non-negotiable guardrails") { found = 1 }
    found { print }
  ' "$file"
}

# check_yaml_scalars <file> <body>: reject a value real YAML would refuse.
#
# Every other rule reads values with frontmatter_value, which does "${line#*:}"
# and so is far more forgiving than a YAML parser. That gap let four agents ship
# in 0.9.0 and 0.9.1 with a description YAML rejects outright, while this
# validator reported "all valid". A bare colon is fine (/playbook:deep-review);
# colon-space is what starts a mapping, and quoting makes it a scalar again.
check_yaml_scalars() {
  local file="$1" body="$2" line key value
  while IFS= read -r line; do
    [[ "$line" =~ ^[a-zA-Z_-]+: ]] || continue
    key="${line%%:*}"
    value="$(trim "${line#*:}")"
    [[ -z "$value" ]] && continue
    # Already a quoted scalar, so the colon-space inside it is inert.
    case "$value" in
      '"'*'"' | "'"*"'") continue ;;
    esac
    if [[ "$value" == *": "* ]]; then
      add_violation "$file: frontmatter '$key' is an unquoted value containing a colon-space, which YAML parses as a nested mapping; wrap the value in double quotes"
    fi
  done <<< "$body"
}

# check_guardrails <file>: the heading, and the no-dash, grounding, and
# attribution clauses, all matched inside the guardrails section only.
check_guardrails() {
  local file="$1" section
  grep -qF '## Non-negotiable guardrails' "$file" \
    || add_violation "$file: missing '## Non-negotiable guardrails' heading"

  section="$(guardrails_section "$file")"

  printf '%s\n' "$section" | grep -Eqi 'no dashes|em dash|en dash' \
    || add_violation "$file: missing no-dash guardrail clause (no 'no dashes', 'em dash', or 'en dash' found in the guardrails section)"

  printf '%s\n' "$section" | grep -Eqi 'ground|cite|quote exact' \
    || add_violation "$file: missing grounding guardrail clause (no 'ground', 'cite', or 'quote exact' found in the guardrails section)"

  printf '%s\n' "$section" | grep -Eqi 'attribution' \
    || add_violation "$file: missing attribution guardrail clause (no 'attribution' found in the guardrails section)"
}

# check_agent <file>: run every rule against one agent definition, recording
# violations instead of stopping at the first one.
check_agent() {
  local file="$1" name
  name="$(basename "$file" .md)"

  local body rc
  body="$(frontmatter_body "$file")"
  rc=$?
  if [[ "$rc" -eq 0 ]]; then
    check_required_keys "$file" "$name" "$body"
    check_tools "$file" "$body"
    check_yaml_scalars "$file" "$body"
  elif [[ "$rc" -eq 1 ]]; then
    add_violation "$file: missing opening --- frontmatter delimiter"
  elif [[ "$rc" -eq 2 ]]; then
    add_violation "$file: missing closing --- frontmatter delimiter"
  fi

  check_guardrails "$file"
}

count=0
for file in "$AGENTS_DIR"/*.md; do
  [[ -e "$file" ]] || continue
  base="$(basename "$file")"
  [[ "$base" == "_TEMPLATE.md" ]] && continue
  count=$(( count + 1 ))
  check_agent "$file"
done

if [[ "${#violations[@]}" -gt 0 ]]; then
  {
    echo "check-agents: ${#violations[@]} violation(s) across agent definitions:"
    for v in "${violations[@]}"; do
      echo "  $v"
    done
  } >&2
  exit 1
fi

echo "check-agents: OK ($count agent definitions, all valid)"
