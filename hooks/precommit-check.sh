#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# PreToolUse(Bash) guard: mechanical sanity check on the staged diff before a commit.
#
# /playbook:commit-and-push runs `context: fork` on the `git` agent, so nothing in that
# flow ever looks at the diff it is about to commit. This guard is the mechanical
# half of that gap: it reads the staged diff and flags the obvious problems that
# do not need judgement (debug leftovers, secret-shaped files, an oversized
# commit). The semantic half lives in the engineering-standards skill, which asks
# the calling session to self-review before it delegates.
#
# Warn only, never block. A debug statement can be deliberate and a large commit
# can be a legitimate refactor; only the author knows. Emits a single
# additionalContext line. Disable with PRECOMMIT_CHECK=0.
#
# Cost note: the gate below runs on every Bash call, but guards for one event run
# in PARALLEL, so this adds no measurable wall-clock time to the event as long as
# it stays cheaper than the slowest sibling guard. Real work only happens when the
# command is a commit.
# shellcheck source=hooks/lib/common.sh
. "$(dirname "$0")/lib/common.sh"

[[ "${PRECOMMIT_CHECK:-1}" == "0" ]] && exit 0

CMD="$(hi_field '.tool_input.command')"
[[ -z "$CMD" ]] && exit 0

# Only act on a real commit. `git commit` with --amend still counts; `git log`,
# `git commit --help` and the like do not.
printf '%s' "$CMD" | grep -qE '(^|[;&|[:space:]])git([[:space:]]+-[^[:space:]]+)*[[:space:]]+commit([[:space:]]|$)' || exit 0
printf '%s' "$CMD" | grep -qE '[[:space:]]--help([[:space:]]|$)' && exit 0

# Not a git repo, or nothing staged: nothing to say.
git rev-parse --git-dir >/dev/null 2>&1 || exit 0
STAGED="$(git diff --cached --name-only 2>/dev/null)"
[[ -z "$STAGED" ]] && exit 0

FINDINGS=""

# 1. Secret-shaped files. Names only, never contents.
SECRETS="$(printf '%s\n' "$STAGED" | grep -iE '(^|/)(\.env(\..+)?|.*\.pem|.*\.p12|.*\.pfx|id_rsa|id_ed25519|.*credentials.*\.json|.*\.keystore)$' || true)"
if [[ -n "$SECRETS" ]]; then
  FINDINGS+="Secret-shaped files are staged: $(printf '%s' "$SECRETS" | tr '\n' ' '). Confirm these belong in the repo. "
fi

# 2. Debug leftovers, added lines only.
ADDED="$(git diff --cached --unified=0 2>/dev/null | grep '^+' | grep -v '^+++' || true)"
DEBUG="$(printf '%s' "$ADDED" | grep -cE '(console\.(log|debug)|debugger[[:space:]]*;|dbg!\(|breakpoint\(\)|pdb\.set_trace|binding\.pry|fmt\.Println|System\.out\.println)' || true)"
if [[ "${DEBUG:-0}" -gt 0 ]]; then
  FINDINGS+="$DEBUG added line(s) look like debug output (console.log, debugger, dbg!, breakpoint, pdb, pry, println). "
fi

# 3. Oversized commit. Small single-concern commits are the house rule.
FILE_COUNT="$(printf '%s\n' "$STAGED" | grep -c . || true)"
LINE_COUNT="$(git diff --cached --numstat 2>/dev/null | awk '{a+=$1; d+=$2} END {print a+d+0}')"
if [[ "${FILE_COUNT:-0}" -gt 20 || "${LINE_COUNT:-0}" -gt 600 ]]; then
  FINDINGS+="Large commit: ${FILE_COUNT} file(s), ${LINE_COUNT} changed line(s). Consider splitting it into single-concern commits. "
fi

if [[ -n "$FINDINGS" ]]; then
  emit_pre_context "PreToolUse" \
"Staged-diff check before this commit: ${FINDINGS}Warning only, nothing is blocked. Review the staged diff and either fix it or proceed deliberately. Disable this guard with PRECOMMIT_CHECK=0."
fi
exit 0
