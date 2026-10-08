#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# install-binary-path.test.sh: install.sh places the binary and one PATH
# marker, hands the right flags to `playbook init`, and copes with a release
# binary older than the script. Removal is `playbook uninstall`, covered by
# tests/uninstall.rs and tests/uninstall_rc.rs.
#
# Sourced from `git archive HEAD` so untracked build output cannot report
# leaks no user could hit.
#
# Run:  bash shell/install-binary-path.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1${2:+ -- $2}"; (( FAIL++ )) || true; }

if ! git -C "$REPO_ROOT" rev-parse --git-dir >/dev/null 2>&1; then
    echo "SKIP: not a git checkout, cannot build a tracked-files-only source"
    exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

SRC="$WORK/src"
HOME_DIR="$WORK/home"
CLAUDE_DIR="$HOME_DIR/.claude"
mkdir -p "$SRC" "$CLAUDE_DIR"
git -C "$REPO_ROOT" archive HEAD | tar -x -C "$SRC"

# --- Binary install lifecycle -----------------------------------------------
# install_release_binary and ensure_bin_dir_on_path only run on install.sh's
# network path, never under PLAYBOOK_SRC, so the later scenarios never touch
# the binary or its PATH line. These scenarios stub curl and uname (same
# technique as shell/install-resolve.test.sh) and drive that path directly.
# The stub serves no source tarball, so install.sh always dies right after
# installing the binary and wiring PATH; that is expected here, since the
# binary and PATH work are already done by the time it dies.

BIN_STUB="$WORK/binstub"
mkdir -p "$BIN_STUB"

# Same dual call-shape stub as shell/install-resolve.test.sh: install.sh calls
# curl both as `-o FILE -w '%{http_code}'` (resolve_tarball_url) and as
# `-fsSL URL -o FILE` (_fetch). Serves the releases API, the release asset,
# and SHA256SUMS; anything else, including the source tarball, fails on
# purpose, which is what drives the "dies right after the binary" behaviour
# above.
cat > "$BIN_STUB/curl" <<'STUB'
#!/usr/bin/env bash
out=""; url=""; fail_on_http_error=0
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    -w) shift 2 ;;
    -*) case "$1" in *f*) fail_on_http_error=1 ;; esac; shift ;;
    *)  url="$1"; shift ;;
  esac
