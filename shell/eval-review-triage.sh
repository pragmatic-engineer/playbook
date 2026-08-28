#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# eval-review-triage.sh: real, live regression eval for the `review-triage`
# classifier (agents/review-triage.md). For each fixture PR in a curated
# fixture set, fetches the PR's real diff, sends review-triage's classifier
# prompt plus that diff to a live `claude -p --model haiku` call (the same
# invocation shape shell/shared/worktree.sh already uses for its AI-resolve
# path), parses the returned per-lens tier map, and compares it against the
# fixture's ground-truth per-lens `found` fact.
#
# COST WARNING: this issues one real `gh pr diff` fetch and one real
# `claude -p --model haiku` call per fixture. It requires network access, a
# working `gh` login, and a live Claude API key, and it is NOT free. It is
# NOT part of `just check` or the shell-ci `*.test.sh` matrix (that coverage
# is shell/eval-review-triage.test.sh, which shims `claude` and runs for
# free on every PR); this script is run by hand, or on a schedule, never
# automatically.
#
# RE-RUN whenever agents/review-triage.md's classification logic changes.
# Per the review-dispatch-triage-and-cost-optimization plan: "S6 (the eval
# harness) is what actually validates classification accuracy", and
# `/playbook:deep-review`, `/playbook:implement`, and any operator relying
# on triage's skip/cheap-check decisions MUST NOT treat them as the
# validated, trusted-by-default dispatch path until this harness has run
# against the real fixture set and recorded a pass verdict. This script IS
# that gate; a run that exits non-zero means triage is not yet trustworthy
# as the default path.
#
# Usage:
#   shell/eval-review-triage.sh [fixture-file]
#   REVIEW_TRIAGE_FIXTURES=<path> shell/eval-review-triage.sh
#
#   fixture-file   defaults to shell/fixtures/review-triage-eval-set.json,
#                  relative to this script's repo root. A positional
#                  argument wins over REVIEW_TRIAGE_FIXTURES, which wins
#                  over the default.
#
# Exit status: 0 when every (fixture, lens) pair matched or was a
# non-critical mismatch; 1 when at least one critical false-negative or
# errored pair occurred, or on a usage/setup failure.

set -u

_die() {
  printf 'eval-review-triage: %s\n' "$*" >&2
  exit 1
}

_usage() {
  cat <<'EOF'
Usage: eval-review-triage.sh [fixture-file]

Runs the real review-triage classifier (a live `claude -p --model haiku`
call) against each fixture PR in fixture-file, fetches that PR's real diff
via `gh pr diff`, and compares the returned tier map against the fixture's
ground-truth per-lens `found` fact. Requires network, `gh`, `jq`, and a
working `claude` CLI with a live API key. NOT part of the default
cargo test / shell-ci matrix; run by hand.

  fixture-file   defaults to shell/fixtures/review-triage-eval-set.json.
                 Overridable via the REVIEW_TRIAGE_FIXTURES env var, or by
                 passing it as the one positional argument (wins over the
                 env var).

Exit 0: no critical false-negatives, no errored (fixture, lens) pairs.
Exit 1: at least one critical false-negative or errored pair, or setup failed.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  _usage
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
AGENT_FILE="$REPO_ROOT/agents/review-triage.md"

FIXTURE_FILE="${1:-${REVIEW_TRIAGE_FIXTURES:-$REPO_ROOT/shell/fixtures/review-triage-eval-set.json}}"

command -v jq >/dev/null 2>&1 || _die "jq is required"
command -v gh >/dev/null 2>&1 || _die "gh is required"
command -v claude >/dev/null 2>&1 || _die "claude CLI is required (needs a live API key)"

[[ -f "$FIXTURE_FILE" ]] || _die "fixture file not found: $FIXTURE_FILE"
[[ -f "$AGENT_FILE" ]] || _die "agent file not found: $AGENT_FILE"
jq empty "$FIXTURE_FILE" 2>/dev/null || _die "fixture file is not valid JSON: $FIXTURE_FILE"

# Strip the YAML frontmatter (everything between the first two '---' lines),
# leaving review-triage's classifier prompt body: the same shape the real
# review-triage agent is handed, replicated here for a scripted,
# non-Agent-tool call.
_system_prompt() {
  awk 'BEGIN { seen = 0 } /^---[[:space:]]*$/ { seen++; next } seen >= 2 { print }' "$AGENT_FILE"
}

SYSTEM_PROMPT="$(_system_prompt)"
[[ -n "$SYSTEM_PROMPT" ]] || _die "could not extract a prompt body from $AGENT_FILE"

FIXTURE_COUNT="$(jq 'length' "$FIXTURE_FILE")"
[[ "$FIXTURE_COUNT" =~ ^[0-9]+$ ]] || _die "could not read fixture count from $FIXTURE_FILE"

# Verdict counters across every (fixture, lens) pair evaluated in the run.
count_match=0
count_noncritical=0
count_critical=0
count_errored=0

# Print one indented report line for a single (fixture, lens) verdict and
# bump its counter. $1 lens name  $2 verdict  $3 human-readable detail.
_record_verdict() {
  local lens="$1" verdict="$2" detail="$3"
  case "$verdict" in
    match) count_match=$((count_match + 1)) ;;
    "non-critical mismatch") count_noncritical=$((count_noncritical + 1)) ;;
    "critical false-negative") count_critical=$((count_critical + 1)) ;;
    errored) count_errored=$((count_errored + 1)) ;;
    *) _die "internal error: unknown verdict '$verdict'" ;;
  esac
  printf '    %-14s %-26s %s\n' "$lens" "$verdict" "$detail"
}

