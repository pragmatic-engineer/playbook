#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# Installer for pragmatic-engineer/playbook. Plugin based: the toolkit (skills,
# commands, agents, and the functional hooks) is delivered as a Claude Code
# plugin, while this script installs the always-on safety guards and the other
# local configs (settings.json, statusline, shell integration, dependencies).
# It also fetches, verifies (SHA256, then a --version smoke test), and installs
# the playbook binary into PLAYBOOK_BIN_DIR (default $HOME/.local/bin), and
# puts that directory on PATH. Interactive by default; every optional step
# asks before it runs.
#
#   curl -fsSL https://raw.githubusercontent.com/pragmatic-engineer/playbook/main/install.sh | bash
#
# Source of truth: the latest GitHub release by default, or PLAYBOOK_REF
# (any tag/branch/sha). Falls back to the main branch when no release exists.
# The release binary, unlike the source tree, can only come from a confirmed
# release: PLAYBOOK_REF pins the source tree, not the binary, since a branch
# or a commit has no published release to fetch one from.
# Existing tracked files are backed up before being replaced; runtime state
# (sessions/, projects/, history, plugins/) is never touched.
set -euo pipefail

PLUGIN_REPO="pragmatic-engineer/playbook"
MARKETPLACE="pragmatic-engineer/marketplace"
PLUGIN="playbook@pragmatic-engineer"
CLAUDE_HOME="${CLAUDE_HOME:-$HOME/.claude}"
PLAYBOOK_BIN_DIR="${PLAYBOOK_BIN_DIR:-$HOME/.local/bin}"
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
  PLAYBOOK_BIN_DIR=<dir>         binary install dir (default: $HOME/.local/bin)

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
# shasum ships on macOS, sha256sum on Linux (including a bare debian:stable-slim
# container, which has no shasum); either verifies the release binary.
if command -v shasum >/dev/null 2>&1; then
    CKSUM_CMD=(shasum -a 256 -c)
elif command -v sha256sum >/dev/null 2>&1; then
    CKSUM_CMD=(sha256sum -c)
else
    die "shasum or sha256sum is required to verify the release binary"
fi

# Only a 404 ("this repo published no release") may fall back to main. Any
# other failure means we do not know what the latest release is, and quietly
# installing main would swap a tagged release for in-progress branch work.
# Hence no -f, which would empty the body and collapse 403, 404 and a dead
# network into one indistinguishable empty string. The 403 is routine: the
# unauthenticated API allows 60 requests per hour per IP.
#
# Sets three globals rather than printing the URL to stdout, so the caller can
# also read the resolved tag (needed to name and version the release binary)
# without a second round trip. Called directly, not via command substitution,
# so die()'s exit ends the real script rather than only a subshell.
#   RESOLVED_TAG       the git ref to fetch: a real tag, "main", or $REF
#   RESOLVED_FROM_REF   1 when RESOLVED_TAG did not come from a confirmed
#                       release lookup (a REF pin, or the no-release
#                       fallback), which install_release_binary refuses below
#   TARBALL_URL         source tarball URL for that ref
resolve_tarball_url() {
    if [ -n "$REF" ]; then
        RESOLVED_TAG="$REF"
        RESOLVED_FROM_REF=1
        TARBALL_URL="https://codeload.github.com/$PLUGIN_REPO/tar.gz/$REF"
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

    case "$code" in
        200)
            [ -n "$tag" ] || die "the release API returned 200 with no tag_name; refusing to guess a version"
            RESOLVED_TAG="$tag"
            RESOLVED_FROM_REF=0
            TARBALL_URL="https://codeload.github.com/$PLUGIN_REPO/tar.gz/refs/tags/$tag"
            ;;
        404)
            warn "$PLUGIN_REPO has published no release; installing from the main branch"
            RESOLVED_TAG="main"
            RESOLVED_FROM_REF=1
            TARBALL_URL="https://codeload.github.com/$PLUGIN_REPO/tar.gz/refs/heads/main"
            ;;
        *)
            die "could not read the release API (HTTP $code); retry, or pin a version with PLAYBOOK_REF=vX.Y.Z"
            ;;
    esac
}

# One seam for the download transport, so adding a wget fallback later is a
# one-function change. curl only for now: this repo already hard-requires curl
# (see the preflight check above), and shell/install-resolve.test.sh's stub
# only implements curl.
_fetch() {
    curl -fsSL "$1" -o "$2"
}

# Maps `uname -s`/`uname -m` to the exact asset name a release publishes, into
# the global ASSET. macOS always reports arm64, never aarch64. Under Rosetta,
# `uname -m` still reports x86_64 and the x86_64 binary runs correctly there,
# so no Rosetta special case is needed.
release_asset_name() {
    local version="$1" os arch
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os-$arch" in
        Darwin-arm64)  ASSET="playbook-${version}-aarch64-apple-darwin" ;;
        Darwin-x86_64) ASSET="playbook-${version}-x86_64-apple-darwin" ;;
        Linux-aarch64) ASSET="playbook-${version}-aarch64-unknown-linux-musl" ;;
        Linux-x86_64)  ASSET="playbook-${version}-x86_64-unknown-linux-musl" ;;
        *)
            die "unsupported platform: $os $arch (a Windows binary is published as playbook-${version}-x86_64-pc-windows-msvc.exe, but this is a bash installer and cannot select it)"
            ;;
    esac
}

