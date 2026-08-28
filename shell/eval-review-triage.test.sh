#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# eval-review-triage.test.sh: hermetic, free regression test for
# shell/eval-review-triage.sh. Shims both external processes the script
# under test calls (`gh pr diff` and `command claude -p --model haiku`) via
# a temp PATH directory, following shell/worktree.test.sh's sentinel-shim
# pattern (see its scenario_maybe_rebase_conflict, lines ~503-586). `jq` is
# NOT shimmed; the real one is used, since it is already required by this
# repo's toolchain and the script under test depends on its real behavior.
#
# This test issues ZERO live network or `claude` API calls: every `gh` and
# `claude` invocation the script under test makes is intercepted by a fake
# executable placed earlier on PATH, which prints fixed canned output and
# records itself via a sentinel/call-count file.
#
# Run:  bash shell/eval-review-triage.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SCRIPT_DIR/eval-review-triage.sh"

PASS=0
FAIL=0

run_scenario() {
  local name="$1" fn="$2"
  if "$fn" 2>&1; then echo "PASS: $name"; (( PASS++ )) || true
  else echo "FAIL: $name"; (( FAIL++ )) || true; fi
}

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

# ── shim builder ──────────────────────────────────────────────────────────
#
# mk_shims <dir> [gh_mode]
#   gh_mode "ok" (default): `gh pr diff <pr>` prints a fixed placeholder
#     diff and exits 0. gh_mode "fail": exits non-zero with a stderr message.
#   `claude -p --model haiku <prompt>` always ignores its argument, touches
#     "$dir/sentinel" every call, and prints "$dir/response_N.json" for its
#     Nth call if that file exists, else falls back to "$dir/response.json".
#     Tests write those response files before invoking the script.
mk_shims() {
  local dir="$1" gh_mode="${2:-ok}"
  mkdir -p "$dir"

  if [[ "$gh_mode" == "fail" ]]; then
    cat > "$dir/gh" <<'EOF'
#!/bin/sh
echo "gh: could not resolve pull request (simulated fetch failure)" >&2
exit 1
EOF
  else
    cat > "$dir/gh" <<'EOF'
#!/bin/sh
echo "diff --git a/placeholder.txt b/placeholder.txt"
echo "@@ -0,0 +1 @@"
echo "+placeholder diff content, never inspected by the script under test"
exit 0
EOF
  fi
  chmod +x "$dir/gh"

  cat > "$dir/claude" <<EOF
#!/bin/sh
touch "$dir/sentinel"
n_file="$dir/call_count"
n=0
[ -f "\$n_file" ] && n=\$(cat "\$n_file")
n=\$((n + 1))
echo "\$n" > "\$n_file"
per_call="$dir/response_\$n.json"
if [ -f "\$per_call" ]; then
  cat "\$per_call"
else
  cat "$dir/response.json"
fi
exit 0
EOF
  chmod +x "$dir/claude"
}

# ── assertion helpers ─────────────────────────────────────────────────────

assert_contains() {
  local haystack="$1" needle="$2"
  if ! printf '%s' "$haystack" | grep -qF -- "$needle"; then
    echo "  expected output to contain: $needle"
    return 1
  fi
}

assert_not_contains() {
  local haystack="$1" needle="$2"
  if printf '%s' "$haystack" | grep -qF -- "$needle"; then
    echo "  expected output NOT to contain: $needle"
    return 1
  fi
}

# assert_lens_line_has <output> <lens> <verdict-substring>
# Finds the report line for <lens> (the indented "<lens> <verdict> <detail>"
# line _record_verdict prints) and checks it contains <verdict-substring>.
assert_lens_line_has() {
  local output="$1" lens="$2" want="$3" line
  line="$(printf '%s\n' "$output" | grep -E "^[[:space:]]+${lens}[[:space:]]")"
  if [[ -z "$line" ]]; then
    echo "  no report line found for lens '$lens'"
    return 1
  fi
  if ! printf '%s' "$line" | grep -qF -- "$want"; then
    echo "  lens '$lens' line does not contain '$want': $line"
    return 1
  fi
}

