#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# setup-local.sh: idempotent local wiring for pragmatic-engineer/playbook.
# Copies the always-on safety-guard hooks, seeds or merges settings.json from
# the shipped template, and optionally installs deps (brew), the shell
# launchers (cc.sh/cc.zsh), and the system prompt.
#
# Self-locates its own source tree so it works when called from install.sh
# (after the file-copy loop) or directly from the /setup plugin command.
#
# Usage:  bash shell/setup-local.sh [--aliases] [--system-prompt] [--skip-deps] [--yes]
# Env:    CLAUDE_HOME  target directory (default: $HOME/.claude)
#
# Flags --skip-plugin and --skip-shell are accepted and silently ignored.
# Default (no --aliases, no --system-prompt): copy the 3 guard hooks and
# seed/merge settings.json only. The shell rc and launcher files are NOT
# touched unless --aliases is given.
set -euo pipefail

SELF_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
CLAUDE_HOME="${CLAUDE_HOME:-$HOME/.claude}"
SKIP_DEPS=0
OPT_ALIASES=0
OPT_SYSTEM_PROMPT=0
# ASSUME_YES is parsed for forward-compatibility; setup-local.sh has no
# interactive prompts of its own.
ASSUME_YES=0

if [ -t 1 ]; then
    C_B=$'\033[1;34m'; C_Y=$'\033[1;33m'; C_R=$'\033[1;31m'; C_0=$'\033[0m'
else
    C_B=""; C_Y=""; C_R=""; C_0=""
fi
log()  { printf '%s==>%s %s\n' "$C_B" "$C_0" "$*"; }
warn() { printf '%swarning:%s %s\n' "$C_Y" "$C_0" "$*" >&2; }
die()  { printf '%serror:%s %s\n'   "$C_R" "$C_0" "$*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    # SC2034: ASSUME_YES is set here but read nowhere, on purpose. See the note
    # above its declaration: the flag is accepted for forward-compatibility.
    # shellcheck disable=SC2034
    case "$1" in
        --skip-deps)     SKIP_DEPS=1 ;;
        --aliases)       OPT_ALIASES=1 ;;
        --system-prompt) OPT_SYSTEM_PROMPT=1; OPT_ALIASES=1 ;;
        --yes|-y)        ASSUME_YES=1 ;;
        --skip-plugin)   ;; # accepted, ignored -- wiring always runs
        --skip-shell)    ;; # accepted, ignored -- use --aliases to wire shell
        *)               die "unknown option: $1" ;;
    esac
    shift
done

[ -n "$CLAUDE_HOME" ] || die "CLAUDE_HOME is empty"

STAMP="$(date +%Y%m%d-%H%M%S)"
BACKUP="$CLAUDE_HOME/backups/setup-$STAMP"
backed_up=0

# ---------------------------------------------------------------------------
# 1. Copy the 3 always-on safety-guard hooks.
#    Overwriting is fine: these are product files, not user files.
#    Skip the copy if source and destination are the same physical file
#    (e.g. when SELF_ROOT == CLAUDE_HOME after install.sh ran the copy loop).
# ---------------------------------------------------------------------------
mkdir -p "$CLAUDE_HOME/hooks"
for g in rm-workspace-guard.sh bg-await-guard.sh no-dash-guard.sh; do
    src_hook="$SELF_ROOT/hooks/$g"
    dst_hook="$CLAUDE_HOME/hooks/$g"
    [ -f "$src_hook" ] || continue
    # -ef: same device + inode -> same file; self-copy would error on macOS.
    if [ "$src_hook" -ef "$dst_hook" ] 2>/dev/null; then
        continue
    fi
    cp "$src_hook" "$dst_hook"
done
log "Safety-guard hooks installed in $CLAUDE_HOME/hooks"

# ---------------------------------------------------------------------------
# 2. Seed or 3-way-merge settings.json from the shipped template.
#    The template wires the always-on guards; functional hooks live in the
#    plugin and must not be duplicated here (no double-fire).
#
#    Fresh install (no existing settings.json):
#      cp template -> settings.json; record as baseline in .settings.base.json.
#    Existing install:
#      3-way merge (baseline + template + user) preserving user customisations.
#    No template: no-op.
# ---------------------------------------------------------------------------
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
MERGE_TMP="$TMP/settings-merge"
mkdir -p "$MERGE_TMP"

