#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# workflow-triggers.test.sh: offline guard for privileged workflow triggers.
#
# Usage: workflow-triggers.test.sh
#
# 1. No workflow may use `pull_request_target`, except those listed in
#    ALLOWED below (empty today). That trigger runs with write access on fork
#    PRs.
# 2. A workflow with a `pull_request_target` or `issue_comment` trigger must
#    not check out the PR head, and must not put `${{ github.event` inside a
#    `run:` script (pass it through `env` instead).
#
# The check reads each workflow as text. It also runs itself on a fixture, so
# a broken check fails too.
set -euo pipefail

cd "$(dirname "$0")/.."

# Workflow file names allowed to use pull_request_target. Keep this empty
# unless a maintainer approves an exception.
ALLOWED=''

# Prints one line per problem in the workflow file $1.
problems() {
  awk -v file="$1" -v allowed="$ALLOWED" '
    function indent(s,   t) { t = s; sub(/[^ ].*$/, "", t); return length(t) }
    BEGIN { n = split(allowed, a, " "); for (i = 1; i <= n; i++) ok[a[i]] = 1 }
    { code = $0; if (code ~ /^[ \t]*#/) code = "" }
    code ~ /pull_request_target/ {
      prt = 1
      base = file; sub(/.*\//, "", base)
      if (!(base in ok)) print file ":" NR ": pull_request_target is not allowed"
    }
    code ~ /(^|[^a-z_])issue_comment([^a-z_]|$)/ { risky = 1 }
    code ~ /pull_request_target/ { risky = 1 }
    { lines[NR] = code }
    END {
      if (!risky) exit
      inrun = 0
      for (i = 1; i <= NR; i++) {
        l = lines[i]
        if (l ~ /^[ \t]*$/) continue
        if (inrun && indent(l) <= runind) inrun = 0
        if (l ~ /^[ \t-]*(run|script):/) {
          runind = indent(l); sub(/^[ \t-]*/, "", l)
          # `- run:` puts the key two columns right of the dash
          if (lines[i] ~ /^[ \t]*- /) runind += 2
          inrun = 1
        }
        if (inrun && lines[i] ~ /\$\{\{[^}]*github\.event/)
          print file ":" i ": ${{ github.event inside run: or script: (use env)"
        if (lines[i] ~ /^[ \t-]*ref:/ && lines[i] ~ /head|refs\/pull|merge/)
          print file ":" i ": checks out the PR head"
      }
    }
  ' "$1"
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
cat > "$tmp/bad.yml" <<'YAML'
on:
  pull_request_target:
    types: [opened]
jobs:
  a:
    steps:
      - uses: actions/checkout@abc # v1
        with:
          ref: ${{ github.event.pull_request.head.sha }}
      - run: echo "${{ github.event.pull_request.title }}"
      - run: |
          echo hi
          echo "${{ github.event.comment.body }}"
YAML
cat > "$tmp/bad2.yml" <<'YAML'
on: [push, issue_comment]
jobs:
  a:
    steps:
      - run: echo "see #1 ${{ github.event.issue.title }}"
      - uses: actions/github-script@abc # v1
        with:
          script: |
            const t = '${{ toJSON(github.event.comment) }}';
YAML
cat > "$tmp/good.yml" <<'YAML'
on:
  issue_comment:
    types: [created]
jobs:
  a:
    if: contains(github.event.comment.body, 'x')
    steps:
      - uses: actions/checkout@abc # v1
        with:
          ref: main
      - env:
          T: ${{ github.event.comment.body }}
        run: echo "$T"
      # run: echo ${{ github.event.x }}
YAML
want="$tmp/bad.yml:2: pull_request_target is not allowed
$tmp/bad.yml:9: checks out the PR head
$tmp/bad.yml:10: \${{ github.event inside run: or script: (use env)
$tmp/bad.yml:13: \${{ github.event inside run: or script: (use env)"
got="$(problems "$tmp/bad.yml")"
if [ "$got" != "$want" ]; then
  printf 'FAIL: the check missed the bad fixture.\nGot:\n%s\n' "$got" >&2
  exit 1
fi
want2="$tmp/bad2.yml:5: \${{ github.event inside run: or script: (use env)
$tmp/bad2.yml:9: \${{ github.event inside run: or script: (use env)"
got2="$(problems "$tmp/bad2.yml")"
if [ "$got2" != "$want2" ]; then
  printf 'FAIL: the check missed the inline-trigger fixture.\nGot:\n%s\n' "$got2" >&2
  exit 1
fi
if [ -n "$(problems "$tmp/good.yml")" ]; then
  echo "FAIL: the check flagged the good fixture: $(problems "$tmp/good.yml")" >&2
  exit 1
fi

fail=0
for f in .github/workflows/*.yml; do
  out="$(problems "$f")"
  if [ -n "$out" ]; then echo "$out" >&2; fail=1; fi
done
[ "$fail" -eq 0 ] || { echo "FAIL: privileged trigger rules broken (see above)." >&2; exit 1; }
echo "ok: no workflow uses pull_request_target, and issue_comment workflows are safe"
