#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# uninstall.sh: remove shipped files from ~/.claude and clean the launcher
# block from .zshrc and .bashrc.  Runtime state (sessions, history,
# credentials) is preserved by default.  Use --purge to also remove user
# config.
#
# Usage:
#   bash ~/.claude/uninstall.sh [--yes] [--force] [--purge]
#
# Flags:
#   --yes     skip the confirmation prompt
#   --force   bypass the git-repo guard (development environments only)
#   --purge   also remove settings.json, .settings.base.json, and backups/
#
# Git note: if CLAUDE_HOME is a git working tree, raw rm leaves index entries
# dangling.  The git-correct path for decommissioning is: git rm -r <entries>
# Pass --force to bypass the guard if you know what you are doing.
set -euo pipefail

CLAUDE_HOME="${CLAUDE_HOME:-$HOME/.claude}"
FORCE=0
YES=0
PURGE=0

if [ -t 1 ]; then
    C_B=$'\033[1;34m'; C_Y=$'\033[1;33m'; C_R=$'\033[1;31m'; C_0=$'\033[0m'
else
    C_B=""; C_Y=""; C_R=""; C_0=""
fi
log()  { printf '%s==>%s %s\n' "$C_B" "$C_0" "$*"; }
warn() { printf '%swarning:%s %s\n' "$C_Y" "$C_0" "$*" >&2; }
die()  { printf '%serror:%s %s\n' "$C_R" "$C_0" "$*" >&2; exit 1; }

print_help() {
    cat <<'EOF'
Remove shipped files from ~/.claude and clean the launcher block from
.zshrc and .bashrc.

Usage:
  bash ~/.claude/uninstall.sh [--yes] [--force] [--purge]

Env:
  CLAUDE_HOME=<dir>   target directory (default: $HOME/.claude)

Flags:
  --yes     skip the confirmation prompt
  --purge   also remove settings.json, .settings.base.json, and backups/
  --force   bypass the git-repo guard (for git-managed ~/.claude only)
  -h,--help show this help

What is removed (allowlist only):
  .claude-plugin  .gitignore  agents  Brewfile  Cargo.lock  Cargo.toml
  CODE_OF_CONDUCT.md  commands  CONTRIBUTING.md  docs  hooks  install.sh
  LICENSE  justfile  Makefile  output-styles  permissions.shared.json  prompts
  README.md  ruff.toml  SECURITY.md  settings.shared.json  shell  skills
  src  statusline.sh  tests  uninstall.sh

What is preserved by default:
  settings.json  .settings.base.json  backups/  sessions/  projects/
  history*  plugins/  memory/  plans/  runtime/  cache/  logs/  todos/
  shell-snapshots/  .credentials*  cc-state/  ccd-state/

Git note: if ~/.claude is a git working tree, raw rm leaves index entries
dangling.  Use git rm -r <entries> for a proper decommission.  Pass --force
to bypass this guard.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --force) FORCE=1 ;;
        --yes)   YES=1 ;;
        --purge) PURGE=1 ;;
        -h|--help) print_help; exit 0 ;;
        *) die "unknown option: $1 (try --help)" ;;
    esac
    shift
done