if [ -f "$SELF_ROOT/settings.shared.json" ]; then
    if [ ! -e "$CLAUDE_HOME/settings.json" ]; then
        # Fresh install: seed settings.json and record the shipped baseline.
        cp "$SELF_ROOT/settings.shared.json" "$CLAUDE_HOME/settings.json"
        cp "$SELF_ROOT/settings.shared.json" "$CLAUDE_HOME/.settings.base.json"
        log "Seeded default settings.json from settings.shared.json"
    else
        # Existing install: 3-way merge.
        MERGE_BIN="$SELF_ROOT/shell/merge-settings.sh"
        MERGE_SKIP_TMP="$MERGE_TMP/settings-merge-skipped.json"
        if merged="$(bash "$MERGE_BIN" \
                "$CLAUDE_HOME/.settings.base.json" \
                "$SELF_ROOT/settings.shared.json" \
                "$CLAUDE_HOME/settings.json" \
                "$MERGE_TMP/newbase" \
                "$MERGE_SKIP_TMP" 2>/dev/null)"; then
            if printf '%s\n' "$merged" | cmp -s - "$CLAUDE_HOME/settings.json"; then
                # Idempotent: refresh base only; do not touch settings.json.
                mv "$MERGE_TMP/newbase" "$CLAUDE_HOME/.settings.base.json"
                log "settings.json already up to date"
            else
                # Content changed: snapshot, write, move skip file into backup.
                mkdir -p "$BACKUP"
                cp "$CLAUDE_HOME/settings.json" "$BACKUP/"
                mv "$MERGE_SKIP_TMP" "$BACKUP/settings-merge-skipped.json"
                printf '%s\n' "$merged" > "$MERGE_TMP/settings.json.new"
                mv "$MERGE_TMP/settings.json.new" "$CLAUDE_HOME/settings.json"
                mv "$MERGE_TMP/newbase" "$CLAUDE_HOME/.settings.base.json"
                backed_up=1
                _nw="$(jq 'length' "$BACKUP/settings-merge-skipped.json" 2>/dev/null)" \
                    || _nw='0'
                log "Merged settings.json (${_nw} keys withheld; see $BACKUP/settings-merge-skipped.json)"
                if [ "${_nw:-0}" -gt 0 ]; then
                    warn "Some customised keys were also updated by the new template."
                    warn "Review $BACKUP/settings-merge-skipped.json after setup."
                fi
            fi
            # Prune setup backup dirs older than the newest 5.
            find "$CLAUDE_HOME/backups" -maxdepth 1 -type d -name 'setup-*' \
                2>/dev/null | sort -r | tail -n +6 \
                | while IFS= read -r _old; do [ -n "$_old" ] && rm -rf "$_old"; done \
                || true
        else
            warn "settings.json merge failed; settings.json left unchanged."
            warn "If this persists, delete $CLAUDE_HOME/.settings.base.json to reset to additive merge."
        fi
    fi
fi

# ---------------------------------------------------------------------------
# 3. Install dependencies via Homebrew (unless --skip-deps).
# ---------------------------------------------------------------------------
if [ "$SKIP_DEPS" -eq 0 ]; then
    if command -v brew >/dev/null 2>&1; then
        if [ -f "$SELF_ROOT/Brewfile" ]; then
            log "Installing dependencies (brew bundle)"
            brew bundle --file "$SELF_ROOT/Brewfile" </dev/null \
                || warn "brew bundle reported errors"
        fi
    else
        warn "Homebrew not found; skipping deps. See https://brew.sh"
        warn "When Homebrew is available: brew bundle --file $SELF_ROOT/Brewfile"
    fi
fi

