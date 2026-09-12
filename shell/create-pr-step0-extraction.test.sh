#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# create-pr-step0-extraction.test.sh: pins commands/create-pull-request.md's
# Step 0 sed extraction against the real skill files' current heading
# structure. Extracts the actual bash block from the command file itself
# (not a copy) so this test can never drift from what the command really
# ships; runs it for real against this repo's own skills/ tree with
# CLAUDE_PLUGIN_ROOT pointed at the repo root (skills/ lives there too, in
# this one repo, since it's the plugin's own source).
#
# Run:  bash shell/create-pr-step0-extraction.test.sh
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SCRIPT_DIR}/.."
CMD_FILE="${REPO_ROOT}/commands/create-pull-request.md"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1${2:+ -- $2}"; (( FAIL++ )) || true; }

[ -f "$CMD_FILE" ] || { echo "commands/create-pull-request.md not found" >&2; exit 2; }

# Extract the fenced bash block under "## Step 0", not any other step's
# block: isolate the Step 0 section first, then pull its one ```bash fence.
STEP0_SECTION="$(sed -n '/^## Step 0:/,/^## Step 1:/p' "$CMD_FILE")"
STEP0_BASH="$(printf '%s\n' "$STEP0_SECTION" | sed -n '/^```bash$/,/^```$/p' | sed '1d;$d')"
[ -n "$STEP0_BASH" ] || { echo "could not extract Step 0's bash block from $CMD_FILE" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

run_step0() {
  env CLAUDE_PLUGIN_ROOT="$REPO_ROOT" bash -c "$STEP0_BASH" 2>&1
}

# (A) The real Step 0 block, run against this repo's real skills/ tree,
# must succeed and print the "extracted and verified" line, not any of its
# own ERROR guards.
scenario_extracts_cleanly() {
  local out rc
  out="$(run_step0)"; rc=$?
  [ "$rc" -eq 0 ] || { echo "  Step 0 exited $rc: $out"; return 1; }
  echo "$out" | grep -q "extracted and verified" || { echo "  success line missing: $out"; return 1; }
  echo "$out" | grep -qi "ERROR" && { echo "  a guard fired unexpectedly: $out"; return 1; }
  return 0
}

# (B) Content sanity: the four extracted files (named by the block's own
# $EXTRACT_DIR, parsed from its stdout) carry the specific rules Step 0
# claims to need, so a partial-but-nonempty extraction still gets caught.
scenario_extracted_content_is_correct() {
  local out extract_dir
  out="$(run_step0)"
  extract_dir="$(echo "$out" | sed -n 's/^Skill sections extracted and verified under \(.*\)\.$/\1/p')"
  [ -n "$extract_dir" ] && [ -d "$extract_dir" ] || { echo "  could not locate the extraction dir from: $out"; return 1; }

  grep -q "IRON RULE" "$extract_dir/writing-style-core.md" || { echo "  core: missing IRON RULE"; return 1; }
  grep -q "Banned Words" "$extract_dir/writing-style-core.md" || { echo "  core: missing Banned Words"; return 1; }
  grep -q "When creating PRs" "$extract_dir/writing-style-prs.md" || { echo "  prs: missing its own heading"; return 1; }
  grep -q "Prohibited GitHub Content" "$extract_dir/writing-style-github.md" || { echo "  github: missing its own heading"; return 1; }
  grep -q "commit hashes" "$extract_dir/writing-style-github.md" || { echo "  github: missing a real rule from the section"; return 1; }
  grep -q "Readiness" "$extract_dir/eng-standards.md" || { echo "  eng: missing Readiness"; return 1; }
  grep -q "Size" "$extract_dir/eng-standards.md" || { echo "  eng: missing Size"; return 1; }
  grep -qi "Automated Testing" "$extract_dir/eng-standards.md" && { echo "  eng: ran past its intended range into Automated Testing"; return 1; }
  grep -qi "Review Comments" "$extract_dir/eng-standards.md" && { echo "  eng: end-marker heading itself leaked into the extract"; return 1; }
  rm -rf "$extract_dir"
  return 0
}

# (C) The runaway-range guard is not cosmetic: if engineering-standards.md's
# "### Review Comments" heading is renamed, the same sed command (rewritten
# here against a scratch copy, not the real file) would otherwise pull
# everything through EOF; confirm the guard's own detection string
# ("Automated Testing" present) actually appears in that failure case, the
# same check Step 0's guard performs.
scenario_runaway_range_is_detectable() {
  local es_copy renamed_extract
  es_copy="$WORK/eng-standards-renamed.md"
  sed 's/^### Review Comments$/### Comment Guidelines/' \
    "$REPO_ROOT/skills/engineering-standards/SKILL.md" > "$es_copy"

  renamed_extract="$WORK/eng-renamed-extract.md"
  sed -n '/^### Readiness/,/^### Review Comments/p' "$es_copy" | sed '$d' > "$renamed_extract"

  grep -q "Automated Testing" "$renamed_extract" \
    || { echo "  expected the renamed-heading case to run away into Automated Testing, it didn't: $(wc -l < "$renamed_extract") lines extracted"; return 1; }
  return 0
}

run_scenario() {
  local name="$1" fn="$2"
  if "$fn"; then pass "$name"; else fail "$name"; fi
}

run_scenario "A: Step 0's real bash block extracts cleanly against this repo's skills" scenario_extracts_cleanly
run_scenario "B: extracted content carries the specific rules Step 0 claims to need" scenario_extracted_content_is_correct
run_scenario "C: a renamed end-marker heading produces a detectable runaway extraction" scenario_runaway_range_is_detectable

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
