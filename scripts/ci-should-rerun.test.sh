#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# ci-should-rerun.test.sh: offline tests for ci-should-rerun.sh. Every input
# is a fixture JSON file written to a temp directory.
#
# Usage: ci-should-rerun.test.sh
set -euo pipefail

cd "$(dirname "$0")/.."

SCRIPT="$PWD/scripts/ci-should-rerun.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

EXIT_RERUN=0
EXIT_SKIP=1
EXIT_USAGE=2

pass=0
fail() { echo "FAIL $*" >&2; exit 1; }

RC=0
OUT=""

# run <args...>: runs the script with stdin closed; records RC and OUT.
run() {
  RC=0
  bash "$SCRIPT" "$@" <&- >"$TMP/out" 2>&1 || RC=$?
  OUT="$(cat "$TMP/out")"
}

# expect <label> <exit code> <text the output must contain>
expect() {
  local label="$1" code="$2" text="$3"
  [ "$RC" -eq "$code" ] || fail "$label: exit $RC, want $code; output: $OUT"
  case "$OUT" in
    *"$text"*) ;;
    *) fail "$label: output lacks \"$text\": $OUT" ;;
  esac
  pass=$((pass + 1))
}

# fixture <name> <json>: writes a fixture file and prints its path.
fixture() {
  printf '%s\n' "$2" >"$TMP/$1.json"
  printf '%s' "$TMP/$1.json"
}

f="$(fixture checkout '{"jobs":[{"name":"links","conclusion":"failure","steps":[
  {"name":"Set up job","conclusion":"success"},
  {"name":"Run actions/checkout@abc","conclusion":"failure"},
  {"name":"Check links","conclusion":"skipped"}]}]}')"
run --json-file "$f"
expect "checkout failure" "$EXIT_RERUN" 'rerun links: setup step "Run actions/checkout@abc" failed'

f="$(fixture harden '{"jobs":[{"name":"spdx","conclusion":"failure","steps":[
  {"name":"Set up job","conclusion":"success"},
  {"name":"Run step-security/harden-runner@abc","conclusion":"failure"}]}]}')"
run --json-file "$f"
expect "harden-runner failure" "$EXIT_RERUN" "rerun spdx"

f="$(fixture setup '{"jobs":[{"name":"build","conclusion":"failure","steps":[
  {"name":"Set up Rust toolchain","conclusion":"failure"}]}]}')"
run --json-file "$f"
expect "setup step failure" "$EXIT_RERUN" "rerun build"

f="$(fixture timeout '{"jobs":[{"name":"review","conclusion":"failure","steps":[
  {"name":"Run actions/checkout@abc","conclusion":"success"},
  {"name":"Dependency review","conclusion":"cancelled"}]}]}')"
run --json-file "$f"
expect "timeout" "$EXIT_RERUN" "cancelled (job timeout)"

f="$(fixture nosteps '{"jobs":[{"name":"sanitize","conclusion":"failure","steps":[]}]}')"
run --json-file "$f"
expect "failed job without steps" "$EXIT_RERUN" "rerun sanitize"

f="$(fixture code '{"jobs":[{"name":"test","conclusion":"failure","steps":[
  {"name":"Run actions/checkout@abc","conclusion":"success"},
  {"name":"Run tests","conclusion":"failure"}]}]}')"
run --json-file "$f"
expect "test failure" "$EXIT_SKIP" 'skip test: step "Run tests" failed'

f="$(fixture mixed '{"jobs":[
  {"name":"links","conclusion":"failure","steps":[{"name":"Run actions/checkout@abc","conclusion":"failure"}]},
  {"name":"lint","conclusion":"failure","steps":[{"name":"Run clippy","conclusion":"failure"}]}]}')"
run --json-file "$f"
expect "mixed failures" "$EXIT_SKIP" "rerun links"
expect "mixed failures skip line" "$EXIT_SKIP" "skip lint"

f="$(fixture green '{"jobs":[{"name":"ok","conclusion":"success","steps":[]}]}')"
run --json-file "$f"
expect "no failed job" "$EXIT_SKIP" "no failed job"

f="$(fixture bad 'not json')"
run --json-file "$f"
expect "bad json" "$EXIT_USAGE" "cannot parse"

run
expect "missing argument" "$EXIT_USAGE" "--json-file is required"
run --json-file "$TMP/absent.json"
expect "missing file" "$EXIT_USAGE" "cannot read"
run --nope
expect "unknown argument" "$EXIT_USAGE" "unknown argument"

echo "ci-should-rerun.test.sh: $pass checks passed"
