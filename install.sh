#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# Installer for pragmatic-engineer/playbook. Plugin based: the toolkit (skills,
# commands, agents, and the functional hooks) is delivered as a Claude Code
# plugin, while this script installs the always-on safety guards and the other
# local configs (settings.json, statusline, shell integration, dependencies).
# Interactive by default; every optional step asks before it runs.
#
#   curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
#
# Source of truth: the latest GitHub release by default, or PLAYBOOK_REF
# (any tag/branch/sha). Falls back to the main branch when no release exists.
# Existing tracked files are backed up before being replaced; runtime state
# (sessions/, projects/, history, plugins/) is never touched.
set -euo pipefail

PLUGIN_REPO="pragmatic-engineer/playbook"
MARKETPLACE="pragmatic-engineer/marketplace"
PLUGIN="playbook@pragmatic-engineer"
CLAUDE_HOME="${CLAUDE_HOME:-$HOME/.claude}"
REF="${PLAYBOOK_REF:-}"
SKIP_DEPS=0
SKIP_PLUGIN=0
OPT_ALIASES=0
OPT_SYSTEM_PROMPT=0
ASSUME_YES=0

# Component dirs the plugin owns: never copied into ~/.claude directly, so the
# plugin stays the single source and skills/commands/agents don't load twice.
PLUGIN_DIRS="skills commands agents"

if [ -t 1 ]; then
    C_B=$'\033[1;34m'; C_Y=$'\033[1;33m'; C_R=$'\033[1;31m'; C_0=$'\033[0m'
else
    C_B=""; C_Y=""; C_R=""; C_0=""
fi
log()  { printf '%s==>%s %s\n' "$C_B" "$C_0" "$*"; }
warn() { printf '%swarning:%s %s\n' "$C_Y" "$C_0" "$*" >&2; }
die()  { printf '%serror:%s %s\n' "$C_R" "$C_0" "$*" >&2; exit 1; }

# ask PROMPT [default]  ->  0 = yes, 1 = no. Default is yes unless the second
# argument is "n". With --yes, or when there is no controlling terminal (a
# non-interactive `curl | bash` in CI, say), the default is taken without
# prompting. Prompts read from /dev/tty so they work under `curl | bash`.
ask() {
    local prompt="$1" def="${2:-Y}" ans hint
    if [ "$ASSUME_YES" -eq 1 ] || [ ! -e /dev/tty ]; then
        [ "$def" = "n" ] && return 1 || return 0
    fi
    if [ "$def" = "n" ]; then hint="y/N"; else hint="Y/n"; fi
    printf '%s %s[%s]%s ' "$prompt" "$C_Y" "$hint" "$C_0" > /dev/tty
    read -r ans < /dev/tty || ans=""
    [ -z "$ans" ] && ans="$def"
    case "$ans" in
        [Yy]|[Yy][Ee][Ss]) return 0 ;;
        *) return 1 ;;
    esac
}

print_help() {
    cat <<'EOF'
Install pragmatic-engineer/playbook. Plugin based, interactive by default.

Usage:
  curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
  curl -fsSL .../install.sh | bash -s -- [flags]
  ./install.sh [flags]

What it does:
  - installs the toolkit as a Claude Code plugin (adds the marketplace, installs
    and enables playbook), which provides the skills, commands, agents,
    and functional hooks;
  - installs the always-on safety guards (rm, background-await, dash guards) and
    the other local configs (settings.json, statusline, deps);
  - optionally installs the shell launchers (cc/ccd) and the custom system prompt.

Env:
  PLAYBOOK_REF=<tag|branch|sha>  source ref (default: latest release, else main)
  CLAUDE_HOME=<dir>              install target (default: $HOME/.claude)

Flags:
  --yes              non-interactive: accept every step's default
  --skip-plugin      don't add the marketplace or install the plugin
  --skip-deps        skip 'brew bundle'
  --aliases          install the shell launchers (cc/ccd) without prompting
  --system-prompt    install the custom system prompt without prompting (implies --aliases)
  --no-setup         install files only: no plugin, deps, or shell edits
  --ref <ref>        same as PLAYBOOK_REF
  -h, --help         show this help
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --yes|-y)        ASSUME_YES=1 ;;
        --skip-plugin)   SKIP_PLUGIN=1 ;;
        --skip-deps)     SKIP_DEPS=1 ;;
        --skip-shell)    ;; # accepted, ignored -- use --aliases to wire shell
        --aliases)       OPT_ALIASES=1 ;;
        --system-prompt) OPT_SYSTEM_PROMPT=1; OPT_ALIASES=1 ;;
        --no-setup)      SKIP_DEPS=1; SKIP_PLUGIN=1 ;;
        --ref)           shift; REF="${1:-}" ;;
        --ref=*)         REF="${1#--ref=}" ;;
        -h|--help)       print_help; exit 0 ;;
        *)               die "unknown option: $1 (try --help)" ;;
    esac
    shift
