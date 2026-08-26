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
REPO_ROOT="${SCRIPT_DIR}/.."
GOLDEN_DIR="${REPO_ROOT}/tests/fixtures/golden"

PASS=0
FAIL=0

pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1"; (( FAIL++ )) || true; }

# Single top-level scratch dir; each scenario carves its own subtree.
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

# A real `playbook` binary, built once and staged at a scratch
# PLAYBOOK_BIN_DIR, for scenarios that need `playbook init` to actually run
# (not just be invoked): the settings merge, hooks wiring, and statusline
# placement it now performs in one pass. Mirrors the same seam
# shell/install-seed.test.sh and shell/plugin-e2e.sh already use for a real
# binary, rather than the stubbed-binary seam scenarios I/J/K/L below use to
# pin invocation shape alone.
command -v cargo >/dev/null 2>&1 || { echo "cargo not found on PATH" >&2; exit 2; }
REAL_BIN_SRC="${REPO_ROOT}/target/debug/playbook"
if [ ! -x "$REAL_BIN_SRC" ]; then
    echo "Building playbook (cargo build)..."
    ( cd "$REPO_ROOT" && cargo build --quiet ) || { echo "cargo build failed" >&2; exit 2; }
fi
REAL_BIN_DIR="$WORK/real-playbook-bin"
mkdir -p "$REAL_BIN_DIR"
cp "$REAL_BIN_SRC" "$REAL_BIN_DIR/playbook"
chmod 0755 "$REAL_BIN_DIR/playbook"
# PATH stripped to system dirs plus the staged binary: the developer's own
# `playbook`, if any, must not leak in and mask a real regression.
REAL_BIN_PATH="$REAL_BIN_DIR:/usr/bin:/bin:/usr/sbin:/sbin"

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
# (a) DEFAULT: settings + guards + statusline via `playbook init`; NO guard
#     .sh file copied (that copy loop is gone), NO rc file written, NO shell
#     files copied into CLAUDE_HOME/shell/.
#
#     claude_home is nested under home (CLAUDE_HOME == $HOME/.claude), not a
#     sibling directory: `playbook init` resolves its own target from $HOME
#     alone and has no CLAUDE_HOME override, so the script's own
#     CLAUDE_HOME-equals-$HOME/.claude guard must actually hold for this
#     scenario to exercise the real settings/hooks/statusline pipeline
#     instead of the skip-with-warning path scenario L pins.
# ---------------------------------------------------------------------------
scenario_a_default() {
    local d home claude_home rc guards g
    d="$(mktemp -d "$WORK/default.XXXXXX")"
    home="$d/home"
    claude_home="$home/.claude"
    mkdir -p "$claude_home"

    PATH="$REAL_BIN_PATH" run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  rc=$rc"; return 1; }

    # The guard .sh copy loop is gone: no guard script lands in hooks/.
    for g in rm-workspace-guard.sh bg-await-guard.sh no-dash-guard.sh; do
        [ ! -f "$claude_home/hooks/$g" ] \
            || { echo "  guard hook copied despite Step 1's removal: $g"; return 1; }
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

    # statusline.sh must be placed: new behaviour, since a plain `playbook
    # init` call dispatches the full pipeline, not a merge-only subset.
    [ -f "$claude_home/statusline.sh" ] \
        || { echo "  statusline.sh not placed by a default run"; return 1; }

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
# (f) MERGE-PRESERVES: pre-create settings.json with a custom key, then run
#     through `playbook init`. The custom key must survive (a naive cp would
#     overwrite it).
#
#     This seed (base={}, an additive novel key) never triggers a skip
#     report: `my_custom_key` is not in the template at all, so the merge's
#     skip list (populated only when the template ALSO tried to change a
#     contested key) stays empty here by design. The genuine three-way
#     collision that does produce a skip report is covered end to end,
#     through this same shell wrapper, by scenario O's golden differential
#     below (seeds a value the template also updates).
# ---------------------------------------------------------------------------
scenario_f_merge_preserves() {
    local d home claude_home rc
    d="$(mktemp -d "$WORK/merge.XXXXXX")"
    home="$d/home"
    claude_home="$home/.claude"
    mkdir -p "$claude_home"

    # Seed a custom key with no baseline. With base={}, every user key is
    # treated as contested (user != base) and is preserved by the merge policy.
    printf '{"my_custom_key":"sentinel_value"}\n' > "$claude_home/settings.json"

    PATH="$REAL_BIN_PATH" run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  rc=$rc"; return 1; }

    jq -e '.my_custom_key == "sentinel_value"' "$claude_home/settings.json" \
        >/dev/null 2>&1 \
        || { echo "  custom key lost: $(jq -c . "$claude_home/settings.json" 2>/dev/null)"; return 1; }
}

# ---------------------------------------------------------------------------
# (bonus) DEFAULT IDEMPOTENT: third run is byte-identical to second, and the
#     idempotent 2nd-to-3rd transition creates no new backup or skip-report
#     file (run 1 is expected to create exactly one backup of the placeholder
#     settings.json `playbook init` seeds before its first real merge; only
#     the 2nd-to-3rd transition needs to be silent).
# ---------------------------------------------------------------------------
scenario_g_default_idempotent() {
    local d home claude_home rc backups_after2 skips_after2 backups_after3 skips_after3
    d="$(mktemp -d "$WORK/idem_default.XXXXXX")"
    home="$d/home"
    claude_home="$home/.claude"
    mkdir -p "$claude_home"

    PATH="$REAL_BIN_PATH" run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  run1 rc=$rc"; return 1; }

    PATH="$REAL_BIN_PATH" run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  run2 rc=$rc"; return 1; }

    cp "$claude_home/settings.json"       "$d/settings_after2.json"
    cp "$claude_home/.settings.base.json" "$d/base_after2.json"
    backups_after2=$(find "$claude_home" -maxdepth 1 -name 'settings.json.bak.*' 2>/dev/null | wc -l | tr -d ' ')
    skips_after2=$(find "$claude_home" -maxdepth 1 -name 'settings-merge-skipped.*.json' 2>/dev/null | wc -l | tr -d ' ')

    PATH="$REAL_BIN_PATH" run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  run3 rc=$rc"; return 1; }

    cmp -s "$claude_home/settings.json" "$d/settings_after2.json" \
        || { echo "  settings.json changed on idempotent run"; return 1; }
    cmp -s "$claude_home/.settings.base.json" "$d/base_after2.json" \
        || { echo "  .settings.base.json changed on idempotent run"; return 1; }

    backups_after3=$(find "$claude_home" -maxdepth 1 -name 'settings.json.bak.*' 2>/dev/null | wc -l | tr -d ' ')
    skips_after3=$(find "$claude_home" -maxdepth 1 -name 'settings-merge-skipped.*.json' 2>/dev/null | wc -l | tr -d ' ')
    [ "$backups_after3" -eq "$backups_after2" ] \
        || { echo "  backup count grew on idempotent run: $backups_after2 -> $backups_after3"; return 1; }
    [ "$skips_after3" -eq "$skips_after2" ] \
        || { echo "  skip-report count grew on idempotent run: $skips_after2 -> $skips_after3"; return 1; }
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

# Step 2: `playbook init` (no flags) must be invoked, with CLAUDE_PLUGIN_ROOT
# pointed at SELF_ROOT, whenever CLAUDE_HOME resolves to the default
# $HOME/.claude and a `playbook` binary is on PATH. The stub records its own
# args and CLAUDE_PLUGIN_ROOT to a file instead of doing anything real, since
# the actual wiring behaviour is covered at the Rust level (tests/init_run.rs)
# and, for the settings/hooks/statusline pipeline specifically, by scenario A
# and the golden differentials below; this only pins that setup-local.sh
# calls the binary correctly.
scenario_k_init_invoked() {
    local home="$WORK/k-home" ch="$WORK/k-home/.claude" bin="$WORK/k-bin" record out
    mkdir -p "$home" "$ch" "$bin"
    record="$WORK/k-record"
    cat > "$bin/playbook" <<EOF
#!/bin/sh
printf '%s\n' "\$*" > "$record"
printf 'CLAUDE_PLUGIN_ROOT=%s\n' "\$CLAUDE_PLUGIN_ROOT" >> "$record"
EOF
    chmod 0755 "$bin/playbook"

    out="$(CLAUDE_HOME="$ch" HOME="$home" PATH="$bin:/usr/bin:/bin:/usr/sbin:/sbin" \
           bash "$SCRIPT" --skip-deps 2>&1)"

    [ -f "$record" ] || { echo "playbook was never invoked: $out" >&2; return 1; }
    grep -q "^init$" "$record" \
        || { echo "unexpected invocation: $(cat "$record")" >&2; return 1; }
    grep -q "^CLAUDE_PLUGIN_ROOT=$SCRIPT_DIR/\.\.$" "$record" \
        || grep -q "^CLAUDE_PLUGIN_ROOT=$(cd "$SCRIPT_DIR/.." && pwd)$" "$record" \
        || { echo "CLAUDE_PLUGIN_ROOT not passed: $(cat "$record")" >&2; return 1; }
}

# `playbook init` must NOT fire when CLAUDE_HOME is not the default
# $HOME/.claude: `playbook init` has no CLAUDE_HOME override, so calling it
# would silently wire the wrong directory. This is the same skip-with-warning
# guard the old Step 2b already had, now widened to cover the settings merge
# too, not just the hooks fix: the accepted regression is that a non-default
# CLAUDE_HOME now skips the merge as well.
scenario_l_hooks_only_skipped_for_non_default_claude_home() {
    local home="$WORK/l-home" ch="$WORK/l-claude" bin="$WORK/l-bin" record out
    mkdir -p "$home" "$ch" "$bin"
    record="$WORK/l-record"
    cat > "$bin/playbook" <<EOF
#!/bin/sh
printf '%s\n' "\$*" > "$record"
EOF
    chmod 0755 "$bin/playbook"

    out="$(CLAUDE_HOME="$ch" HOME="$home" PATH="$bin:/usr/bin:/bin:/usr/sbin:/sbin" \
           bash "$SCRIPT" --skip-deps 2>&1)"

    [ ! -f "$record" ] || { echo "playbook was invoked against a non-default CLAUDE_HOME: $(cat "$record")" >&2; return 1; }
    case "$out" in
        *"skipping playbook init"*) ;;
        *) echo "expected a skip warning, got: $out" >&2; return 1 ;;
    esac
}

