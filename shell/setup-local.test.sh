#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# setup-local.test.sh: hermetic tests for shell/setup-local.sh.
# Each scenario carves its own mktemp HOME; the real ~/.claude is never
# touched. --skip-deps keeps brew out of scope.
#
# Run:  bash shell/setup-local.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="${SCRIPT_DIR}/setup-local.sh"

PASS=0
FAIL=0

pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1"; (( FAIL++ )) || true; }

# Single top-level scratch dir; each scenario carves its own subtree.
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

# Run setup-local.sh with a controlled CLAUDE_HOME and HOME.
# Always skips brew so the test stays hermetic.
# Extra args are forwarded to the script.
run_setup() {
    local home="$1" claude_home="$2"; shift 2
    CLAUDE_HOME="$claude_home" HOME="$home" bash "$SCRIPT" --skip-deps "$@" >/dev/null 2>&1
}

# Like run_setup but captures stderr+stdout for assertion.
run_setup_out() {
    local home="$1" claude_home="$2"; shift 2
    CLAUDE_HOME="$claude_home" HOME="$home" bash "$SCRIPT" --skip-deps "$@" 2>&1
}

run_scenario() {
    local name="$1" fn="$2"
    if "$fn"; then pass "$name"; else fail "$name"; fi
}

# ---------------------------------------------------------------------------
# (a) DEFAULT: guards + settings only; NO rc file written, NO shell files
#     copied into CLAUDE_HOME/shell/.
# ---------------------------------------------------------------------------
scenario_a_default() {
    local d home claude_home rc guards g
    d="$(mktemp -d "$WORK/default.XXXXXX")"
    home="$d/home"
    claude_home="$d/claude"
    mkdir -p "$home" "$claude_home"

    run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  rc=$rc"; return 1; }

    # Guards must be installed.
    for g in rm-workspace-guard.sh bg-await-guard.sh no-dash-guard.sh; do
        [ -f "$claude_home/hooks/$g" ] \
            || { echo "  guard hook not copied: $g"; return 1; }
    done

    # settings.json must be seeded.
    [ -f "$claude_home/settings.json" ] \
        || { echo "  settings.json not created"; return 1; }
    [ -f "$claude_home/.settings.base.json" ] \
        || { echo "  .settings.base.json not created"; return 1; }

    guards="$(jq '[.hooks.PreToolUse[]?.hooks[]?.command]
                  | map(select(test("rm-workspace-guard|bg-await-guard|no-dash-guard")))
                  | length' \
                "$claude_home/settings.json" 2>/dev/null || echo 0)"
    [ "${guards:-0}" -ge 3 ] \
        || { echo "  guards=${guards:-0} (expected >=3)"; return 1; }

    # No rc file must have been written.
    [ ! -f "$home/.zshrc" ] \
        || { echo "  .zshrc was written by default run"; return 1; }
    [ ! -f "$home/.bashrc" ] \
        || { echo "  .bashrc was written by default run"; return 1; }

    # No cc.zsh or cc.sh in CLAUDE_HOME/shell/.
    [ ! -f "$claude_home/shell/cc.zsh" ] \
        || { echo "  cc.zsh was copied by default run"; return 1; }
    [ ! -f "$claude_home/shell/cc.sh" ] \
        || { echo "  cc.sh was copied by default run"; return 1; }
}

# ---------------------------------------------------------------------------
# (b) --aliases with SHELL=/bin/bash: copies launcher files and adds the
#     cc.sh source line to the throwaway .bashrc.
# ---------------------------------------------------------------------------
scenario_b_aliases_bash() {
    local d home claude_home rc
    d="$(mktemp -d "$WORK/aliases_bash.XXXXXX")"
    home="$d/home"
    claude_home="$d/claude"
    mkdir -p "$home" "$claude_home"

    SHELL=/bin/bash run_setup "$home" "$claude_home" --aliases; rc=$?
    [ "$rc" -eq 0 ] || { echo "  rc=$rc"; return 1; }

    # cc.sh must be in CLAUDE_HOME/shell/bash/.
    [ -f "$claude_home/shell/bash/cc.sh" ] \
        || { echo "  cc.sh not copied to CLAUDE_HOME/shell/bash"; return 1; }

    # .bashrc must contain the cc.sh source line.
    [ -f "$home/.bashrc" ] \
        || { echo "  .bashrc not created"; return 1; }
    grep -qF 'shell/bash/cc.sh' "$home/.bashrc" \
        || { echo "  cc.sh source line not in .bashrc"; return 1; }

    # .zshrc must NOT have been written.
    [ ! -f "$home/.zshrc" ] \
        || { echo "  .zshrc was written for bash shell"; return 1; }
}

