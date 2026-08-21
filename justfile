# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# Task runner for this repo. Replaces the Makefile, which had one target and
# still shelled out to `python3 shell/gen-shared-settings.py` even after
# ADR 0007 WU-20 ported that generator into the binary as
# `playbook settings gen`. WU-20's own acceptance criterion says
# `grep -c python3 Makefile` is 0, which was false for as long as that file
# existed. Moving to just and calling the binary makes it true.
#
# The recipe is `settings-shared`, not `settings.shared.json`. just recipe
# names cannot contain dots, so the file-target naming the Makefile used does
# not carry over. The output file is unchanged.

set shell := ["bash", "-uc"]

# Where to read the live settings from. Override for testing:
#   just src=/tmp/fixture.json settings-shared
src := env_var_or_default("SRC", env_var("HOME") / ".claude/settings.json")
perms := "permissions.shared.json"
out := "settings.shared.json"
bin := "./target/release/playbook"

# List the available recipes.
default:
    @just --list

# Regenerate settings.shared.json from a live settings.json. Dry run by default.
settings-shared write="0": build
    #!/usr/bin/env bash
    # Reads a file that is NOT in the repo, so this is a manual step and never
    # part of CI; see ADR 0006 on why the seed is generated rather than
    # reproducible.
    #
    # DRY RUN BY DEFAULT, and that default is load-bearing. The generator
    # derives the seed from the maintainer's live settings.json, so whatever
    # has drifted there lands in a tracked file. Measured 2026-08-21: running
    # this on a machine whose settings.json still carried the legacy
    # ~/.claude/hooks/<name>.sh guard commands deleted the ENTIRE .hooks block
    # from the seed, because ADR 0007 WU-13 removed that form from the
    # generator's SAFETY_REGEXP. `playbook settings check` passed on the gutted
    # result, so nothing downstream would have objected.
    #
    # Pass write=1 only after reading the diff this prints.
    set -uo pipefail
    test -r "{{ src }}" || { echo "just: source settings not readable: {{ src }}; nothing regenerated" >&2; exit 1; }
    {{ bin }} settings gen "{{ src }}" "{{ perms }}" > "{{ out }}.tmp" || exit 1
    if diff -u "{{ out }}" "{{ out }}.tmp" > /dev/null 2>&1; then
        rm -f "{{ out }}.tmp"
        echo "just: {{ out }} is already up to date with {{ src }}"
        exit 0
    fi
    echo "just: proposed changes to {{ out }} (from {{ src }}):"
    diff -u "{{ out }}" "{{ out }}.tmp" || true
    if [ "{{ write }}" != "1" ]; then
        rm -f "{{ out }}.tmp"
        echo
        echo "just: DRY RUN, nothing written. Review the diff above, then:"
        echo "just: only the .hooks block should differ; any other changed key is"
        echo "just: personal drift from {{ src }} and must not be committed."
        echo "just:   just settings-shared write=1"
        exit 0
    fi
    mv "{{ out }}.tmp" "{{ out }}"
    echo "just: regenerated {{ out }} from {{ src }}"
    {{ bin }} settings check "{{ out }}" "{{ perms }}" .

# Build the release binary the other recipes call.
build:
    @cargo build --release --quiet

# Everything CI runs, so a green local run means a green PR.
check: build
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test
    git ls-files '*.sh' | xargs shellcheck --severity=warning
    @for f in $(git ls-files '*.sh'); do bash -n "$f"; done
    {{ bin }} settings check "{{ out }}" "{{ perms }}" .
    {{ bin }} manifest check .

# Run every behavioural shell suite exactly the way CI does.
test-shell:
    #!/usr/bin/env bash
    set -uo pipefail
    export GIT_CONFIG_GLOBAL="$(mktemp)"
    git config --global user.email "ci@github.local"
    git config --global user.name "CI"
    total=0; fails=0
    while IFS= read -r t; do
      total=$((total+1))
      bash "$t" >/dev/null 2>&1 || { fails=$((fails+1)); echo "FAIL: $t"; }
    done < <(git ls-files '*.test.sh')
    echo "suites=$total failed=$fails"
    [ "$fails" -eq 0 ]