# --- Validate CLAUDE_HOME before any destructive action ---
[ -n "$CLAUDE_HOME" ] || die "CLAUDE_HOME is empty"
case "$CLAUDE_HOME" in
    /*) ;;
    *) die "CLAUDE_HOME must be an absolute path: $CLAUDE_HOME" ;;
esac

# --- Git-repo guard ---
# Raw rm inside a git working tree leaves index entries dangling.  Refuse
# unless --force is passed.  --force bypasses ONLY this guard; it does not
# imply --yes.
if git -C "$CLAUDE_HOME" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    if [ "$FORCE" -eq 0 ]; then
        die "$CLAUDE_HOME is a git working tree. Raw rm leaves index entries dangling. For a real decommission, use: git rm -r <entries>. Pass --force to bypass this guard."
    fi
    warn "Git guard bypassed via --force. Proceeding with raw removal in a git working tree."
fi

# --- Shipped-entry allowlist ---
# Only these entries are ever removed.  CLAUDE_HOME itself is never touched.
# Must list everything install.sh copies, or an uninstall strands it. install.sh
# derives its copy set dynamically while this is hardcoded, so the two drift on
# every new repo-root file; six had already accumulated. Drift is now caught by
# shell/install-uninstall-roundtrip.test.sh, which runs the real pair.
#
# agents, commands and skills are deliberately still here even though install.sh
# no longer copies them: they are plugin-owned now, and this clears the residue
# of older direct installs.
SHIPPED=(
    .claude-plugin
    .gitignore
    agents
    Brewfile
    Cargo.lock
    Cargo.toml
    CODE_OF_CONDUCT.md
    commands
    CONTRIBUTING.md
    docs
    hooks
    install.sh
    LICENSE
    justfile
    # Makefile stays listed even though the repo no longer ships one. Every
    # install before the justfile migration copied it into CLAUDE_HOME, so
    # dropping it here would strand a real file on every existing machine.
    Makefile
    output-styles
    permissions.shared.json
    prompts
    README.md
    ruff.toml
    SECURITY.md
    settings.shared.json
    shell
    skills
    src
    statusline.sh
    tests
    uninstall.sh
)

# --- Confirmation prompt (skipped by --yes) ---
if [ "$YES" -eq 0 ]; then
    printf 'This will remove shipped config files from: %s\n' "$CLAUDE_HOME"
    printf 'Runtime state (sessions, history, credentials) is preserved.\n'
    if [ "$PURGE" -eq 1 ]; then
        printf 'With --purge: settings.json, .settings.base.json, and backups/ will also be removed.\n'
    fi
    printf 'Continue? [y/N] '
    read -r reply
    case "$reply" in
        y|Y|yes|YES) ;;
        *) die "Aborted." ;;
    esac
fi

# --- Remove shipped entries ---
log "Removing shipped entries from $CLAUDE_HOME"
for entry in "${SHIPPED[@]}"; do
    target="$CLAUDE_HOME/$entry"
    if [ -e "$target" ] || [ -L "$target" ]; then
        rm -rf "$target"
    fi
done

# --- Purge user config (--purge only) ---
if [ "$PURGE" -eq 1 ]; then
    log "Purging user config"
    for f in settings.json .settings.base.json backups; do
        target="$CLAUDE_HOME/$f"
        if [ -e "$target" ] || [ -L "$target" ]; then
            rm -rf "$target"
        fi
    done
fi

# --- Remove launcher source lines from rc files ---
# Shared helper: strips one shell's launcher source line, and the
# "launchers (cc/ccd)" comment immediately preceding it, from the given rc
# file.  Called once for .zshrc and once for .bashrc so the removal logic
# lives in exactly one place instead of two drifting copies.
#
# Uses a same-directory tempfile to avoid a cross-filesystem EXDEV rename
# error that would silently leave the file unchanged.
# path_pat matches both the pre-layout-move path (e.g. shell/cc.zsh) and the
# current path (e.g. shell/zsh/cc.zsh) so an install made before the
# shell/bash/zsh/shared reorganisation doesn't get left with a dead source
# line.  Only removes a comment when it both matches launchers (cc/ccd) AND
# is immediately followed by the source line.  Handles multiple occurrences
# and the no-comment case.  A second awk pass squeezes any resulting doubled
# blank line.
#
# has: an explicit "a line is buffered" flag.  Using prev != "" as the
# sentinel instead cannot tell "nothing buffered" apart from "buffered a
# blank line", and silently eats the user's blank lines around the block.
strip_rc_launcher() {
    local rc="$1" path_pat="$2" label="$3"
    local stamp rc_tmp

    if [ -f "$rc" ] && grep -qE "$path_pat" "$rc"; then
        stamp="$(date +%Y%m%d-%H%M%S)"
        cp "$rc" "${rc}.bak-${stamp}"
        rc_tmp="$(mktemp "${rc}.tmp.XXXXXX")"
        awk -v pat="source.*${path_pat}" '
          $0 ~ pat {
            if (has && prev ~ /launchers \(cc\/ccd\)/) has = 0
            if (has) print prev
            has = 0; next
          }
          { if (has) print prev; prev = $0; has = 1 }
          END { if (has) print prev }
        ' "$rc" | awk '
          /^[[:space:]]*$/ { blank++; if (blank <= 1) print; next }
          { blank = 0; print }
        ' > "$rc_tmp"
        mv "$rc_tmp" "$rc"
        log "Removed $label launcher from $(basename "$rc") (backup: ${rc}.bak-${stamp})"
    else
        log "$(basename "$rc"): $label source line not found; nothing to remove"
    fi
}

strip_rc_launcher "$HOME/.zshrc"  'shell/(zsh/)?cc\.zsh'   'cc.zsh'
strip_rc_launcher "$HOME/.bashrc" 'shell/(bash/)?cc\.sh'   'cc.sh'

# --- Remove the release binary and its PATH line ---
# install.sh places the binary outside CLAUDE_HOME, so the SHIPPED sweep above
# cannot reach it. Whatever the installer creates, this must remove, or
# uninstall stops being a true inverse and leaves a stale binary that
# `playbook --version` still answers from.
BIN_DIR="${PLAYBOOK_BIN_DIR:-$HOME/.local/bin}"
if [ -f "$BIN_DIR/playbook" ]; then
    rm -f "$BIN_DIR/playbook"
    log "Removed $BIN_DIR/playbook"
else
    log "No playbook binary at $BIN_DIR; nothing to remove"
fi

# Strips the two-line block install.sh appends: the "# playbook binary"
# marker and the export immediately after it. Anchored on the marker rather
# than on any `export PATH` line, so a user's own PATH edits are never touched.
strip_rc_binary_path() {
    local rc="$1" stamp rc_tmp
    [ -f "$rc" ] && grep -qF '# playbook binary' "$rc" || {
        log "$(basename "$rc"): no playbook binary PATH line; nothing to remove"
        return 0
    }
    stamp="$(date +%Y%m%d-%H%M%S)"
    cp "$rc" "${rc}.bak-${stamp}"
    rc_tmp="$(mktemp "${rc}.tmp.XXXXXX")"
    awk '
      /^# playbook binary$/ { skip = 1; next }
      skip && /^export PATH=/ { skip = 0; next }
      { skip = 0; print }
    ' "$rc" > "$rc_tmp"
    mv "$rc_tmp" "$rc"
    log "Removed playbook binary PATH line from $(basename "$rc") (backup: ${rc}.bak-${stamp})"
}

strip_rc_binary_path "$HOME/.zshrc"
strip_rc_binary_path "$HOME/.bashrc"

log "Done. Reload your shell or open a new terminal to apply changes."