# ---------------------------------------------------------------------------
# (m) SYSTEM PROMPT REFRESH WITHOUT THE FLAG: disclosed side effect 2. A
#     machine that ever opted into --system-prompt has its installed
#     SYSTEM_PROMPT.md silently refreshed by a later PLAIN (no-flag) run too,
#     mirroring tests/init_system_prompt.rs's
#     system_prompt_false_but_existing_stale_copy_is_refreshed at the shell
#     level. A machine that never installed one stays untouched (absent).
# ---------------------------------------------------------------------------
scenario_m_system_prompt_refresh_without_flag() {
    local d home claude_home rc dest

    # Sub-case 1: a stale, previously-installed copy is refreshed by a plain
    # (no --system-prompt) run.
    d="$(mktemp -d "$WORK/sysprompt_refresh.XXXXXX")"
    home="$d/home"
    claude_home="$home/.claude"
    dest="$claude_home/prompts/SYSTEM_PROMPT.md"
    mkdir -p "$claude_home/prompts"
    printf 'a stale, hand-edited copy that predates the shipped prompt\n' > "$dest"

    PATH="$REAL_BIN_PATH" run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  refresh run rc=$rc"; return 1; }

    cmp -s "$dest" "$SCRIPT_DIR/../prompts/SYSTEM_PROMPT.md" \
        || { echo "  stale SYSTEM_PROMPT.md was not refreshed by a plain run"; return 1; }

    # Sub-case 2: a machine that never installed one stays absent.
    d="$(mktemp -d "$WORK/sysprompt_absent.XXXXXX")"
    home="$d/home"
    claude_home="$home/.claude"
    mkdir -p "$claude_home"

    PATH="$REAL_BIN_PATH" run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  fresh run rc=$rc"; return 1; }

    [ ! -f "$claude_home/prompts/SYSTEM_PROMPT.md" ] \
        || { echo "  SYSTEM_PROMPT.md installed on a plain run with no prior copy"; return 1; }
}