# Fetches, verifies, and installs the release binary matching $RESOLVED_TAG.
# Refuses when the tag is not a confirmed release (a PLAYBOOK_REF pin, or the
# no-release-published fallback in resolve_tarball_url): a branch or a commit
# has no release and therefore no binary, and mixing a "latest" binary with a
# pinned or unpublished tree is exactly the failure this must not produce
# silently.
install_release_binary() {
    if [ "$RESOLVED_FROM_REF" -eq 1 ]; then
        if [ -n "$REF" ]; then
            die "PLAYBOOK_REF=$REF pins a branch or commit, not a published release; no release binary exists for it. Unset PLAYBOOK_REF to install the latest release together with its matching binary."
        fi
        die "$PLUGIN_REPO has published no release; a verified release binary cannot be fetched."
    fi

    local version
    version="${RESOLVED_TAG#v}"
    release_asset_name "$version"

    STAGE="$(mktemp -d)"

    log "Fetching release binary $ASSET"
    _fetch "https://github.com/$PLUGIN_REPO/releases/download/$RESOLVED_TAG/$ASSET" "$STAGE/$ASSET" \
        || die "could not download $ASSET from the $RESOLVED_TAG release"
    # SHA256SUMS is not signed. Its integrity rests on TLS and on trusting
    # github.com, not on any cryptographic signature; do not imply more
    # assurance than that.
    _fetch "https://github.com/$PLUGIN_REPO/releases/download/$RESOLVED_TAG/SHA256SUMS" "$STAGE/SHA256SUMS" \
        || die "could not download SHA256SUMS from the $RESOLVED_TAG release"

    # SHA256SUMS lists all five published assets; shasum/sha256sum -c on the
    # whole file fails, because the other four are not present locally. Filter
    # to the one line for our asset first. cd into staging so the filename in
    # that line resolves relative to it.
    (cd "$STAGE" && grep -E "^[0-9a-f]{64}  ${ASSET}$" SHA256SUMS > "$ASSET.sha256") \
        || die "no checksum line for $ASSET in SHA256SUMS; the release may be incomplete or corrupt"
    (cd "$STAGE" && "${CKSUM_CMD[@]}" "$ASSET.sha256") >/dev/null 2>&1 \
        || die "checksum mismatch for $ASSET; the download is corrupt"

    chmod 0755 "$STAGE/$ASSET"
    mv "$STAGE/$ASSET" "$STAGE/playbook"

    # The checksum only proves the bytes downloaded intact; it cannot prove
    # they run. A binary that is intact-but-empty, or built for the wrong
    # platform, needs an actual execution to catch.
    local bin_version
    bin_version="$("$STAGE/playbook" --version 2>/dev/null)" \
        || die "the downloaded binary did not run; the $RESOLVED_TAG release may be broken for this platform"
    case "$bin_version" in
        *"$version"*) ;;
        *) die "version mismatch: the downloaded binary reports '$bin_version', expected $version" ;;
    esac

    # ---- first durable write: everything above here leaves no trace on failure.
    mkdir -p "$PLAYBOOK_BIN_DIR"
    local bin_tmp
    bin_tmp="$(mktemp "$PLAYBOOK_BIN_DIR/.playbook.XXXXXX")"
    cp "$STAGE/playbook" "$bin_tmp"
    chmod 0755 "$bin_tmp"
    mv -f "$bin_tmp" "$PLAYBOOK_BIN_DIR/playbook"
    log "Installed playbook $version to $PLAYBOOK_BIN_DIR/playbook"

    rm -rf "$STAGE"
    STAGE=""

    ensure_bin_dir_on_path
}

# Puts $PLAYBOOK_BIN_DIR on PATH for future shells via one idempotent rc-file
# line, using the same grep -qF guard and comment-marker idiom as
# shell/setup-local.sh:263-268, so uninstall.sh can find and strip this exact
# line later.
ensure_bin_dir_on_path() {
    local shell_bin rc_file
    shell_bin="$(basename "${SHELL:-}")"
    case "$shell_bin" in
        zsh)  rc_file="$HOME/.zshrc" ;;
        bash) rc_file="$HOME/.bashrc" ;;
        *)
            rc_file=""
            warn "Shell '$shell_bin' not recognised; add $PLAYBOOK_BIN_DIR to PATH manually."
            ;;
    esac

    if [ -n "$rc_file" ]; then
        if grep -qF "$PLAYBOOK_BIN_DIR" "$rc_file" 2>/dev/null; then
            log "$rc_file already has $PLAYBOOK_BIN_DIR on PATH"
        else
            printf '\n# playbook binary\nexport PATH="%s:$PATH"\n' "$PLAYBOOK_BIN_DIR" >> "$rc_file"
            log "Added $PLAYBOOK_BIN_DIR to PATH in $rc_file"
        fi
    fi

    warn "playbook is installed to $PLAYBOOK_BIN_DIR. Open a new terminal (or source your rc file) so it resolves on PATH."
}

# STAGE (the binary staging dir) and TMP (the source tarball staging dir) are
# cleaned up on any exit, including an interrupted `curl | bash`: the previous
# version of this trap covered EXIT only, so ctrl-c during the network path
# leaked a temp dir.
STAGE=""
TMP=""
_cleanup_staging() {
    [ -n "$STAGE" ] && rm -rf "$STAGE"
    [ -n "$TMP" ] && rm -rf "$TMP"
    return 0
}
trap _cleanup_staging EXIT INT TERM

# PLAYBOOK_SRC is a test seam: when set, install straight from a local
# checkout and skip the network path (resolve/curl/tar, and the release
# binary fetch) entirely.
SRC="${PLAYBOOK_SRC:-}"
if [ -n "$SRC" ]; then
    [ -d "$SRC" ] || die "PLAYBOOK_SRC is not a directory: $SRC"
    log "Installing from local source $SRC"
else
    resolve_tarball_url
    log "Downloading $TARBALL_URL"

    install_release_binary

    TMP="$(mktemp -d)"
    _fetch "$TARBALL_URL" "$TMP/config.tar.gz" || die "download failed: $TARBALL_URL"
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
