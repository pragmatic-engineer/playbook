#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# setup-local.sh: idempotent local wiring for pragmatic-engineer/playbook.
# Bootstraps the `playbook` binary, then seeds or merges settings.json and
# wires every guard/functional hook via `playbook init`, and optionally
# installs deps (brew), the shell launchers (cc.sh/cc.zsh), and the system
# prompt.
#
# Self-locates its own source tree so it works when called from install.sh
# (after the file-copy loop) or directly from the /playbook:setup plugin command.
#
# Usage:  bash shell/setup-local.sh [--aliases] [--system-prompt] [--skip-deps] [--yes]
# Env:    CLAUDE_HOME  target directory (default: $HOME/.claude)
#
# Flags --skip-plugin and --skip-shell are accepted and silently ignored.
# Default (no --aliases, no --system-prompt): seed/merge settings.json and
# wire guards/hooks via `playbook init` only. The shell rc and launcher
# files are NOT touched unless --aliases is given.
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

# ---------------------------------------------------------------------------
# 0. Ensure the `playbook` binary exists.
#
# Every ported hook is a bare `playbook hook <name>` command, so without the
# binary all 17 are dead and `/playbook:doctor` reports the guards as unwired.
# Neither `claude plugin install` nor this script used to install it, which made
# the README's primary path produce a half-broken install: plugin content worked
# and every ported hook silently did nothing.
#
# The download mirrors install.sh's `install_release_binary`, deliberately
# duplicated rather than shared. install.sh is the proven primary path and this
# script is scheduled for deletion by ADR 0007 WU-14, so breaking the working
# installer costs more than a temporary second copy. Keep the two in step until
# this file goes.
# ---------------------------------------------------------------------------
PLAYBOOK_BIN_DIR="${PLAYBOOK_BIN_DIR:-$HOME/.local/bin}"

ensure_playbook_binary() {
    local suffix os arch tag asset stage
    if command -v playbook >/dev/null 2>&1; then
        log "binary: ok ($(command -v playbook))"
        return 0
    fi
    if [ -x "$PLAYBOOK_BIN_DIR/playbook" ]; then
        PATH="$PLAYBOOK_BIN_DIR:$PATH"; export PATH
        log "binary: ok ($PLAYBOOK_BIN_DIR/playbook, was not yet on PATH)"
        return 0
    fi

    os="$(uname -s)"; arch="$(uname -m)"
    case "$os-$arch" in
        Darwin-arm64)  suffix="aarch64-apple-darwin" ;;
        Darwin-x86_64) suffix="x86_64-apple-darwin" ;;
        Linux-aarch64) suffix="aarch64-unknown-linux-musl" ;;
        Linux-x86_64)  suffix="x86_64-unknown-linux-musl" ;;
        *) warn "binary: unsupported platform $os $arch; install it manually"; return 1 ;;
    esac

    tag="$(curl -fsSL https://api.github.com/repos/pragmatic-engineer/playbook/releases/latest 2>/dev/null |
           sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
    [ -n "$tag" ] || { warn "binary: could not resolve the latest release"; return 1; }

    asset="playbook-${tag#v}-${suffix}"
    stage="$(mktemp -d)"
    log "binary: fetching $asset ($tag)"

    if ! curl -fsSL "https://github.com/pragmatic-engineer/playbook/releases/download/$tag/$asset" -o "$stage/$asset"; then
        warn "binary: download failed"; return 1
    fi
    # SHA256SUMS is not signed. Its integrity rests on TLS and on trusting
    # github.com, not on any signature. Verify anyway, so a truncated or corrupt
    # download is caught before it is made executable. An unverifiable download
    # is refused rather than installed.
    if ! curl -fsSL "https://github.com/pragmatic-engineer/playbook/releases/download/$tag/SHA256SUMS" -o "$stage/SHA256SUMS"; then
        warn "binary: SHA256SUMS unavailable; refusing to install unverified"; return 1
    fi
    if ! ( cd "$stage" &&
           grep -E "^[0-9a-f]{64}  ${asset}$" SHA256SUMS > "$asset.sha256" &&
           { shasum -a 256 -c "$asset.sha256" >/dev/null 2>&1 ||
             sha256sum -c "$asset.sha256" >/dev/null 2>&1; } ); then
        warn "binary: checksum mismatch for $asset; refusing to install"; return 1
    fi

    mkdir -p "$PLAYBOOK_BIN_DIR"
    chmod 0755 "$stage/$asset"
    mv "$stage/$asset" "$PLAYBOOK_BIN_DIR/playbook"
    PATH="$PLAYBOOK_BIN_DIR:$PATH"; export PATH
    log "binary: installed $tag to $PLAYBOOK_BIN_DIR/playbook"
    warn "binary: open a new shell so $PLAYBOOK_BIN_DIR resolves on PATH"
}

