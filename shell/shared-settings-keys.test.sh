#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# shared-settings-keys.test.sh: pins the top-level key SET of
# settings.shared.json, the public install seed. The seed is generated from
# the maintainer's personal settings.json by denylisting a handful of keys,
# so any new key the harness adds passes straight through to every
# installer. This snapshot makes such an addition fail CI until a human
# accepts it deliberately, by name.
#
# Run:  bash shell/shared-settings-keys.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SEED="${SCRIPT_DIR}/../settings.shared.json"
PASS=0
FAIL=0

if ! command -v jq >/dev/null 2>&1; then
  echo "SKIP: jq not available; shared settings key-set tests need jq"
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1${2:+ -> $2}"; (( FAIL++ )) || true; }

# The committed key set, one per line, sorted. Any deliberate addition or
# removal in settings.shared.json must update this list in the same PR.
EXPECTED="${WORK}/expected-keys.txt"
cat > "$EXPECTED" <<'KEYS'
$schema
agentPushNotifEnabled
autoMode
autoUpdatesChannel
awaySummaryEnabled
cleanupPeriodDays
editorMode
enabledPlugins
env
feedbackSurveyRate
hooks
includeCoAuthoredBy
includeGitInstructions
inputNeededNotifEnabled
outputStyle
permissions
remoteControlAtStartup
skipAutoPermissionPrompt
skipDangerousModePermissionPrompt
spinnerTipsEnabled
statusLine
teammateMode
tui
useAutoModeDuringPlan
worktree
KEYS

# Compares the top-level key set of a settings file against EXPECTED,
# printing the keys that differ in both directions on mismatch.
assert_key_set() {
  local label="$1" file="$2"
  local actual="${WORK}/actual-keys.txt"
  jq -r 'keys_unsorted[]' "$file" | sort > "$actual"
  if diff -q "$EXPECTED" "$actual" >/dev/null 2>&1; then
    pass "$label"
  else
    local extra missing
    extra="$(comm -13 "$EXPECTED" "$actual" | tr '\n' ' ')"
    missing="$(comm -23 "$EXPECTED" "$actual" | tr '\n' ' ')"
    fail "$label" "extra=[${extra% }] missing=[${missing% }]"
  fi
}

# A: the committed seed matches the pinned key set.
assert_key_set "committed seed matches pinned key set" "$SEED"

# B: a fixture copy with an EXTRA key fails, naming the extra key.
FIXTURE_EXTRA="${WORK}/seed-extra.json"
jq '. + {"zzExtraKey": true}' "$SEED" > "$FIXTURE_EXTRA"
extra_output="$(assert_key_set "fixture with extra key" "$FIXTURE_EXTRA" 2>&1)"
if [[ "$extra_output" == FAIL:* && "$extra_output" == *"zzExtraKey"* ]]; then
  pass "extra-key drift fails and names the extra key"
else
  fail "extra-key drift fails and names the extra key" "$extra_output"
fi

# C: a fixture copy with a key REMOVED fails, naming the missing key.
FIXTURE_MISSING="${WORK}/seed-missing.json"
jq 'del(.autoMode)' "$SEED" > "$FIXTURE_MISSING"
missing_output="$(assert_key_set "fixture with removed key" "$FIXTURE_MISSING" 2>&1)"
if [[ "$missing_output" == FAIL:* && "$missing_output" == *"autoMode"* ]]; then
  pass "removed-key drift fails and names the missing key"
else
  fail "removed-key drift fails and names the missing key" "$missing_output"
fi

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ "$FAIL" -eq 0 ]]