# ---------------------------------------------------------------------------
# (c) --aliases with SHELL=/bin/zsh: copies launcher files and adds the
#     cc.zsh source line to the throwaway .zshrc.
# ---------------------------------------------------------------------------
scenario_c_aliases_zsh() {
    local d home claude_home rc
    d="$(mktemp -d "$WORK/aliases_zsh.XXXXXX")"
    home="$d/home"
    claude_home="$d/claude"
    mkdir -p "$home" "$claude_home"

    SHELL=/bin/zsh run_setup "$home" "$claude_home" --aliases; rc=$?
    [ "$rc" -eq 0 ] || { echo "  rc=$rc"; return 1; }

    # cc.zsh must be in CLAUDE_HOME/shell/zsh/.
    [ -f "$claude_home/shell/zsh/cc.zsh" ] \
        || { echo "  cc.zsh not copied to CLAUDE_HOME/shell/zsh"; return 1; }

    # .zshrc must contain the cc.zsh source line.
    [ -f "$home/.zshrc" ] \
        || { echo "  .zshrc not created"; return 1; }
    grep -qF 'shell/zsh/cc.zsh' "$home/.zshrc" \
        || { echo "  cc.zsh source line not in .zshrc"; return 1; }

    # .bashrc must NOT have been written.
    [ ! -f "$home/.bashrc" ] \
        || { echo "  .bashrc was written for zsh shell"; return 1; }
}

# ---------------------------------------------------------------------------
# (d) --system-prompt: implies --aliases AND copies SYSTEM_PROMPT.md to
#     CLAUDE_HOME/prompts/.
# ---------------------------------------------------------------------------
scenario_d_system_prompt() {
    local d home claude_home rc
    d="$(mktemp -d "$WORK/sysprompt.XXXXXX")"
    home="$d/home"
    claude_home="$d/claude"
    mkdir -p "$home" "$claude_home"

    SHELL=/bin/bash run_setup "$home" "$claude_home" --system-prompt; rc=$?
    [ "$rc" -eq 0 ] || { echo "  rc=$rc"; return 1; }

    # Implies --aliases: launcher files must be present.
    [ -f "$claude_home/shell/bash/cc.sh" ] \
        || { echo "  cc.sh not copied (--system-prompt implies --aliases)"; return 1; }

    # SYSTEM_PROMPT.md must be in CLAUDE_HOME/prompts/.
    [ -f "$claude_home/prompts/SYSTEM_PROMPT.md" ] \
        || { echo "  SYSTEM_PROMPT.md not copied to prompts/"; return 1; }
}

# ---------------------------------------------------------------------------
# (e) Idempotency: re-run --aliases makes no changes (rc file source line
#     appears exactly once; no duplicate appended).
# ---------------------------------------------------------------------------
scenario_e_idempotent_aliases() {
    local d home claude_home rc count
    d="$(mktemp -d "$WORK/idem_aliases.XXXXXX")"
    home="$d/home"
    claude_home="$d/claude"
    mkdir -p "$home" "$claude_home"

    # First run.
    SHELL=/bin/bash run_setup "$home" "$claude_home" --aliases; rc=$?
    [ "$rc" -eq 0 ] || { echo "  first run rc=$rc"; return 1; }

    # Capture state after first run.
    cp "$home/.bashrc" "$d/bashrc_after1"

    # Second run.
    SHELL=/bin/bash run_setup "$home" "$claude_home" --aliases; rc=$?
    [ "$rc" -eq 0 ] || { echo "  second run rc=$rc"; return 1; }

    # .bashrc must be byte-identical (source line appended only once).
    cmp -s "$home/.bashrc" "$d/bashrc_after1" \
        || { echo "  .bashrc changed on idempotent re-run"; return 1; }

    # Double-check: source line appears exactly once.
    count="$(grep -cF 'shell/bash/cc.sh' "$home/.bashrc" 2>/dev/null || echo 0)"
    [ "$count" -eq 1 ] \
        || { echo "  cc.sh source line count=$count (expected 1)"; return 1; }
}