done
case "$url" in
  *api.github.com/repos/*/releases/latest)
    code="${STUB_CODE:-200}"
    if [ "$fail_on_http_error" = "1" ] && [ "$code" -ge 400 ]; then exit 22; fi
    if [ -n "$out" ]; then
      printf '%s' "${STUB_BODY:-}" > "$out"
      printf '%s' "$code"
    else
      printf '%s' "${STUB_BODY:-}"
    fi
    exit 0 ;;
  */releases/download/*/SHA256SUMS)
    if [ -n "$out" ]; then printf '%s' "${STUB_SUMS_BODY:-}" > "$out"
    else printf '%s' "${STUB_SUMS_BODY:-}"; fi
    exit 0 ;;
  */releases/download/*)
    if [ -n "$out" ]; then printf '%s' "${STUB_ASSET_BODY:-}" > "$out"
    else printf '%s' "${STUB_ASSET_BODY:-}"; fi
    exit 0 ;;
  *) exit 22 ;;
esac
STUB
chmod +x "$BIN_STUB/curl"

# No attestation support, so a developer's real gh never verifies the fake binary.
printf '#!/usr/bin/env bash\nexit 127\n' > "$BIN_STUB/gh"
chmod +x "$BIN_STUB/gh"

# Stub uname so the asset name install.sh computes is deterministic across
# hosts (real macOS/Linux dev boxes and CI alike), matching the fixed ASSET
# below instead of whatever the machine running the suite actually is.
cat > "$BIN_STUB/uname" <<'STUB'
#!/usr/bin/env bash
case "$1" in
  -s) printf '%s\n' "${STUB_UNAME_S:-Linux}" ;;
  -m) printf '%s\n' "${STUB_UNAME_M:-x86_64}" ;;
  *) command -p uname "$@" ;;
esac
STUB
chmod +x "$BIN_STUB/uname"

_sha256() {
    if command -v shasum >/dev/null 2>&1; then
        printf '%s' "$1" | shasum -a 256 | awk '{print $1}'
    else
        printf '%s' "$1" | sha256sum | awk '{print $1}'
    fi
}

RELEASE_BODY='{"tag_name": "v1.2.3"}'
ASSET="playbook-1.2.3-x86_64-unknown-linux-musl"
GOOD_ASSET_BODY=$'#!/usr/bin/env bash\necho "playbook 1.2.3"\n'
SUMS_BODY="$(_sha256 "$GOOD_ASSET_BODY")  $ASSET"

run_binary_install() {
    local home="$1" bindir="$2"
    PATH="$BIN_STUB:$PATH" CLAUDE_HOME="$home/.claude" HOME="$home" \
        PLAYBOOK_BIN_DIR="$bindir" SHELL=/bin/bash \
        STUB_CODE=200 STUB_BODY="$RELEASE_BODY" \
        STUB_ASSET_BODY="$GOOD_ASSET_BODY" STUB_SUMS_BODY="$SUMS_BODY" \
        bash "$REPO_ROOT/install.sh" --no-setup --skip-plugin >/dev/null 2>&1
}
marker_count() {
    local n
    n="$(grep -cF '# playbook binary' "$1" 2>/dev/null || true)"
    printf '%s' "${n:-0}"
}

# 1. After install the binary is executable and the rc file carries exactly
# one marker.
bin_home="$(mktemp -d "$WORK/bin-home.XXXXXX")"
bin_dir="$WORK/bin-dir"
run_binary_install "$bin_home" "$bin_dir"

if [ -x "$bin_dir/playbook" ] && [ "$(marker_count "$bin_home/.bashrc")" -eq 1 ]; then
    pass "install places the binary and exactly one PATH marker"
else
    fail "install places the binary and exactly one PATH marker" \
        "binary executable: $([ -x "$bin_dir/playbook" ] && echo yes || echo no), markers: $(marker_count "$bin_home/.bashrc")"
fi

# 2. Idempotence: installing twice must not double the marker.
idem_home="$(mktemp -d "$WORK/bin-idem.XXXXXX")"
idem_dir="$WORK/bin-idem-dir"
run_binary_install "$idem_home" "$idem_dir"
run_binary_install "$idem_home" "$idem_dir"

if [ "$(marker_count "$idem_home/.bashrc")" -eq 1 ]; then
    pass "installing twice leaves exactly one PATH marker"
else
    fail "installing twice leaves exactly one PATH marker" \
        "markers: $(marker_count "$idem_home/.bashrc")"
fi

# --- install.sh must not assume a binary feature newer than the last release
# -----------------------------------------------------------------------------
# install.sh is fetched from the main branch by the documented curl
# one-liner, while the binary comes from the latest RELEASE, so the two are
# permanently allowed to drift. This stub behaves like a real release binary
# that predates --system-prompt: it accepts --version and a bare `init`, but
# rejects any flag it does not recognise, exactly the way clap does, exit
# code and wording included. --yes auto-accepts both the aliases and
# system-prompt prompts, which is what feeds --system-prompt into
# install.sh's $_INIT_ARGS in the first place; without install.sh's own
# `_init_supports` probe checking `init --help` first, that reaches this
# stub and install.sh dies with settings.json never written, exactly what
# happened against the real v0.10.0 release on 2026-08-20.
STRICT_STUB_DIR="$WORK/strict-init-bin"
mkdir -p "$STRICT_STUB_DIR"
cat > "$STRICT_STUB_DIR/playbook" <<'STUB'
#!/usr/bin/env bash
case "${1:-}" in
  --version) printf 'playbook 0.10.0\n'; exit 0 ;;
  init)
    shift
    if [ "${1:-}" = "--help" ]; then
      printf 'Install or repair the local Claude Code configuration\n\nUsage: playbook init\n'
      exit 0
    fi
    if [ $# -gt 0 ]; then
      printf "error: unexpected argument '%s' found\n\nUsage: playbook init\n" "$1" >&2
      exit 2
    fi
    home="${CLAUDE_HOME:-$HOME/.claude}"
    mkdir -p "$home"
    [ -f "$home/settings.json" ] || printf '{}\n' > "$home/settings.json"
    [ -f "$home/.settings.base.json" ] || printf '{}\n' > "$home/.settings.base.json"
    printf 'settings: wired - seeded from template\n'
    printf 'guards: ok - already in place\n'
    printf 'hooks: ok - all hooks already wired\n'
    printf 'shim: skipped - $SHELL is neither bash nor zsh\n'
    printf 'statusline: ok - already up to date\n'
    printf 'system-prompt: skipped - not installed; pass --system-prompt to opt in\n'
    exit 0
    ;;
esac
exit 0
STUB
chmod +x "$STRICT_STUB_DIR/playbook"

strict_home="$(mktemp -d "$WORK/strict-home.XXXXXX")"
strict_out="$(PLAYBOOK_SRC="$SRC" CLAUDE_HOME="$strict_home/.claude" HOME="$strict_home" \
    PLAYBOOK_BIN_DIR="$STRICT_STUB_DIR" SHELL=/bin/bash \
    bash "$REPO_ROOT/install.sh" --yes --skip-plugin 2>&1)"
strict_rc=$?

if [ "$strict_rc" -eq 0 ] && [ -f "$strict_home/.claude/settings.json" ]; then
    pass "install succeeds against a release binary that predates an optional init flag"
else
    fail "install succeeds against a release binary that predates an optional init flag" \
        "exit $strict_rc, settings.json present: $([ -f "$strict_home/.claude/settings.json" ] && echo yes || echo no); output: $strict_out"
fi

# --- install.sh must forward --aliases to `playbook init`, same as
# --system-prompt already does -------------------------------------------
# Regression test: OPT_ALIASES was tracked from the --aliases flag and from
# the interactive prompt, but never appended to $_INIT_ARGS, so `playbook
# init` never received --aliases and its own shim step silently ran
# unconfigured (always "skipped"), no matter what the user asked for. Only
# --system-prompt was ever forwarded. This stub records the exact argv
# `init` receives so the fix is pinned by an assertion, not just a comment.
RECORDING_STUB_DIR="$WORK/recording-init-bin"
mkdir -p "$RECORDING_STUB_DIR"
cat > "$RECORDING_STUB_DIR/playbook" <<STUB
#!/usr/bin/env bash
case "\${1:-}" in
  --version) printf 'playbook 0.0.0-stub\n' ;;
  init)
    shift
    printf '%s\n' "\$@" > "$WORK/init-argv.txt"
    if [ "\${1:-}" = "--help" ]; then
      printf -- '--aliases\n--system-prompt\n'
      exit 0
    fi
    home="\${CLAUDE_HOME:-\$HOME/.claude}"
    [ -f "\$home/settings.json" ] || printf '{}\n' > "\$home/settings.json"
    [ -f "\$home/.settings.base.json" ] || printf '{}\n' > "\$home/.settings.base.json"
    printf 'settings: wired - seeded from template\n'
    printf 'guards: ok - already in place\n'
    printf 'hooks: ok - all hooks already wired\n'
    printf 'shim: ok - installed\n'
    printf 'statusline: ok - already up to date\n'
    printf 'system-prompt: ok - installed\n'
    ;;
esac
exit 0
STUB
chmod +x "$RECORDING_STUB_DIR/playbook"

aliases_home="$(mktemp -d "$WORK/aliases-home.XXXXXX")"
PLAYBOOK_SRC="$SRC" CLAUDE_HOME="$aliases_home/.claude" HOME="$aliases_home" \
    PLAYBOOK_BIN_DIR="$RECORDING_STUB_DIR" SHELL=/bin/bash \
    bash "$REPO_ROOT/install.sh" --yes --aliases --system-prompt --skip-plugin >/dev/null 2>&1
aliases_argv="$(cat "$WORK/init-argv.txt" 2>/dev/null || true)"

if printf '%s' "$aliases_argv" | grep -q -- '--aliases'; then
    pass "install.sh --aliases forwards --aliases to playbook init"
else
    fail "install.sh --aliases forwards --aliases to playbook init" \
        "playbook init received: '$aliases_argv'"
fi

if printf '%s' "$aliases_argv" | grep -q -- '--system-prompt'; then
    pass "install.sh --system-prompt forwards --system-prompt to playbook init"
else
    fail "install.sh --system-prompt forwards --system-prompt to playbook init" \
        "playbook init received: '$aliases_argv'"
fi

# Note: there is no non-interactive way to get OPT_ALIASES=0 to test the
# absence case here. ask()'s own short-circuit (install.sh:56) takes the
# prompt's default (Y for aliases) whenever --yes is set OR /dev/tty does not
# exist, which is every automated test run; there is no --no-aliases flag to
# force a decline. --no-setup does NOT suppress this (install.sh:97 documents
# it as skipping only the plugin: "guards, settings, and shell wiring still
# run"), so it is not a substitute. The unsupported-binary path (aliases
# requested but the installed release predates the flag) is already covered
# by the "predates an optional init flag" scenario above, which runs with
# --yes against a stub that rejects --aliases as well as --system-prompt.

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