printf 'eval-review-triage: %s fixture(s) from %s\n\n' "$FIXTURE_COUNT" "$FIXTURE_FILE"

for (( i = 0; i < FIXTURE_COUNT; i++ )); do
  fixture_json="$(jq -c ".[$i]" "$FIXTURE_FILE")"
  fixture_id="$(printf '%s' "$fixture_json" | jq -r '.id')"
  pr_number="$(printf '%s' "$fixture_json" | jq -r '.pr')"
  lens_list="$(printf '%s' "$fixture_json" | jq -r '.lenses | keys | join(", ")')"

  printf '%s (PR #%s) -- lenses: %s\n' "$fixture_id" "$pr_number" "$lens_list"

  # Step 2: fetch the real diff. A fetch failure errors every lens this
  # fixture declares and moves on; one bad fixture never aborts the run.
  diff_text=""
  if ! diff_text="$(gh pr diff "$pr_number" 2>&1)"; then
    printf '  fetch failed: %s\n' "$diff_text"
    while IFS= read -r lens; do
      [[ -n "$lens" ]] || continue
      _record_verdict "$lens" errored "gh pr diff $pr_number failed to fetch"
    done < <(printf '%s' "$fixture_json" | jq -r '.lenses | keys[]')
    printf '\n'
    continue
  fi

  # Step 3: the real, live classifier call. Same invocation shape as
  # shell/shared/worktree.sh:455's `command claude -p --model haiku "$prompt"`.
  prompt="$SYSTEM_PROMPT

Candidate lenses to classify (return a tier for every one of these, and only these): $lens_list

Diff to classify:
$diff_text"

  response="$(command claude -p --model haiku "$prompt" 2>/dev/null)"

  # Step 4: parse the tier map. A wholly unparseable response errors every
  # lens for this fixture; a parseable response missing individual lenses
  # errors only those (fixture, lens) pairs, the rest compare normally.
  tier_map=""
  if ! tier_map="$(printf '%s' "$response" | jq -c '.' 2>/dev/null)"; then
    printf '  unparseable classifier response; every lens errored\n'
    while IFS= read -r lens; do
      [[ -n "$lens" ]] || continue
      _record_verdict "$lens" errored "classifier response was not valid JSON"
    done < <(printf '%s' "$fixture_json" | jq -r '.lenses | keys[]')
    printf '\n'
    continue
  fi

  # Step 5: compare each present (fixture, lens) pair against ground truth,
  # using exactly this table (no other verdict category exists):
  #   found=true,  tier=full-lens               -> match
  #   found=true,  tier=cheap-check or skip      -> critical false-negative
  #   found=false, tier=skip                     -> match
  #   found=false, tier=cheap-check or full-lens -> non-critical mismatch
  while IFS= read -r lens; do
    [[ -n "$lens" ]] || continue
    found="$(printf '%s' "$fixture_json" | jq -r --arg l "$lens" '.lenses[$l].found')"
    tier="$(printf '%s' "$tier_map" | jq -r --arg l "$lens" 'if type == "object" and has($l) then (.[$l].tier // "") else "" end')"

    if [[ -z "$tier" ]]; then
      _record_verdict "$lens" errored "classifier response missing a tier for lens '$lens'"
      continue
    fi

    case "$found:$tier" in
      true:full-lens)
        _record_verdict "$lens" match "ground truth found=true, tier=full-lens" ;;
      true:cheap-check | true:skip)
        _record_verdict "$lens" "critical false-negative" "ground truth found=true, tier=$tier" ;;
      false:skip)
        _record_verdict "$lens" match "ground truth found=false, tier=skip" ;;
      false:cheap-check | false:full-lens)
        _record_verdict "$lens" "non-critical mismatch" "ground truth found=false, tier=$tier" ;;
      *)
        _record_verdict "$lens" errored "unrecognised tier '$tier' for lens '$lens'" ;;
    esac
  done < <(printf '%s' "$fixture_json" | jq -r '.lenses | keys[]')

  printf '\n'
done

# Step 6: overall summary. The four counts sum to the total number of
# (fixture, lens) pairs evaluated across the whole fixture set, not to the
# fixture count alone.
total=$((count_match + count_noncritical + count_critical + count_errored))

printf 'Summary: %s (fixture, lens) pair(s) evaluated across %s fixture(s)\n' "$total" "$FIXTURE_COUNT"
printf '  match:                    %s\n' "$count_match"
printf '  non-critical mismatch:    %s\n' "$count_noncritical"
printf '  critical false-negative:  %s\n' "$count_critical"
printf '  errored:                  %s\n' "$count_errored"

if (( count_critical > 0 || count_errored > 0 )); then
  printf '\nFAIL: %s critical false-negative(s) and %s errored pair(s). review-triage is NOT the validated default dispatch path until this is zero and zero.\n' "$count_critical" "$count_errored"
  exit 1
fi

printf '\nPASS: no critical false-negatives, no errored pairs.\n'
exit 0