# ---------------------------------------------------------------------------
# (f) MERGE-PRESERVES: pre-create settings.json with a custom key, then run.
#     The custom key must survive (a naive cp would overwrite it).
# ---------------------------------------------------------------------------
scenario_f_merge_preserves() {
    local d home claude_home rc
    d="$(mktemp -d "$WORK/merge.XXXXXX")"
    home="$d/home"
    claude_home="$d/claude"
    mkdir -p "$home" "$claude_home"

    # Seed a custom key with no baseline. With base={}, every user key is
    # treated as contested (user != base) and is preserved by the merge policy.
    printf '{"my_custom_key":"sentinel_value"}\n' > "$claude_home/settings.json"

    run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  rc=$rc"; return 1; }

    jq -e '.my_custom_key == "sentinel_value"' "$claude_home/settings.json" \
        >/dev/null 2>&1 \
        || { echo "  custom key lost: $(jq -c . "$claude_home/settings.json" 2>/dev/null)"; return 1; }
}

# ---------------------------------------------------------------------------
# (bonus) DEFAULT IDEMPOTENT: third run is byte-identical to second.
# ---------------------------------------------------------------------------
scenario_g_default_idempotent() {
    local d home claude_home rc
    d="$(mktemp -d "$WORK/idem_default.XXXXXX")"
    home="$d/home"
    claude_home="$d/claude"
    mkdir -p "$home" "$claude_home"

    run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  run1 rc=$rc"; return 1; }

    run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  run2 rc=$rc"; return 1; }

    cp "$claude_home/settings.json"       "$d/settings_after2.json"
    cp "$claude_home/.settings.base.json" "$d/base_after2.json"

    run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  run3 rc=$rc"; return 1; }

    cmp -s "$claude_home/settings.json" "$d/settings_after2.json" \
        || { echo "  settings.json changed on idempotent run"; return 1; }
    cmp -s "$claude_home/.settings.base.json" "$d/base_after2.json" \
        || { echo "  .settings.base.json changed on idempotent run"; return 1; }
}

# ---------------------------------------------------------------------------
# (h) MIGRATION: an rc file holding the old-form source line (with its
#     comment) is migrated to the new-form line on --aliases. Unrelated user
#     content, including the blank line that separates it, survives. A
#     second run makes no further changes.
# ---------------------------------------------------------------------------
scenario_h_migration() {
    local d home claude_home rc
    d="$(mktemp -d "$WORK/migration.XXXXXX")"
    home="$d/home"
    claude_home="$d/claude"
    mkdir -p "$home" "$claude_home"

    # Old-form .zshrc. FOO and BAR are unrelated user lines with their own
    # separating blank line, well away from the launcher block, so the test
    # can tell "the block was migrated" apart from "a user blank survived".
    printf 'export FOO=1\n\nexport BAR=2\n\n# playbook launchers (cc/ccd)\nsource "$HOME/.claude/shell/cc.zsh"\n' \
        > "$home/.zshrc"

    SHELL=/bin/zsh run_setup "$home" "$claude_home" --aliases; rc=$?
    [ "$rc" -eq 0 ] || { echo "  rc=$rc"; return 1; }

    # Zero old-form lines remain.
    grep -qxF 'source "$HOME/.claude/shell/cc.zsh"' "$home/.zshrc" \
        && { echo "  old-form source line still present"; return 1; }

    # Exactly one new-form line.
    local new_count
    new_count="$(grep -cF 'shell/zsh/cc.zsh' "$home/.zshrc" 2>/dev/null || echo 0)"
    [ "$new_count" -eq 1 ] || { echo "  new-form line count=$new_count (expected 1)"; return 1; }

    # Exactly one launchers comment (the old one was absorbed, not doubled).
    local comment_count
    comment_count="$(grep -cF 'launchers (cc/ccd)' "$home/.zshrc" 2>/dev/null || echo 0)"
    [ "$comment_count" -eq 1 ] || { echo "  launchers comment count=$comment_count (expected 1)"; return 1; }

    # A .bak- backup exists.
    local bak_count
    bak_count=$(find "$home" -maxdepth 1 -name '.zshrc.bak-*' 2>/dev/null | wc -l | tr -d ' ')
    [ "$bak_count" -ge 1 ] || { echo "  no .zshrc backup found"; return 1; }

    # Unrelated user lines survive.
    grep -qxF 'export FOO=1' "$home/.zshrc" || { echo "  export FOO=1 missing"; return 1; }
    grep -qxF 'export BAR=2' "$home/.zshrc" || { echo "  export BAR=2 missing"; return 1; }

    # The blank line separating FOO and BAR survives: assert the gap between
    # the two markers, not just their presence.
    local foo_line bar_line gap
    foo_line="$(grep -n '^export FOO=1$' "$home/.zshrc" | head -1 | cut -d: -f1)"
    bar_line="$(grep -n '^export BAR=2$' "$home/.zshrc" | head -1 | cut -d: -f1)"
    [ -n "$foo_line" ] || { echo "  export FOO=1 marker missing"; return 1; }
    [ -n "$bar_line" ] || { echo "  export BAR=2 marker missing"; return 1; }
    gap=$(( bar_line - foo_line ))
    [ "$gap" -eq 2 ] || {
        echo "  gap between FOO and BAR markers = $gap, expected 2 (one blank line between them)"
        return 1
    }

    # Re-run: the file must be byte identical (already migrated, already
    # up to date).
    cp "$home/.zshrc" "$d/zshrc_after1"
    SHELL=/bin/zsh run_setup "$home" "$claude_home" --aliases; rc=$?
    [ "$rc" -eq 0 ] || { echo "  second run rc=$rc"; return 1; }
    cmp -s "$home/.zshrc" "$d/zshrc_after1" \
        || { echo "  .zshrc changed on second run"; return 1; }
}