# ---------------------------------------------------------------------------
# (n) GOLDEN DIFFERENTIAL, CLEAN INSTALL: run the NEW flow (playbook init)
#     against an empty CLAUDE_HOME and compare its settings.json and
#     .settings.base.json against tests/fixtures/golden/setup-local.clean-install.json,
#     captured from TODAY's unmodified script before this WU's rewrite.
#
#     Compared semantically (jq -S, sorted keys), not byte-for-byte: a fresh
#     install's settings.json now has alphabetically-sorted top-level keys
#     instead of the template's own insertion order (disclosed side effect 3
#     in setup-local.sh's own comment), because `playbook init` always routes
#     a missing settings.json through the merge algorithm rather than a
#     verbatim template copy. Semantically identical; JSON objects are
#     unordered by spec and nothing reads settings.json positionally.
# ---------------------------------------------------------------------------
scenario_n_golden_clean_install() {
    local d home claude_home rc fixture expected_settings expected_base

    fixture="$GOLDEN_DIR/setup-local.clean-install.json"
    [ -f "$fixture" ] || { echo "  golden fixture missing: $fixture"; return 1; }

    d="$(mktemp -d "$WORK/golden_clean.XXXXXX")"
    home="$d/home"
    claude_home="$home/.claude"
    mkdir -p "$claude_home"
    expected_settings="$d/expected-settings.json"
    expected_base="$d/expected-base.json"
    # -j, not -r: -r appends a trailing newline after the raw string it
    # prints, which would double up the one already embedded in the field,
    # a spurious diff every jq -S semantic comparison here would swallow
    # silently at the value level anyway, but scenario O's byte diff below
    # would not.
    jq -j '.settings_json' "$fixture" > "$expected_settings"
    jq -j '.settings_base_json' "$fixture" > "$expected_base"

    PATH="$REAL_BIN_PATH" run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  rc=$rc"; return 1; }

    diff <(jq -S . "$claude_home/settings.json") <(jq -S . "$expected_settings") \
        || { echo "  settings.json diverged from the clean-install golden fixture (semantic diff)"; return 1; }
    diff <(jq -S . "$claude_home/.settings.base.json") <(jq -S . "$expected_base") \
        || { echo "  .settings.base.json diverged from the clean-install golden fixture (semantic diff)"; return 1; }
}

