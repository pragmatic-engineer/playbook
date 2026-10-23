#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# check-help-lint-shrink.sh: fails a change that grows the help lint allowlists,
# edits golden/flags.txt or edits the lint rule block without the matching flag.
#
# Usage: check-help-lint-shrink.sh [--allow-flags-change] [--allow-rules-change]
#
# In CI (env CI set) the base is HEAD^1, the main tip of a pull_request merge
# commit, so the checkout needs fetch-depth: 2. Elsewhere the base is the merge
# base with origin/main.
#
# The workflow runs a copy of this file from $RUNNER_TEMP, so the repository
# root comes from git and never from the path of the script.
set -euo pipefail

ALLOW_DIR=crates/cli/tests/help_lint_allowlist
FLAGS=crates/cli/tests/golden/flags.txt
LINT=crates/cli/tests/help_lint.rs
RULES_BEGIN='// RULES-BEGIN'
RULES_END='// RULES-END'

allow_flags=0
allow_rules=0
for arg in "$@"; do
  case "$arg" in
    --allow-flags-change) allow_flags=1 ;;
    --allow-rules-change) allow_rules=1 ;;
    *)
      echo "check-help-lint-shrink: unknown argument: $arg" >&2
      exit 2
      ;;
  esac
done

problem() { echo "check-help-lint-shrink: $*" >&2; }

cd "$(git rev-parse --show-toplevel)"

if [ -n "${CI:-}" ]; then
  if ! base="$(git rev-parse -q --verify 'HEAD^1')"; then
    problem "HEAD^1 does not resolve; the checkout needs fetch-depth: 2"
    exit 1
  fi
else
  if ! git rev-parse -q --verify refs/remotes/origin/main >/dev/null; then
    if ! git fetch -q origin main 2>/dev/null; then
      echo "check-help-lint-shrink: skipped, could not fetch origin main; CI is the guard"
      exit 0
    fi
  fi
  if ! base="$(git merge-base origin/main HEAD 2>/dev/null)"; then
    problem "no merge base with origin/main; run 'git fetch --unshallow' and retry"
    exit 1
  fi
fi

# in_tree <ref> <path>: whether the path exists at the ref.
in_tree() { git cat-file -e "$1:$2" 2>/dev/null; }

# The rule block of a ref, markers included, or nothing.
rule_block() {
  git show "$1:$LINT" 2>/dev/null |
    awk -v b="$RULES_BEGIN" -v e="$RULES_END" 'index($0, b) { on = 1 } on { print } index($0, e) { on = 0 }'
}

has_markers() {
  local block
  block="$(rule_block "$1")"
  case "$block" in
    *"$RULES_BEGIN"*"$RULES_END"*) return 0 ;;
    *) return 1 ;;
  esac
}

status=0

# (a) No line is added under the allowlist directory, and no file is created in
# it once it exists at base.
if in_tree "$base" "$ALLOW_DIR"; then
  added="$(git diff --no-renames --numstat "$base" HEAD -- "$ALLOW_DIR" | awk '$1 > 0 { print $3 }')"
  if [ -n "$added" ]; then
    while IFS= read -r path; do
      problem "$path: a line was added; the help lint allowlists only shrink"
    done <<EOF
$added
EOF
    status=1
  fi
fi

# (d) golden/flags.txt is unchanged when it exists at base.
if [ "$allow_flags" -eq 0 ] && in_tree "$base" "$FLAGS" && in_tree HEAD "$FLAGS"; then
  if ! git diff --quiet "$base" HEAD -- "$FLAGS"; then
    problem "$FLAGS changed; label the PR flags-change and re-run the job if the change is deliberate"
    status=1
  fi
fi

# (e) The rule block is unchanged, and its markers stay, when they exist at base.
if [ "$allow_rules" -eq 0 ] && has_markers "$base"; then
  if ! has_markers HEAD; then
    problem "$LINT lost the RULES-BEGIN and RULES-END markers; label the PR help-lint-rules if deliberate"
    status=1
  elif [ "$(rule_block "$base")" != "$(rule_block HEAD)" ]; then
    problem "$LINT: the RULES block changed; label the PR help-lint-rules and re-run the job if deliberate"
    status=1
  fi
fi

exit "$status"