done

[ -n "$CLAUDE_HOME" ] || die "CLAUDE_HOME is empty"
command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar  >/dev/null 2>&1 || die "tar is required"

# Only a 404 ("this repo published no release") may fall back to main. Any
# other failure means we do not know what the latest release is, and quietly
# installing main would swap a tagged release for in-progress branch work.
# Hence no -f, which would empty the body and collapse 403, 404 and a dead
# network into one indistinguishable empty string. The 403 is routine: the
# unauthenticated API allows 60 requests per hour per IP.
resolve_tarball_url() {
    if [ -n "$REF" ]; then
        printf 'https://codeload.github.com/%s/tar.gz/%s\n' "$PLUGIN_REPO" "$REF"
        return
    fi
    local body code tag
    body="$(mktemp)"
    if ! code="$(curl -sSL -o "$body" -w '%{http_code}' \
        "https://api.github.com/repos/$PLUGIN_REPO/releases/latest" 2>/dev/null)"; then
        code="000"
    fi
    tag="$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$body" 2>/dev/null || true)"
    tag="${tag%%$'\n'*}"
    rm -f "$body"

    # die runs in this command substitution's subshell, so its exit only ends
    # the subshell. That still aborts the install, because `set -e` makes the
    # enclosing assignment inherit the non-zero status.
    case "$code" in
        200)
            [ -n "$tag" ] || die "the release API returned 200 with no tag_name; refusing to guess a version"
            printf 'https://codeload.github.com/%s/tar.gz/refs/tags/%s\n' "$PLUGIN_REPO" "$tag"
            ;;
        404)
            warn "$PLUGIN_REPO has published no release; installing from the main branch"
            printf 'https://codeload.github.com/%s/tar.gz/refs/heads/main\n' "$PLUGIN_REPO"
            ;;
        *)
            die "could not read the release API (HTTP $code); retry, or pin a version with PLAYBOOK_REF=vX.Y.Z"
            ;;
    esac
}

# PLAYBOOK_SRC is a test seam: when set, install straight from a local
# checkout and skip the network path (resolve/curl/tar) entirely.
SRC="${PLAYBOOK_SRC:-}"
if [ -n "$SRC" ]; then
    [ -d "$SRC" ] || die "PLAYBOOK_SRC is not a directory: $SRC"
    log "Installing from local source $SRC"
else
    TMP="$(mktemp -d)"
    trap 'rm -rf "$TMP"' EXIT

    url="$(resolve_tarball_url)"
    log "Downloading $url"
    curl -fsSL "$url" -o "$TMP/config.tar.gz" || die "download failed: $url"
    tar -xzf "$TMP/config.tar.gz" -C "$TMP" || die "could not extract archive"

    SRC="$(find "$TMP" -mindepth 1 -maxdepth 1 -type d -name '*playbook*' | head -1)"
    [ -n "$SRC" ] || SRC="$(find "$TMP" -mindepth 1 -maxdepth 1 -type d | head -1)"
    [ -d "$SRC" ] || die "could not locate extracted source directory"
fi

mkdir -p "$CLAUDE_HOME"
STAMP="$(date +%Y%m%d-%H%M%S)"
BACKUP="$CLAUDE_HOME/backups/install-$STAMP"
backed_up=0