# ---------------------------------------------------------------------------
# (o) GOLDEN DIFFERENTIAL, SKIP-TRIGGERING: seed a settings.json customisation
#     that collides with a template update, run the NEW flow, and compare
#     against tests/fixtures/golden/setup-local.skip-triggering.json,
#     captured from TODAY's unmodified script. Both starting states go
#     through the merge algorithm on both the old and new side (unlike the
#     clean-install case above, which shortcuts to a verbatim copy on the old
#     side only), so this one IS byte-for-byte identical; kept as a literal
#     comparison rather than loosened to match scenario N.
#
#     Also confirms the skip-report/pruned-backup scheme from WU-0 is
#     reachable end to end through this shell wrapper, closing the gap
#     scenario F's additive-only seed leaves.
# ---------------------------------------------------------------------------
scenario_o_golden_skip_triggering() {
    local d home claude_home rc fixture expected_settings expected_base skip_count

    fixture="$GOLDEN_DIR/setup-local.skip-triggering.json"
    [ -f "$fixture" ] || { echo "  golden fixture missing: $fixture"; return 1; }

    d="$(mktemp -d "$WORK/golden_skip.XXXXXX")"
    home="$d/home"
    claude_home="$home/.claude"
    mkdir -p "$claude_home"
    expected_settings="$d/expected-settings.json"
    expected_base="$d/expected-base.json"
    # -j, not -r: see scenario N's comment on the same extraction. This
    # scenario's comparison below is a literal byte cmp, so the extra
    # trailing newline -r would add is not a false pass here, it is an
    # outright failure.
    jq -j '.settings_json' "$fixture" > "$expected_settings"
    jq -j '.settings_base_json' "$fixture" > "$expected_base"

    # The same collision the fixture was captured against: a customised
    # value the shipped template also updates.
    printf '{"cleanupPeriodDays": 999}\n' > "$claude_home/settings.json"

    PATH="$REAL_BIN_PATH" run_setup "$home" "$claude_home"; rc=$?
    [ "$rc" -eq 0 ] || { echo "  rc=$rc"; return 1; }

    cmp -s "$claude_home/settings.json" "$expected_settings" \
        || { echo "  settings.json diverged from the skip-triggering golden fixture (byte diff)"; return 1; }
    cmp -s "$claude_home/.settings.base.json" "$expected_base" \
        || { echo "  .settings.base.json diverged from the skip-triggering golden fixture (byte diff)"; return 1; }

    skip_count=$(find "$claude_home" -maxdepth 1 -name 'settings-merge-skipped.*.json' 2>/dev/null | wc -l | tr -d ' ')
    [ "$skip_count" -ge 1 ] \
        || { echo "  no skip-report file found after a genuine three-way collision"; return 1; }
}