# run_eval <shim_dir> <positional-fixture-or-empty> <env-fixture-or-empty> [extra script args...]
# Sets globals EVAL_OUT and EVAL_RC.
run_eval() {
  local shim_dir="$1" pos="$2" envfix="$3"
  shift 3
  if [[ -n "$pos" ]]; then
    EVAL_OUT="$(PATH="$shim_dir:$PATH" REVIEW_TRIAGE_FIXTURES="$envfix" bash "$SCRIPT" "$pos" "$@" 2>&1)"
  else
    EVAL_OUT="$(PATH="$shim_dir:$PATH" REVIEW_TRIAGE_FIXTURES="$envfix" bash "$SCRIPT" "$@" 2>&1)"
  fi
  EVAL_RC=$?
}

# ── scenario 1: match verdict ─────────────────────────────────────────────

scenario_match() {
  local dir fixture
  dir="$(mktemp -d -p "$SCRATCH")"
  fixture="$dir/fixture.json"

  cat > "$fixture" <<'EOF'
[
  {"id": "scn1-match", "pr": 101, "lenses": {"security": {"found": true}}}
]
EOF
  jq empty "$fixture" || { echo "  BUG: invalid fixture JSON"; return 1; }

  mk_shims "$dir" ok
  cat > "$dir/response.json" <<'EOF'
{"security": {"tier": "full-lens", "reason": "auth path touched"}}
EOF

  run_eval "$dir" "$fixture" ""

  [[ "$EVAL_RC" -eq 0 ]] || { echo "  expected exit 0, got $EVAL_RC"; echo "$EVAL_OUT"; return 1; }
  assert_lens_line_has "$EVAL_OUT" "security" "match" || return 1
  # The summary block always prints a "critical false-negative:" count
  # label; assert the count itself is zero, not the label's absence.
  assert_contains "$EVAL_OUT" "$(printf '  critical false-negative:  %s' 0)" || return 1
  assert_contains "$EVAL_OUT" "PASS:" || return 1
}

# ── scenario 2: critical false-negative ───────────────────────────────────

scenario_critical() {
  local dir fixture
  dir="$(mktemp -d -p "$SCRATCH")"
  fixture="$dir/fixture.json"

  cat > "$fixture" <<'EOF'
[
  {"id": "scn2-critical", "pr": 102, "lenses": {"security": {"found": true}}}
]
EOF
  jq empty "$fixture" || { echo "  BUG: invalid fixture JSON"; return 1; }

  mk_shims "$dir" ok
  cat > "$dir/response.json" <<'EOF'
{"security": {"tier": "skip", "reason": "nothing to see"}}
EOF

  run_eval "$dir" "$fixture" ""

  [[ "$EVAL_RC" -eq 1 ]] || { echo "  expected exit 1, got $EVAL_RC"; echo "$EVAL_OUT"; return 1; }
  assert_lens_line_has "$EVAL_OUT" "security" "critical false-negative" || return 1
  assert_contains "$EVAL_OUT" "FAIL:" || return 1
}

# ── scenario 3: non-critical mismatch alone does not fail the run ────────

scenario_noncritical() {
  local dir fixture
  dir="$(mktemp -d -p "$SCRATCH")"
  fixture="$dir/fixture.json"

  cat > "$fixture" <<'EOF'
[
  {"id": "scn3-noncritical", "pr": 103, "lenses": {"security": {"found": false}}}
]
EOF
  jq empty "$fixture" || { echo "  BUG: invalid fixture JSON"; return 1; }

  mk_shims "$dir" ok
  cat > "$dir/response.json" <<'EOF'
{"security": {"tier": "full-lens", "reason": "overcautious"}}
EOF

  run_eval "$dir" "$fixture" ""

  [[ "$EVAL_RC" -eq 0 ]] || { echo "  expected exit 0, got $EVAL_RC"; echo "$EVAL_OUT"; return 1; }
  assert_lens_line_has "$EVAL_OUT" "security" "non-critical mismatch" || return 1
  assert_contains "$EVAL_OUT" "PASS:" || return 1
}

