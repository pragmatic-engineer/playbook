#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# uninstall.test.sh: hermetic tests for uninstall.sh.
# Every scenario runs in fresh mktemp dirs with CLAUDE_HOME and HOME pointing
# only to temp space.  A hard guard at the top of each destructive scenario
# asserts both dirs begin with the test's temp root before uninstall.sh runs.
#
# Run:  bash shell/uninstall.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SCRIPT_DIR}/.."
UNINSTALL="${REPO_ROOT}/uninstall.sh"

PASS=0
FAIL=0

pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1"; (( FAIL++ )) || true; }

# Single top-level scratch dir; each scenario carves out its own subtree.
WORK="$(mktemp -d)"
# shellcheck disable=SC2064
trap 'rm -rf "$WORK"' EXIT INT TERM

run_scenario() {
    local name="$1" fn="$2"
    if "$fn"; then pass "$name"; else fail "$name"; fi
}

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# assert_hermetic <temp_root> <CLAUDE_HOME> <HOME>
# Hard guard: both dirs must begin with temp_root.  Exits the entire test
# suite (exit 1) rather than just failing the scenario, because a failure
# here means a directory outside the sandbox might be targeted.
assert_hermetic() {
    local root="$1" ch="$2" h="$3"
    [[ "$ch" == "$root"* ]] || { echo "  SAFETY ABORT: CLAUDE_HOME ($ch) not under temp root ($root)"; exit 1; }
    [[ "$h"  == "$root"* ]] || { echo "  SAFETY ABORT: HOME ($h) not under temp root ($root)"; exit 1; }
}

# create_shipped <claude_home>: populate every shipped-entry allowlist entry.
create_shipped() {
    local ch="$1"
    # files
    for f in .gitignore Brewfile CODE_OF_CONDUCT.md CONTRIBUTING.md install.sh \
              LICENSE Makefile permissions.shared.json README.md SECURITY.md \
              settings.shared.json statusline.sh uninstall.sh; do
        touch "$ch/$f"
    done
    # directories (with a sentinel inside each so they are non-empty)
    for dir in agents commands docs hooks output-styles prompts shell skills; do
        mkdir -p "$ch/$dir"
        touch "$ch/$dir/.keep"
    done
}

# assert_shipped_removed <claude_home>: all shipped entries must be gone.
assert_shipped_removed() {
    local ch="$1"
    for f in .gitignore Brewfile CODE_OF_CONDUCT.md CONTRIBUTING.md install.sh \
              LICENSE Makefile permissions.shared.json README.md SECURITY.md \
              settings.shared.json statusline.sh uninstall.sh; do
        [ ! -e "$ch/$f" ] || { echo "  shipped file not removed: $f"; return 1; }
    done
    for dir in agents commands docs hooks output-styles prompts shell skills; do
        [ ! -e "$ch/$dir" ] || { echo "  shipped dir not removed: $dir/"; return 1; }
    done
}

# create_runtime <claude_home>: populate preserved runtime state entries.
create_runtime() {
    local ch="$1"
    touch "$ch/settings.json"
    touch "$ch/.settings.base.json"
    mkdir -p "$ch/backups" "$ch/sessions" "$ch/memory" "$ch/plans"
    touch "$ch/backups/.keep" "$ch/sessions/.keep" "$ch/memory/.keep" "$ch/plans/.keep"
}

# assert_runtime_preserved <claude_home>: runtime state must all still exist.
assert_runtime_preserved() {
    local ch="$1"
    [ -f "$ch/settings.json" ]        || { echo "  settings.json removed"; return 1; }
    [ -f "$ch/.settings.base.json" ]  || { echo "  .settings.base.json removed"; return 1; }
    [ -d "$ch/backups" ]              || { echo "  backups/ removed"; return 1; }
    [ -d "$ch/sessions" ]             || { echo "  sessions/ removed"; return 1; }
    [ -d "$ch/memory" ]               || { echo "  memory/ removed"; return 1; }
    [ -d "$ch/plans" ]                || { echo "  plans/ removed"; return 1; }
}

# write_zshrc_block <zshrc_path> <comment>: write a .zshrc with BEFORE and
# AFTER sentinel lines framing the launcher block.  <comment> is the full
# comment line to use, or "" for the no-comment case (bare source line).
write_zshrc_block() {
    local zshrc="$1" comment="$2"
    if [ -n "$comment" ]; then
        # blank line + comment + source line = 3 lines removed
        printf '# BEFORE_SENTINEL\n\n%s\nsource "$HOME/.claude/shell/cc.zsh"\n# AFTER_SENTINEL\n' \
            "$comment" > "$zshrc"
    else
        # bare source line only = 1 line removed
        printf '# BEFORE_SENTINEL\nsource "$HOME/.claude/shell/cc.zsh"\n# AFTER_SENTINEL\n' \
            > "$zshrc"
    fi
}