# ---------------------------------------------------------------------------
# (p) WRAPPER RESILIENCE: `playbook init` failing must not abort the rest of
#     the script. Stubs the binary to always exit 1, simulating one of its
#     six internal steps failing, and confirms the `|| warn` wrapper is
#     actually in place: Steps 3 (deps, a no-op here under --skip-deps but
#     still reached, not aborted before), 4 (aliases), and 5 (system prompt)
#     all still run afterward, and the script itself exits 0.
# ---------------------------------------------------------------------------
scenario_p_init_failure_does_not_abort_later_steps() {
    local home="$WORK/p-home" ch="$WORK/p-home/.claude" bin="$WORK/p-bin" out rc
    mkdir -p "$home" "$ch" "$bin"
    printf '#!/bin/sh\nexit 1\n' > "$bin/playbook"
    chmod 0755 "$bin/playbook"

    out="$(SHELL=/bin/bash CLAUDE_HOME="$ch" HOME="$home" \
           PATH="$bin:/usr/bin:/bin:/usr/sbin:/sbin" \
           bash "$SCRIPT" --skip-deps --aliases --system-prompt 2>&1)"; rc=$?

    [ "$rc" -eq 0 ] || { echo "  script aborted (rc=$rc) instead of continuing past the failed step: $out"; return 1; }
    case "$out" in
        *"playbook init reported errors"*) ;;
        *) echo "  expected the || warn wrapper's message, got: $out" >&2; return 1 ;;
    esac

    # Step 4 (aliases) still ran.
    [ -f "$home/.bashrc" ] || { echo "  Step 4 did not run: .bashrc not written"; return 1; }
    grep -qF 'shell/bash/cc.sh' "$home/.bashrc" \
        || { echo "  Step 4 did not run: cc.sh source line missing"; return 1; }

    # Step 5 (system prompt) still ran.
    [ -f "$ch/prompts/SYSTEM_PROMPT.md" ] \
        || { echo "  Step 5 did not run: SYSTEM_PROMPT.md not installed"; return 1; }
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
run_scenario "K: playbook init is invoked with CLAUDE_PLUGIN_ROOT set"                          scenario_k_init_invoked
run_scenario "L: playbook init is skipped for a non-default CLAUDE_HOME"                        scenario_l_hooks_only_skipped_for_non_default_claude_home
run_scenario "M: a plain run refreshes an installed SYSTEM_PROMPT.md, leaves a never-installed one absent" scenario_m_system_prompt_refresh_without_flag
run_scenario "N: golden differential, clean install (semantic diff)"                            scenario_n_golden_clean_install
run_scenario "O: golden differential, skip-triggering (byte diff) + skip-report exists"         scenario_o_golden_skip_triggering
run_scenario "P: playbook init failing does not abort Steps 3/4/5 (|| warn wrapper)"             scenario_p_init_failure_does_not_abort_later_steps

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
