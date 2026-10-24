#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# ci-should-rerun.sh: decides whether a failed workflow run failed because of
# the CI infrastructure (a setup step could not run, or a job hit its timeout)
# and not because of the code.
#
# Usage: ci-should-rerun.sh --json-file FILE
#
# FILE holds the output of `gh run view <id> --json jobs`. The script reads no
# network and no stdin. It prints one line per failed job, "rerun <job>: <why>"
# or "skip <job>: <why>", and exits 0 when every failed job looks like an
# infrastructure failure, 1 when any failed job looks like a code failure or
# no job failed, and 2 on a usage error.
set -euo pipefail

# Job and step conclusions that count as "did not pass".
FAILED_JOB_CONCLUSIONS='failure cancelled timed_out'
FAILED_STEP_CONCLUSIONS='failure cancelled timed_out'
# The conclusion GitHub gives a step that a job timeout stopped.
TIMEOUT_STEP_CONCLUSION='cancelled'
# Step names (lowercased) that are setup work, as shell glob patterns. An
# unnamed `uses:` step is named "Run <action>@<ref>" by GitHub.
SETUP_STEP_PATTERNS='set up job|initialize containers|run step-security/harden-runner*|harden runner*|run actions/checkout*|checkout*|run actions/setup-*|set up *|run actions/cache*|run actions/download-artifact*|download*|restore cache*'
EXIT_RERUN=0
EXIT_SKIP=1
EXIT_USAGE=2

die() {
  echo "ci-should-rerun: $*" >&2
  exit "$EXIT_USAGE"
}

json_file=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --json-file)
      [ "$#" -ge 2 ] || die "--json-file needs a value"
      json_file="$2"
      shift 2
      ;;
    *) die "unknown argument: $1" ;;
  esac
done
[ -n "$json_file" ] || die "--json-file is required"
[ -r "$json_file" ] || die "cannot read $json_file"
command -v jq >/dev/null 2>&1 || die "jq is required"

# is_setup <step name>: true when the name matches a setup pattern.
is_setup() {
  local name pat
  name="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')"
  local IFS='|'
  # shellcheck disable=SC2086 # splitting on IFS is the point
  set -- $SETUP_STEP_PATTERNS
  for pat in "$@"; do
    # shellcheck disable=SC2254 # the pattern is a glob on purpose
    case "$name" in $pat) return 0 ;; esac
  done
  return 1
}

# One tab separated line per failed job: job name, then its failed steps as
# "conclusion:name" joined by a unit separator (0x1f).
unit=$'\037'
rows="$(jq -r --arg sep "$unit" --arg jc "$FAILED_JOB_CONCLUSIONS" --arg sc "$FAILED_STEP_CONCLUSIONS" '
  ($jc | split(" ")) as $jcs | ($sc | split(" ")) as $scs
  | .jobs[]
  | select(.conclusion as $c | $jcs | index($c))
  | [.name, ([.steps[]? | select(.conclusion as $c | $scs | index($c)) | "\(.conclusion):\(.name)"] | join($sep))]
  | @tsv' "$json_file")" || die "cannot parse $json_file"

if [ -z "$rows" ]; then
  echo "skip: no failed job found"
  exit "$EXIT_SKIP"
fi

verdict=$EXIT_RERUN
while IFS=$'\t' read -r job steps; do
  why=''
  code_step=''
  if [ -z "$steps" ]; then
    why='the job failed with no failed step (runner or setup)'
  else
    old_ifs="$IFS"
    IFS="$unit"
    for entry in $steps; do
      concl="${entry%%:*}"
      name="${entry#*:}"
      if [ "$concl" = "$TIMEOUT_STEP_CONCLUSION" ]; then
        why="step \"$name\" was cancelled (job timeout)"
      elif is_setup "$name"; then
        [ -n "$why" ] || why="setup step \"$name\" failed"
      else
        code_step="$name"
      fi
    done
    IFS="$old_ifs"
  fi
  if [ -n "$code_step" ]; then
    echo "skip $job: step \"$code_step\" failed"
    verdict=$EXIT_SKIP
  else
    echo "rerun $job: $why"
  fi
done <<EOF_ROWS
$rows
EOF_ROWS

exit "$verdict"