log "Installing config into $CLAUDE_HOME"
shopt -s dotglob nullglob
for src in "$SRC"/*; do
    name="$(basename "$src")"
    case "$name" in
        .git|.github|.DS_Store) continue ;;
        settings.json) continue ;;  # never clobber a user's live settings
        skills|commands|agents) continue ;;  # plugin-owned; installed via the plugin
    esac
    dest="$CLAUDE_HOME/$name"
    if [ -e "$dest" ]; then
        mkdir -p "$BACKUP"
        cp -R "$dest" "$BACKUP/"
        rm -rf "$dest"
        backed_up=1
    fi
    cp -R "$src" "$dest"
done
shopt -u dotglob nullglob

# Wire the always-on guards and seed/merge settings.json via setup-local.sh.
# IMPORTANT: setup-local.sh is called unconditionally -- even with --no-setup.
# --no-setup means "no plugin, no deps", not "no guards or settings".
# The guard hooks and settings.json must always be wired.
#
# Interactive prompts for the opt-in layers (skip when --yes or no tty):
if [ "$OPT_ALIASES" -eq 0 ]; then
    if ask "Install the shell launchers (cc/ccd)? Bash and zsh both supported." Y; then
        OPT_ALIASES=1
    fi
fi
if [ "$OPT_ALIASES" -eq 1 ] && [ "$OPT_SYSTEM_PROMPT" -eq 0 ]; then
    if ask "Install the custom system prompt? (recommended)" Y; then
        OPT_SYSTEM_PROMPT=1
    fi
fi

_SETUP_ARGS=""
[ "$SKIP_DEPS"          -eq 1 ] && _SETUP_ARGS="$_SETUP_ARGS --skip-deps"
[ "$OPT_ALIASES"        -eq 1 ] && _SETUP_ARGS="$_SETUP_ARGS --aliases"
[ "$OPT_SYSTEM_PROMPT"  -eq 1 ] && _SETUP_ARGS="$_SETUP_ARGS --system-prompt"
[ "$ASSUME_YES"         -eq 1 ] && _SETUP_ARGS="$_SETUP_ARGS --yes"
# shellcheck disable=SC2086
bash "$CLAUDE_HOME/shell/setup-local.sh" $_SETUP_ARGS

# --- setup -----------------------------------------------------------------

# Plugin: deliver the toolkit (skills, commands, agents, functional hooks) as a
# Claude Code plugin. Requires the claude CLI. The always-on safety guards were
# just installed into ~/.claude/hooks and are wired by the seeded settings.json,
# so they do not depend on the plugin.
if [ "$SKIP_PLUGIN" -eq 0 ]; then
    if command -v claude >/dev/null 2>&1; then
        if ask "Add the playbook marketplace and install the plugin?" Y; then
            log "Adding marketplace and installing the plugin"
            # </dev/null keeps claude off the script's stdin under `curl | bash`.
            claude plugin marketplace add "$MARKETPLACE" </dev/null \
                || warn "marketplace add failed; later: claude plugin marketplace add $MARKETPLACE"
            claude plugin install "$PLUGIN" </dev/null \
                || warn "plugin install failed; later: claude plugin install $PLUGIN"
            claude plugin enable "$PLUGIN" </dev/null >/dev/null 2>&1 || true
            # Retire component dirs left by an older direct install so the plugin
            # is the single source (no duplicate skills/commands/agents).
            for _cd in $PLUGIN_DIRS; do
                if [ -e "$CLAUDE_HOME/$_cd" ]; then
                    mkdir -p "$BACKUP"
                    cp -R "${CLAUDE_HOME:?}/$_cd" "$BACKUP/" && rm -rf "${CLAUDE_HOME:?}/$_cd"
                    backed_up=1
                    log "Moved legacy $_cd/ into the backup (now provided by the plugin)"
                fi
            done
        fi
    else
        warn "claude CLI not found; skipping the plugin (the toolkit ships as a plugin)."
        warn "Install it, then run: claude plugin marketplace add $MARKETPLACE && claude plugin install $PLUGIN"
    fi
fi

# --- summary ---------------------------------------------------------------

# Re-running install is the documented upgrade path and each run past the first
# copies the whole previous tree here, so unpruned this grows without bound:
# nine installs measured 13M. Keeps the newest 5, as setup-local.sh does for
# its own setup-* dirs.
find "$CLAUDE_HOME/backups" -maxdepth 1 -type d -name 'install-*' \
    2>/dev/null | sort -r | tail -n +6 \
    | while IFS= read -r _old; do [ -n "$_old" ] && rm -rf "$_old"; done \
    || true

log "Done."
printf '\n'
printf 'Installed to: %s\n' "$CLAUDE_HOME"
[ "$backed_up" -eq 1 ] && printf 'Replaced files backed up to: %s\n' "$BACKUP"
printf '\nNext steps:\n'
if ! command -v claude >/dev/null 2>&1; then
    printf '  - Install the claude CLI: npm i -g @anthropic-ai/claude-code (or the native installer)\n'
    printf '  - Then: claude plugin marketplace add %s && claude plugin install %s\n' "$MARKETPLACE" "$PLUGIN"
else
    printf '  - The toolkit is a plugin: manage it with `claude plugin list` / `enable` / `disable`.\n'
fi
printf '  - Safety guards (rm, background-await, dash) are always on via settings.json.\n'
if [ "$OPT_ALIASES" -eq 1 ]; then
    printf '  - Shell launchers installed. Open a new terminal or source the rc file to activate cc/ccd.\n'
fi

# Drop into a fresh login shell so the new config is active immediately.
# Only when interactive and the aliases were installed. Reconnect stdin to the
# terminal (</dev/tty) so this works under `curl ... | bash`, where stdin is
# the pipe. exec replaces this process, so nothing runs after it.
if [ "$OPT_ALIASES" -eq 1 ] && [ -t 1 ] && [ -e /dev/tty ]; then
    USER_SHELL="${SHELL:-/bin/zsh}"
    log "Reloading your shell ($USER_SHELL)"
    exec "$USER_SHELL" -l </dev/tty
fi
