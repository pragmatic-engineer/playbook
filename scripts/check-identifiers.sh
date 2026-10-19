#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# check-identifiers.sh: fails when a tracked file names a maintainer path, a
# Jira host, the `rtk` local CLI wrapper or the maintainer's username. Run it
# from the root of a checkout; shell-ci and `just check` both call it.
#
# docs/adr/** is skipped: decision records keep the facts of their day. The
# files below reuse these strings on purpose, to test or document real
# product behavior, and this script spells them out in its pattern.
set -euo pipefail

if git ls-files -z -- \
    ':!scripts/check-identifiers.sh' \
    ':!docs/adr/**' \
    ':!crates/cli/src/hooks/rm_workspace_guard.rs' \
    ':!crates/cli/src/hooks/commit_message_sanitizer/engine.rs' \
    ':!crates/cli/src/hooks/commit_message_sanitizer/git.rs' \
    ':!crates/cli/src/init/wire.rs' \
    ':!crates/cli/tests/fixtures/golden/init.clean-install.json' \
    ':!crates/cli/tests/fixtures/golden/init.skip-triggering.json' \
    ':!crates/cli/tests/hooks_commit_message_sanitizer.rs' \
    ':!crates/cli/tests/hooks_guards.rs' \
    ':!crates/cli/tests/init_wire.rs' \
    ':!crates/cli/tests/settings_check.rs' \
    ':!crates/cli/tests/settings_gen.rs' \
    ':!crates/cli/tests/support/sanitizer_lab.rs' \
    | xargs -0 -r grep -HnE 'Workspace/|atlassian\.net|\brtk\b|isantos'; then
  echo "FAIL a tracked file names a personal identifier or a maintainer-specific string (see above)"
  exit 1
fi
echo "PASS no tracked file names a personal identifier"