# ── scenario 4: partial tier map, missing lens errors only that lens ─────

scenario_partial() {
  local dir fixture
  dir="$(mktemp -d -p "$SCRATCH")"
  fixture="$dir/fixture.json"

  cat > "$fixture" <<'EOF'
[
  {
    "id": "scn4-partial",
    "pr": 104,
    "lenses": {
      "security": {"found": true},
      "correctness": {"found": false}
    }
  }
]
EOF
  jq empty "$fixture" || { echo "  BUG: invalid fixture JSON"; return 1; }

  mk_shims "$dir" ok
  # Only "security" gets a tier; "correctness" is absent entirely.
  cat > "$dir/response.json" <<'EOF'
{"security": {"tier": "full-lens", "reason": "ok"}}
EOF

  run_eval "$dir" "$fixture" ""

  [[ "$EVAL_RC" -eq 1 ]] || { echo "  expected exit 1 (missing lens errors), got $EVAL_RC"; echo "$EVAL_OUT"; return 1; }
  assert_lens_line_has "$EVAL_OUT" "correctness" "errored" || return 1
  assert_lens_line_has "$EVAL_OUT" "security" "match" || return 1
  assert_contains "$EVAL_OUT" "Summary: 2 (fixture, lens) pair(s)" || return 1
}

# ── scenario 5: whole response unparseable, every declared lens errors ───

scenario_malformed() {
  local dir fixture
  dir="$(mktemp -d -p "$SCRATCH")"
  fixture="$dir/fixture.json"

  cat > "$fixture" <<'EOF'
[
  {
    "id": "scn5-malformed",
    "pr": 105,
    "lenses": {
      "security": {"found": true},
      "correctness": {"found": false}
    }
  }
]
EOF
  jq empty "$fixture" || { echo "  BUG: invalid fixture JSON"; return 1; }

  mk_shims "$dir" ok
  printf 'not valid json at all {[garbage\n' > "$dir/response.json"

  run_eval "$dir" "$fixture" ""

  [[ "$EVAL_RC" -eq 1 ]] || { echo "  expected exit 1, got $EVAL_RC"; echo "$EVAL_OUT"; return 1; }
  assert_lens_line_has "$EVAL_OUT" "security" "errored" || return 1
  assert_lens_line_has "$EVAL_OUT" "correctness" "errored" || return 1
  assert_contains "$EVAL_OUT" "classifier response was not valid JSON" || return 1
}

# ── scenario 6: gh pr diff fetch failure, claude never called ────────────

scenario_fetch_fail() {
  local dir fixture
  dir="$(mktemp -d -p "$SCRATCH")"
  fixture="$dir/fixture.json"

  # Only fixture in this run, so the sentinel check below cannot be
  # polluted by a different, legitimately-fetched fixture's claude call.
  cat > "$fixture" <<'EOF'
[
  {"id": "scn6-fetchfail", "pr": 106, "lenses": {"security": {"found": true}}}
]
EOF
  jq empty "$fixture" || { echo "  BUG: invalid fixture JSON"; return 1; }

  mk_shims "$dir" fail
  cat > "$dir/response.json" <<'EOF'
{"security": {"tier": "full-lens", "reason": "unused"}}
EOF

  run_eval "$dir" "$fixture" ""

  [[ "$EVAL_RC" -eq 1 ]] || { echo "  expected exit 1, got $EVAL_RC"; echo "$EVAL_OUT"; return 1; }
  assert_lens_line_has "$EVAL_OUT" "security" "errored" || return 1
  assert_contains "$EVAL_OUT" "failed to fetch" || return 1
  if [[ -f "$dir/sentinel" ]]; then
    echo "  claude shim WAS invoked after a gh fetch failure (should never be)"
    return 1
  fi
}