# ---------------------------------------------------------------------------
# 4. (--aliases) Copy the shell launcher runtime files and wire the rc file.
#    Copies every file/dir in shell/ EXCEPT *.test.sh files.
#    Uses an -ef self-copy guard per file. For regular files also checks
#    content equality (cmp -s) to report "already up to date" without re-copy.
#    Detects the user's shell from $SHELL (basename):
#      zsh  -> ~/.zshrc   sources $HOME/.claude/shell/zsh/cc.zsh
#      bash -> ~/.bashrc  sources $HOME/.claude/shell/bash/cc.sh
#    Idempotent: grep -qF guard before appending to the rc file.
# ---------------------------------------------------------------------------
if [ "$OPT_ALIASES" -eq 1 ]; then
    CLAUDE_SHELL_DIR="$CLAUDE_HOME/shell"
    SELF_SHELL_DIR="$SELF_ROOT/shell"
    mkdir -p "$CLAUDE_SHELL_DIR"

    for src in "$SELF_SHELL_DIR"/*; do
        name="$(basename "$src")"
        case "$name" in
            *.test.sh) continue ;;
        esac
        dst="$CLAUDE_SHELL_DIR/$name"
        # Self-copy guard: same device+inode means SELF_ROOT == CLAUDE_HOME.
        if [ "$src" -ef "$dst" ] 2>/dev/null; then
            log "shell/$name ... already up to date"
            continue
        fi
        if [ -d "$src" ]; then
            cp -R "$src" "$dst"
            log "shell/$name/ ... installed"
        elif [ -f "$dst" ] && cmp -s "$src" "$dst" 2>/dev/null; then
            log "shell/$name ... already up to date"
        else
            cp "$src" "$dst"
            log "shell/$name ... installed"
        fi
    done

    # Detect shell and wire the appropriate rc file.
    _SHELL_BIN="$(basename "${SHELL:-}")"
    case "$_SHELL_BIN" in
        zsh)
            RC_FILE="$HOME/.zshrc"
            # shellcheck disable=SC2016
            SOURCE_LINE='source "$HOME/.claude/shell/zsh/cc.zsh"'
            GREP_PAT='shell/zsh/cc.zsh'
            OLD_GREP_PAT='shell/cc.zsh'
            ;;
        bash)
            RC_FILE="$HOME/.bashrc"
            # shellcheck disable=SC2016
            SOURCE_LINE='source "$HOME/.claude/shell/bash/cc.sh"'
            GREP_PAT='shell/bash/cc.sh'
            OLD_GREP_PAT='shell/cc.sh'
            ;;
        *)
            warn "Shell '$_SHELL_BIN' not recognised; source the launcher manually."
            warn "For zsh:  source \"\$HOME/.claude/shell/zsh/cc.zsh\" in ~/.zshrc"
            warn "For bash: source \"\$HOME/.claude/shell/bash/cc.sh\" in ~/.bashrc"
            _SHELL_BIN=""
            ;;
    esac

    if [ -n "$_SHELL_BIN" ]; then
        # Migrate a pre-reorganisation source line to the current path. The
        # new-path guard below cannot see the old form, because shell/cc.zsh is
        # not a substring of shell/zsh/cc.zsh (nor shell/cc.sh of
        # shell/bash/cc.sh). Without this step a re-run appends a second line
        # and the launcher gets sourced twice: once through the transitional
        # shim at the old path, once directly.
        if [ -f "$RC_FILE" ] && grep -qF "$OLD_GREP_PAT" "$RC_FILE" 2>/dev/null; then
            cp "$RC_FILE" "${RC_FILE}.bak-${STAMP}"
            RC_TMP="$(mktemp "${RC_FILE}.tmp.XXXXXX")"
            # Drop the old source line and absorb the launchers comment that
            # immediately precedes it, then squeeze the doubled blank line the
            # removal leaves behind. Same shape as uninstall.sh's remover.
            # has: an explicit "prev holds a line" flag. Using prev != "" as the
            # sentinel instead would treat a buffered blank line as nothing
            # buffered and silently eat the user's blank lines around the block.
            awk -v pat="$OLD_GREP_PAT" '
              index($0, pat) {
                if (has && prev ~ /launchers \(cc\/ccd\)/) has = 0
                if (has) print prev
                has = 0; next
              }
              { if (has) print prev; prev = $0; has = 1 }
              END { if (has) print prev }
            ' "$RC_FILE" | awk '
              /^[[:space:]]*$/ { blank++; if (blank <= 1) print; next }
              { blank = 0; print }
            ' > "$RC_TMP"
            mv -f "$RC_TMP" "$RC_FILE"
            log "Migrated the old launcher line in $RC_FILE (backup: ${RC_FILE}.bak-${STAMP})"
        fi
        if grep -qF "$GREP_PAT" "$RC_FILE" 2>/dev/null; then
            log "$RC_FILE already sources the launcher ... already up to date"
        else
            printf '\n# playbook launchers (cc/ccd)\n%s\n' "$SOURCE_LINE" >> "$RC_FILE"
            log "Added launcher source line to $RC_FILE"
        fi
    fi
fi

# ---------------------------------------------------------------------------
# 5. (--system-prompt) Copy SYSTEM_PROMPT.md to CLAUDE_HOME/prompts/.
#    Implied by --system-prompt; --aliases runs first.
#    -ef guard prevents self-copy when SELF_ROOT == CLAUDE_HOME.
#    cmp -s guard prevents unnecessary overwrites on re-run.
# ---------------------------------------------------------------------------
if [ "$OPT_SYSTEM_PROMPT" -eq 1 ]; then
    SRC_PROMPT="$SELF_ROOT/prompts/SYSTEM_PROMPT.md"
    DST_PROMPT_DIR="$CLAUDE_HOME/prompts"
    DST_PROMPT="$DST_PROMPT_DIR/SYSTEM_PROMPT.md"
    if [ -f "$SRC_PROMPT" ]; then
        mkdir -p "$DST_PROMPT_DIR"
        if [ "$SRC_PROMPT" -ef "$DST_PROMPT" ] 2>/dev/null; then
            log "prompts/SYSTEM_PROMPT.md ... already up to date"
        elif [ -f "$DST_PROMPT" ] && cmp -s "$SRC_PROMPT" "$DST_PROMPT" 2>/dev/null; then
            log "prompts/SYSTEM_PROMPT.md ... already up to date"
        else
            cp "$SRC_PROMPT" "$DST_PROMPT"
            log "prompts/SYSTEM_PROMPT.md ... installed"
        fi
    else
        warn "prompts/SYSTEM_PROMPT.md not found at $SRC_PROMPT; skipping."
    fi
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
log "Setup complete."
[ "$backed_up" -eq 1 ] && log "Replaced files backed up to: $BACKUP" || true