# Binary detection. Both cases are deliberately no-download: the fetch path
# needs the network and a published release, so it is exercised manually rather
# than in CI. What these pin is that setup does NOT re-download when a binary is
# already available, which is the part that would otherwise silently re-fetch on
# every run.
#
# PATH is stripped to system dirs so the developer's own `playbook` cannot leak
# in and make either case pass for the wrong reason.
scenario_i_binary_present() {
    local home="$WORK/i-home" ch="$WORK/i-home/.claude" bin="$WORK/i-bin" out
    mkdir -p "$home" "$ch" "$bin"
    printf '#!/bin/sh\necho stub\n' > "$bin/playbook"
    chmod 0755 "$bin/playbook"

    out="$(CLAUDE_HOME="$ch" HOME="$home" PLAYBOOK_BIN_DIR="$bin" \
           PATH="/usr/bin:/bin:/usr/sbin:/sbin" \
           bash "$SCRIPT" --skip-deps 2>&1)"

    case "$out" in
        *"binary: ok"*) ;;
        *) echo "expected 'binary: ok', got: $out" >&2; return 1 ;;
    esac
    # A re-download would have replaced the stub with the real binary.
    grep -q "^echo stub$" "$bin/playbook" || { echo "stub was overwritten" >&2; return 1; }
}

scenario_j_binary_on_path() {
    local home="$WORK/j-home" ch="$WORK/j-home/.claude" bin="$WORK/j-bin" onpath="$WORK/j-path" out
    mkdir -p "$home" "$ch" "$bin" "$onpath"
    printf '#!/bin/sh\necho stub\n' > "$onpath/playbook"
    chmod 0755 "$onpath/playbook"

    out="$(CLAUDE_HOME="$ch" HOME="$home" PLAYBOOK_BIN_DIR="$bin" \
           PATH="$onpath:/usr/bin:/bin" \
           bash "$SCRIPT" --skip-deps 2>&1)"

    case "$out" in
        *"binary: ok"*) ;;
        *) echo "expected 'binary: ok', got: $out" >&2; return 1 ;;
    esac
    # Nothing should be written into PLAYBOOK_BIN_DIR when PATH already resolves.
    [ ! -e "$bin/playbook" ] || { echo "wrote to PLAYBOOK_BIN_DIR unnecessarily" >&2; return 1; }
}

run_scenario "A: default run wires guards+settings; no rc file; no shell files in CLAUDE_HOME" scenario_a_default
run_scenario "B: --aliases bash copies launcher files and adds cc.sh source line to .bashrc"   scenario_b_aliases_bash
run_scenario "C: --aliases zsh copies launcher files and adds cc.zsh source line to .zshrc"    scenario_c_aliases_zsh
run_scenario "D: --system-prompt implies --aliases and copies SYSTEM_PROMPT.md"                scenario_d_system_prompt
run_scenario "E: idempotent --aliases re-run does not duplicate source line in rc file"        scenario_e_idempotent_aliases
run_scenario "F: merge preserves a custom key from pre-existing settings.json"                 scenario_f_merge_preserves
run_scenario "G: default idempotent -- third run byte-identical to second"                     scenario_g_default_idempotent
run_scenario "H: migration of an old-form rc line preserves unrelated content and blanks"      scenario_h_migration
run_scenario "I: an existing binary in PLAYBOOK_BIN_DIR is detected, not re-downloaded"        scenario_i_binary_present
run_scenario "J: a binary already on PATH is detected without touching PLAYBOOK_BIN_DIR"       scenario_j_binary_on_path

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