# ── scenario 7a: summary counts a match + non-critical mismatch, PASS ────

scenario_summary_pass() {
  local dir fixture line_match line_noncrit line_critical line_errored
  dir="$(mktemp -d -p "$SCRATCH")"
  fixture="$dir/fixture.json"

  cat > "$fixture" <<'EOF'
[
  {"id": "scn7a-match", "pr": 107, "lenses": {"security": {"found": true}}},
  {"id": "scn7a-noncrit", "pr": 108, "lenses": {"security": {"found": false}}}
]
EOF
  jq empty "$fixture" || { echo "  BUG: invalid fixture JSON"; return 1; }

  mk_shims "$dir" ok
  # Same "full-lens" tier for every call: found=true -> match,
  # found=false -> non-critical mismatch, per the script's own table.
  cat > "$dir/response.json" <<'EOF'
{"security": {"tier": "full-lens", "reason": "ok"}}
EOF

  run_eval "$dir" "$fixture" ""

  [[ "$EVAL_RC" -eq 0 ]] || { echo "  expected exit 0, got $EVAL_RC"; echo "$EVAL_OUT"; return 1; }

  line_match="$(printf '  match:                    %s' 1)"
  line_noncrit="$(printf '  non-critical mismatch:    %s' 1)"
  line_critical="$(printf '  critical false-negative:  %s' 0)"
  line_errored="$(printf '  errored:                  %s' 0)"

  assert_contains "$EVAL_OUT" "$line_match" || return 1
  assert_contains "$EVAL_OUT" "$line_noncrit" || return 1
  assert_contains "$EVAL_OUT" "$line_critical" || return 1
  assert_contains "$EVAL_OUT" "$line_errored" || return 1
  assert_contains "$EVAL_OUT" "PASS:" || return 1
}

# ── scenario 7b: adding one critical false-negative flips counts and FAIL ─

scenario_summary_fail() {
  local dir fixture line_match line_noncrit line_critical line_errored
  dir="$(mktemp -d -p "$SCRATCH")"
  fixture="$dir/fixture.json"

  cat > "$fixture" <<'EOF'
[
  {"id": "scn7b-match", "pr": 207, "lenses": {"security": {"found": true}}},
  {"id": "scn7b-noncrit", "pr": 208, "lenses": {"security": {"found": false}}},
  {"id": "scn7b-critical", "pr": 209, "lenses": {"security": {"found": true}}}
]
EOF
  jq empty "$fixture" || { echo "  BUG: invalid fixture JSON"; return 1; }

  mk_shims "$dir" ok
  # Default response ("full-lens") applies to calls 1 and 2 (match, then
  # non-critical mismatch). Call 3 (the critical fixture) gets its own
  # per-call override, returning "skip" against found=true.
  cat > "$dir/response.json" <<'EOF'
{"security": {"tier": "full-lens", "reason": "ok"}}
EOF
  cat > "$dir/response_3.json" <<'EOF'
{"security": {"tier": "skip", "reason": "missed it"}}
EOF

  run_eval "$dir" "$fixture" ""

  [[ "$EVAL_RC" -eq 1 ]] || { echo "  expected exit 1, got $EVAL_RC"; echo "$EVAL_OUT"; return 1; }

  line_match="$(printf '  match:                    %s' 1)"
  line_noncrit="$(printf '  non-critical mismatch:    %s' 1)"
  line_critical="$(printf '  critical false-negative:  %s' 1)"
  line_errored="$(printf '  errored:                  %s' 0)"

  assert_contains "$EVAL_OUT" "$line_match" || return 1
  assert_contains "$EVAL_OUT" "$line_noncrit" || return 1
  assert_contains "$EVAL_OUT" "$line_critical" || return 1
  assert_contains "$EVAL_OUT" "$line_errored" || return 1
  assert_contains "$EVAL_OUT" "FAIL:" || return 1
}