# ---------------------------------------------------------------------------
# Scenario A: basic uninstall (non-git CLAUDE_HOME, --yes)
# ---------------------------------------------------------------------------
scenario_basic() {
    local d ch h
    d="$(mktemp -d "$WORK/basic.XXXXXX")"
    ch="$d/claude"
    h="$d/home"
    mkdir -p "$ch" "$h"

    # Hard hermeticity guard — runs BEFORE uninstall.sh
    assert_hermetic "$d" "$ch" "$h"

    create_shipped "$ch"
    create_runtime "$ch"
    write_zshrc_block "$h/.zshrc" "# playbook launchers (cc/ccd)"
    local orig_lines
    orig_lines=$(wc -l < "$h/.zshrc" | tr -d ' ')

    CLAUDE_HOME="$ch" HOME="$h" bash "$UNINSTALL" --yes >/dev/null 2>&1 || {
        echo "  uninstall.sh exited non-zero"; return 1
    }

    assert_shipped_removed "$ch"  || return 1
    assert_runtime_preserved "$ch" || return 1

    # cc.zsh block must be gone
    grep -qF 'shell/cc.zsh' "$h/.zshrc" && { echo "  cc.zsh still in .zshrc"; return 1; }

    # Sentinel lines on both sides must still be present
    grep -qxF '# BEFORE_SENTINEL' "$h/.zshrc" || { echo "  BEFORE_SENTINEL missing from .zshrc"; return 1; }
    grep -qxF '# AFTER_SENTINEL'  "$h/.zshrc" || { echo "  AFTER_SENTINEL missing from .zshrc"; return 1; }

    # Line count: blank + comment + source = 3 lines removed
    local after_lines
    after_lines=$(wc -l < "$h/.zshrc" | tr -d ' ')
    [ "$after_lines" -eq $(( orig_lines - 3 )) ] || {
        echo "  .zshrc line count $after_lines != $(( orig_lines - 3 )) (orig=$orig_lines)"
        return 1
    }

    # Backup must exist
    local bak_count
    bak_count=$(find "$h" -maxdepth 1 -name '.zshrc.bak-*' 2>/dev/null | wc -l | tr -d ' ')
    [ "$bak_count" -ge 1 ] || { echo "  no .zshrc backup found"; return 1; }

    # Belt-and-suspenders: temp root still intact (nothing rm'd it)
    [ -d "$d" ] || { echo "  SAFETY: temp root removed"; exit 1; }
    [ -f "$h/.zshrc" ] || { echo "  SAFETY: .zshrc was removed instead of edited"; exit 1; }
}

# ---------------------------------------------------------------------------
# Scenario B: git guard — refuse without --force, proceed with --force --yes
# ---------------------------------------------------------------------------
scenario_git_guard() {
    local d ch h
    d="$(mktemp -d "$WORK/gitguard.XXXXXX")"
    ch="$d/claude"
    h="$d/home"
    mkdir -p "$ch" "$h"

    assert_hermetic "$d" "$ch" "$h"

    create_shipped "$ch"
    create_runtime "$ch"
    touch "$h/.zshrc"

    # Make CLAUDE_HOME a git repo
    git -C "$ch" init -q

    # Without --force: must exit non-zero and remove NOTHING
    CLAUDE_HOME="$ch" HOME="$h" bash "$UNINSTALL" --yes 2>/dev/null && {
        echo "  expected non-zero exit without --force, got 0"; return 1
    }

    # A shipped entry must still be present (nothing was removed)
    [ -f "$ch/install.sh" ] || { echo "  install.sh was removed despite guard"; return 1; }

    # With --force --yes: must proceed and remove shipped entries
    CLAUDE_HOME="$ch" HOME="$h" bash "$UNINSTALL" --force --yes >/dev/null 2>&1 || {
        echo "  --force --yes exited non-zero"; return 1
    }
    assert_shipped_removed "$ch" || return 1

    [ -d "$d" ] || { echo "  SAFETY: temp root removed"; exit 1; }
}