ensure_playbook_binary ||
    warn "binary: missing; the 17 ported hooks will not run until it is installed"

STAMP="$(date +%Y%m%d-%H%M%S)"

# ---------------------------------------------------------------------------
# 2. Seed or 3-way-merge settings.json from the shipped template, and rewire
#    every guard and functional hook, in one `playbook init` call. This
#    replaces both the old python 3-way merge (merge-settings.py) and the old
#    `playbook init --hooks-only` patch-up: `playbook init`'s full pipeline
#    already reconciles settings and hooks together in one pass, with hooks
#    upserted per-entry rather than the python merge's whole-key policy
#    (which kept a whole customised `.hooks` key, stale guard commands
#    included, rather than adopting the template's fixed ones).
#
#    Disclosed side effects, all already accepted:
#      1. Every default run now also places statusline.sh: the python merge
#         never did, but `playbook init`'s full pipeline always does.
#      2. A machine that ever opted into --system-prompt has its installed
#         SYSTEM_PROMPT.md silently refreshed on every later plain (no-flag)
#         run too, not just on --system-prompt runs.
#      3. A fresh install's settings.json now has alphabetically-sorted
#         top-level keys instead of the template's own insertion order,
#         because `playbook init` always routes a missing settings.json
#         through the merge algorithm rather than a verbatim template copy.
#         Semantically identical (JSON objects are unordered by spec;
#         nothing reads settings.json positionally) -- noted here so it is
#         not rediscovered as a bug later.
#
#    `playbook init` has no `CLAUDE_HOME` override of its own; it always
#    targets `$HOME/.claude`. Only run it when this script's own `CLAUDE_HOME`
#    (which does support the override) resolves to that same default path,
#    so a non-default `CLAUDE_HOME` is left unmerged rather than silently
#    rewired at the wrong location. The accepted regression this widens: a
#    non-default `CLAUDE_HOME` now skips the settings merge too, not just the
#    hooks fix the old two-step dance used to still apply.
#
#    Wrapped with `|| warn`, not a bare call: `playbook init` exits 1 if ANY
#    of its six internal steps fails (settings, guards, hooks, shim,
#    statusline, system-prompt), and this script runs under `set -euo
#    pipefail`, so a bare call would abort Steps 3/4/5 the moment one
#    unrelated step errors.
# ---------------------------------------------------------------------------
if [ "$CLAUDE_HOME" != "$HOME/.claude" ]; then
    warn "CLAUDE_HOME is not \$HOME/.claude; skipping playbook init (it has no CLAUDE_HOME override)"
elif command -v playbook >/dev/null 2>&1; then
    CLAUDE_PLUGIN_ROOT="$SELF_ROOT" playbook init \
        || warn "playbook init reported errors; settings merge and/or hooks may be incomplete"
else
    warn "playbook binary unavailable; settings.json and guards may be unwired. Re-run /playbook:setup once it is installed."
fi

# ---------------------------------------------------------------------------
# 3. Install dependencies (unless --skip-deps).
#    Per-dependency check-then-install: for each formula in the Brewfile, use
#    the version already on PATH (from any source: brew, nvm, pyenv, system, a
#    manual install) and install via brew only when the tool is missing. This
#    replaces a blanket `brew bundle`, which would force a brew formula even
#    when the tool is already installed from somewhere else.
# ---------------------------------------------------------------------------
if [ "$SKIP_DEPS" -eq 0 ]; then
    if [ -f "$SELF_ROOT/shell/ensure-deps.sh" ]; then
        # shellcheck source=shell/ensure-deps.sh
        . "$SELF_ROOT/shell/ensure-deps.sh"
        log "Checking dependencies (install only what is missing)"
        ensure_all_deps "$SELF_ROOT/Brewfile" || warn "one or more dependency installs reported errors"
    elif command -v brew >/dev/null 2>&1 && [ -f "$SELF_ROOT/Brewfile" ]; then
        # Fallback for a partial checkout without ensure-deps.sh.
        log "Installing dependencies (brew bundle)"
        brew bundle --file "$SELF_ROOT/Brewfile" </dev/null \
            || warn "brew bundle reported errors"
    else
        warn "Cannot resolve dependencies (no ensure-deps.sh and no brew). See https://brew.sh"
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
