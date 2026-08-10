#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# uninstall.sh: remove shipped files from ~/.claude and clean the .zshrc
# launcher block.  Runtime state (sessions, history, credentials) is
# preserved by default.  Use --purge to also remove user config.
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
Remove shipped files from ~/.claude and clean the .zshrc launcher block.

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
  .gitignore  agents  Brewfile  CODE_OF_CONDUCT.md  commands  CONTRIBUTING.md
  docs  hooks  install.sh  LICENSE  Makefile  output-styles  permissions.shared.json
  prompts  README.md  SECURITY.md  settings.shared.json  shell  skills
  statusline.sh  uninstall.sh

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
SHIPPED=(
    .gitignore
    agents
    Brewfile
    CODE_OF_CONDUCT.md
    commands
    CONTRIBUTING.md
    docs
    hooks
    install.sh
    LICENSE
    Makefile
    output-styles
    permissions.shared.json
    prompts
    README.md
    SECURITY.md
    settings.shared.json
    shell
    skills
    statusline.sh
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

# --- Remove .zshrc launcher block ---
# Uses a same-directory tempfile to avoid a cross-filesystem EXDEV rename
# error that would silently leave the file unchanged.
# Matches both the pre-layout-move path (shell/cc.zsh) and the current path
# (shell/zsh/cc.zsh) so an install made before the shell/bash/zsh/shared
# reorganisation doesn't get left with a dead source line.
ZSHRC="$HOME/.zshrc"
if [ -f "$ZSHRC" ] && grep -qE 'shell/(zsh/)?cc\.zsh' "$ZSHRC"; then
    STAMP="$(date +%Y%m%d-%H%M%S)"
    cp "$ZSHRC" "${ZSHRC}.bak-${STAMP}"
    ZSHRC_TMP="$(mktemp "${HOME}/.zshrc.tmp.XXXXXX")"
    # Drop the source line and absorb the preceding launchers comment of
    # any variant (matches "launchers (cc/ccd)" broadly so it handles all
    # historical and current comment forms).  Only removes a comment when it
    # both matches launchers (cc/ccd) AND is immediately followed by the
    # source line.  Handles multiple occurrences and the no-comment case.
    # A second awk pass squeezes any resulting doubled blank line.
    awk '
      /source.*\/shell\/(zsh\/)?cc\.zsh/ {
        if (prev ~ /launchers \(cc\/ccd\)/) prev = ""
        if (prev != "") print prev
        prev = ""; next
      }
      { if (prev != "") print prev; prev = $0 }
      END { if (prev != "") print prev }
    ' "$ZSHRC" | awk '
      /^[[:space:]]*$/ { blank++; if (blank <= 1) print; next }
      { blank = 0; print }
    ' > "$ZSHRC_TMP"
    mv "$ZSHRC_TMP" "$ZSHRC"
    log "Removed cc.zsh launcher from .zshrc (backup: ${ZSHRC}.bak-${STAMP})"
else
    log ".zshrc: cc.zsh source line not found; nothing to remove"
fi

log "Done. Reload your shell or open a new terminal to apply changes."