# ---------------------------------------------------------------------------
# Scenario C: --purge removes user config too
# ---------------------------------------------------------------------------
scenario_purge() {
    local d ch h
    d="$(mktemp -d "$WORK/purge.XXXXXX")"
    ch="$d/claude"
    h="$d/home"
    mkdir -p "$ch" "$h"

    assert_hermetic "$d" "$ch" "$h"

    create_shipped "$ch"
    create_runtime "$ch"
    touch "$h/.zshrc"

    CLAUDE_HOME="$ch" HOME="$h" bash "$UNINSTALL" --yes --purge >/dev/null 2>&1 || {
        echo "  uninstall.sh exited non-zero"; return 1
    }

    assert_shipped_removed "$ch" || return 1

    # With --purge, user config must also be gone
    [ ! -f "$ch/settings.json" ]       || { echo "  settings.json still present after --purge"; return 1; }
    [ ! -f "$ch/.settings.base.json" ] || { echo "  .settings.base.json still present after --purge"; return 1; }
    [ ! -d "$ch/backups" ]             || { echo "  backups/ still present after --purge"; return 1; }

    # sessions/ and memory/ must still be present (--purge doesn't touch them)
    [ -d "$ch/sessions" ] || { echo "  sessions/ removed by --purge (should not be)"; return 1; }
    [ -d "$ch/memory" ]   || { echo "  memory/ removed by --purge (should not be)"; return 1; }
    [ -d "$ch/plans" ]    || { echo "  plans/ removed by --purge (should not be)"; return 1; }

    [ -d "$d" ] || { echo "  SAFETY: temp root removed"; exit 1; }
}

# ---------------------------------------------------------------------------
# Scenario D: .zshrc comment variants
# ---------------------------------------------------------------------------

# D1: old variant — # claude-config launchers (cc/ccd)
scenario_comment_old() {
    local d ch h
    d="$(mktemp -d "$WORK/comment_old.XXXXXX")"
    ch="$d/claude"
    h="$d/home"
    mkdir -p "$ch" "$h"

    assert_hermetic "$d" "$ch" "$h"

    create_shipped "$ch"
    write_zshrc_block "$h/.zshrc" "# claude-config launchers (cc/ccd)"
    local orig_lines
    orig_lines=$(wc -l < "$h/.zshrc" | tr -d ' ')

    CLAUDE_HOME="$ch" HOME="$h" bash "$UNINSTALL" --yes >/dev/null 2>&1 || {
        echo "  uninstall.sh exited non-zero"; return 1
    }

    grep -qF 'shell/cc.zsh' "$h/.zshrc" && { echo "  cc.zsh still in .zshrc"; return 1; }
    grep -qxF '# BEFORE_SENTINEL' "$h/.zshrc" || { echo "  BEFORE_SENTINEL missing"; return 1; }
    grep -qxF '# AFTER_SENTINEL'  "$h/.zshrc" || { echo "  AFTER_SENTINEL missing"; return 1; }
    grep -qF 'claude-config' "$h/.zshrc" && { echo "  old comment line still present"; return 1; }

    local after_lines
    after_lines=$(wc -l < "$h/.zshrc" | tr -d ' ')
    [ "$after_lines" -eq $(( orig_lines - 3 )) ] || {
        echo "  .zshrc line count $after_lines != $(( orig_lines - 3 )) (orig=$orig_lines)"
        return 1
    }

    [ -d "$d" ] || { echo "  SAFETY: temp root removed"; exit 1; }
}

# D2: new variant — # playbook launchers (cc/ccd)
scenario_comment_new() {
    local d ch h
    d="$(mktemp -d "$WORK/comment_new.XXXXXX")"
    ch="$d/claude"
    h="$d/home"
    mkdir -p "$ch" "$h"

    assert_hermetic "$d" "$ch" "$h"

    create_shipped "$ch"
    write_zshrc_block "$h/.zshrc" "# playbook launchers (cc/ccd)"
    local orig_lines
    orig_lines=$(wc -l < "$h/.zshrc" | tr -d ' ')

    CLAUDE_HOME="$ch" HOME="$h" bash "$UNINSTALL" --yes >/dev/null 2>&1 || {
        echo "  uninstall.sh exited non-zero"; return 1
    }

    grep -qF 'shell/cc.zsh' "$h/.zshrc" && { echo "  cc.zsh still in .zshrc"; return 1; }
    grep -qxF '# BEFORE_SENTINEL' "$h/.zshrc" || { echo "  BEFORE_SENTINEL missing"; return 1; }
    grep -qxF '# AFTER_SENTINEL'  "$h/.zshrc" || { echo "  AFTER_SENTINEL missing"; return 1; }
    grep -qF 'playbook launchers' "$h/.zshrc" && { echo "  new comment line still present"; return 1; }

    local after_lines
    after_lines=$(wc -l < "$h/.zshrc" | tr -d ' ')
    [ "$after_lines" -eq $(( orig_lines - 3 )) ] || {
        echo "  .zshrc line count $after_lines != $(( orig_lines - 3 )) (orig=$orig_lines)"
        return 1
    }

    [ -d "$d" ] || { echo "  SAFETY: temp root removed"; exit 1; }
}