# ── scenario 8: -h/--help, positional vs REVIEW_TRIAGE_FIXTURES precedence ─

scenario_args() {
  local dir fixture_pos fixture_env

  # -h/--help exits 0 with usage text, before any gh/claude/jq dependency
  # check and without a fixture file: run with a PATH that has NEITHER gh
  # nor claude shimmed (only whatever real PATH provides), to prove the
  # usage path never needs them.
  EVAL_OUT="$(bash "$SCRIPT" --help 2>&1)"
  EVAL_RC=$?
  [[ "$EVAL_RC" -eq 0 ]] || { echo "  --help: expected exit 0, got $EVAL_RC"; echo "$EVAL_OUT"; return 1; }
  assert_contains "$EVAL_OUT" "Usage: eval-review-triage.sh" || return 1

  dir="$(mktemp -d -p "$SCRATCH")"
  mk_shims "$dir" ok
  cat > "$dir/response.json" <<'EOF'
{"security": {"tier": "full-lens", "reason": "ok"}}
EOF

  fixture_pos="$dir/fixture-pos.json"
  cat > "$fixture_pos" <<'EOF'
[
  {"id": "argtest-pos", "pr": 301, "lenses": {"security": {"found": true}}}
]
EOF
  fixture_env="$dir/fixture-env.json"
  cat > "$fixture_env" <<'EOF'
[
  {"id": "argtest-env", "pr": 302, "lenses": {"security": {"found": true}}}
]
EOF
  jq empty "$fixture_pos" || { echo "  BUG: invalid fixture JSON"; return 1; }
  jq empty "$fixture_env" || { echo "  BUG: invalid fixture JSON"; return 1; }

  # (a) positional argument alone (no env var set) is used.
  run_eval "$dir" "$fixture_pos" ""
  [[ "$EVAL_RC" -eq 0 ]] || { echo "  (a) expected exit 0, got $EVAL_RC"; echo "$EVAL_OUT"; return 1; }
  assert_contains "$EVAL_OUT" "argtest-pos" || return 1
  assert_not_contains "$EVAL_OUT" "argtest-env" || return 1

  # (b) REVIEW_TRIAGE_FIXTURES alone (no positional argument) is honored.
  run_eval "$dir" "" "$fixture_env"
  [[ "$EVAL_RC" -eq 0 ]] || { echo "  (b) expected exit 0, got $EVAL_RC"; echo "$EVAL_OUT"; return 1; }
  assert_contains "$EVAL_OUT" "argtest-env" || return 1
  assert_not_contains "$EVAL_OUT" "argtest-pos" || return 1

  # (c) both set: the positional argument wins over the env var.
  run_eval "$dir" "$fixture_pos" "$fixture_env"
  [[ "$EVAL_RC" -eq 0 ]] || { echo "  (c) expected exit 0, got $EVAL_RC"; echo "$EVAL_OUT"; return 1; }
  assert_contains "$EVAL_OUT" "argtest-pos" || return 1
  assert_not_contains "$EVAL_OUT" "argtest-env" || return 1
}

# ── run all scenarios ──────────────────────────────────────────────────────

run_scenario "1: match verdict"                                    scenario_match
run_scenario "2: critical false-negative"                          scenario_critical
run_scenario "3: non-critical mismatch alone still exits 0"        scenario_noncritical
run_scenario "4: partial tier map errors only the missing lens"    scenario_partial
run_scenario "5: unparseable response errors every declared lens"  scenario_malformed
run_scenario "6: gh fetch failure never invokes claude"            scenario_fetch_fail
run_scenario "7a: summary counts (match + non-critical), PASS"     scenario_summary_pass
run_scenario "7b: summary counts (+ critical), FAIL"                scenario_summary_fail
run_scenario "8: -h/--help and fixture-file precedence"            scenario_args

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ "$FAIL" -eq 0 ]]