# D3: no-comment case — bare source line only
scenario_comment_none() {
    local d ch h
    d="$(mktemp -d "$WORK/comment_none.XXXXXX")"
    ch="$d/claude"
    h="$d/home"
    mkdir -p "$ch" "$h"

    assert_hermetic "$d" "$ch" "$h"

    create_shipped "$ch"
    write_zshrc_block "$h/.zshrc" ""
    local orig_lines
    orig_lines=$(wc -l < "$h/.zshrc" | tr -d ' ')

    CLAUDE_HOME="$ch" HOME="$h" bash "$UNINSTALL" --yes >/dev/null 2>&1 || {
        echo "  uninstall.sh exited non-zero"; return 1
    }

    grep -qF 'shell/cc.zsh' "$h/.zshrc" && { echo "  cc.zsh still in .zshrc"; return 1; }
    grep -qxF '# BEFORE_SENTINEL' "$h/.zshrc" || { echo "  BEFORE_SENTINEL missing"; return 1; }
    grep -qxF '# AFTER_SENTINEL'  "$h/.zshrc" || { echo "  AFTER_SENTINEL missing"; return 1; }

    # No-comment: only the source line (1 line) is removed
    local after_lines
    after_lines=$(wc -l < "$h/.zshrc" | tr -d ' ')
    [ "$after_lines" -eq $(( orig_lines - 1 )) ] || {
        echo "  .zshrc line count $after_lines != $(( orig_lines - 1 )) (orig=$orig_lines)"
        return 1
    }

    [ -d "$d" ] || { echo "  SAFETY: temp root removed"; exit 1; }
}

# ---------------------------------------------------------------------------
# Scenario E: new-path source line (shell/zsh/cc.zsh) is also stripped.
# Pins that an install made after the shell/bash/zsh/shared layout move
# (which sources the new path directly, no shim) is fully uninstallable.
# ---------------------------------------------------------------------------
scenario_new_path() {
    local d ch h
    d="$(mktemp -d "$WORK/new_path.XXXXXX")"
    ch="$d/claude"
    h="$d/home"
    mkdir -p "$ch" "$h"

    assert_hermetic "$d" "$ch" "$h"

    create_shipped "$ch"
    printf '# BEFORE_SENTINEL\n\n# playbook launchers (cc/ccd)\nsource "$HOME/.claude/shell/zsh/cc.zsh"\n# AFTER_SENTINEL\n' \
        > "$h/.zshrc"
    local orig_lines
    orig_lines=$(wc -l < "$h/.zshrc" | tr -d ' ')

    CLAUDE_HOME="$ch" HOME="$h" bash "$UNINSTALL" --yes >/dev/null 2>&1 || {
        echo "  uninstall.sh exited non-zero"; return 1
    }

    grep -qF 'shell/zsh/cc.zsh' "$h/.zshrc" && { echo "  new-path cc.zsh still in .zshrc"; return 1; }
    grep -qxF '# BEFORE_SENTINEL' "$h/.zshrc" || { echo "  BEFORE_SENTINEL missing"; return 1; }
    grep -qxF '# AFTER_SENTINEL'  "$h/.zshrc" || { echo "  AFTER_SENTINEL missing"; return 1; }

    local after_lines
    after_lines=$(wc -l < "$h/.zshrc" | tr -d ' ')
    [ "$after_lines" -eq $(( orig_lines - 3 )) ] || {
        echo "  .zshrc line count $after_lines != $(( orig_lines - 3 )) (orig=$orig_lines)"
        return 1
    }

    [ -d "$d" ] || { echo "  SAFETY: temp root removed"; exit 1; }
}

# ---------------------------------------------------------------------------
# Run all scenarios
# ---------------------------------------------------------------------------
run_scenario "A: basic uninstall removes shipped entries, preserves runtime" scenario_basic
run_scenario "B: git guard refuses without --force; proceeds with --force --yes" scenario_git_guard
run_scenario "C: --purge also removes settings.json, .settings.base.json, backups/" scenario_purge
run_scenario "D1: old comment variant (# claude-config launchers) removed" scenario_comment_old
run_scenario "D2: new comment variant (# playbook launchers) removed" scenario_comment_new
run_scenario "D3: no-comment case (bare source line) removed" scenario_comment_none
run_scenario "E: new-path variant (shell/zsh/cc.zsh) removed" scenario_new_path

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
