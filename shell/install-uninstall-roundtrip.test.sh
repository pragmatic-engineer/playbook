#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# install-uninstall-roundtrip.test.sh: install.sh and uninstall.sh must agree
# on what the toolkit owns.
#
# They cannot agree by construction, since install derives its copy set at
# runtime while uninstall hardcodes one; six entries had already drifted. Any
# re-parse of either list would rot with it, so this runs the real pair and
# inspects the leftovers, sourced from `git archive HEAD` so untracked build
# output cannot report leaks no user could hit.
#
# Run:  bash shell/install-uninstall-roundtrip.test.sh
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

# Stub `playbook` binary. install.sh's PLAYBOOK_SRC seam skips the network
# path entirely, including install_release_binary, which is what would
# normally place a binary at PLAYBOOK_BIN_DIR; yet install.sh still hands off
# unconditionally to "$PLAYBOOK_BIN_DIR/playbook init". Without a binary
# there, that call is a bare command-not-found (exit 127) that kills the
# script before settings.json and .settings.base.json -- both written only by
# the real `init`, never by install.sh's own tree copy -- ever exist, which is
# what the preserved-settings assertion below needs. Answers just enough for
# install.sh to proceed: writes those two files, exits 0, and prints a step
# report shaped like the real `playbook init`'s, so the output stays a
# plausible stand-in. Named distinctly from BIN_STUB below, which stubs curl
# and uname for the unrelated binary-lifecycle scenarios.
PLAYBOOK_STUB_DIR="$WORK/init-bin"
mkdir -p "$PLAYBOOK_STUB_DIR"
cat > "$PLAYBOOK_STUB_DIR/playbook" <<'STUB'
#!/usr/bin/env bash
case "${1:-}" in
  --version) printf 'playbook 0.0.0-stub\n' ;;
  init)
    home="${CLAUDE_HOME:-$HOME/.claude}"
    [ -f "$home/settings.json" ] || printf '{}\n' > "$home/settings.json"
    [ -f "$home/.settings.base.json" ] || printf '{}\n' > "$home/.settings.base.json"
    printf 'settings: wired - seeded from template\n'
    printf 'guards: ok - already in place\n'
    printf 'hooks: ok - all hooks already wired\n'
    printf 'shim: skipped - $SHELL is neither bash nor zsh\n'
    printf 'statusline: ok - already up to date\n'
    printf 'system-prompt: skipped - not installed; pass --system-prompt to opt in\n'
    ;;
esac
exit 0
STUB
chmod +x "$PLAYBOOK_STUB_DIR/playbook"

# --force bypasses the git-repo guard. The scratch HOME can sit inside a git
# working tree depending on where mktemp points, and that guard exists to stop
# a raw rm from stranding index entries, which is irrelevant here.
run_install() {
    PLAYBOOK_SRC="$SRC" CLAUDE_HOME="$CLAUDE_DIR" HOME="$HOME_DIR" PLAYBOOK_BIN_DIR="$PLAYBOOK_STUB_DIR" \
        bash "$REPO_ROOT/install.sh" --no-setup --skip-plugin >/dev/null 2>&1
}
run_uninstall() {
    CLAUDE_HOME="$CLAUDE_DIR" HOME="$HOME_DIR" \
        bash "$REPO_ROOT/uninstall.sh" --yes --force >/dev/null 2>&1
}

entries() { ls -A "$CLAUDE_DIR" 2>/dev/null | sort; }

# Documented as preserved by uninstall.sh's own help text. Anything else left
# behind is a leak.
PRESERVED="$(printf '%s\n' .settings.base.json backups settings.json | sort)"

run_install
install_rc=$?
after_install="$(entries)"

# Guard against a vacuous pass. If the install did nothing, every "was it
# removed" assertion below would be trivially satisfied and the suite would go
# green while testing nothing.
#
# install.sh now places exactly three entries itself (install.sh, uninstall.sh,
# hooks/lib/config-hash.sh); everything else a real install adds (settings.json,
# the shell launcher runtime, the system prompt, the statusline) is
# `playbook init`'s job, and the stub above only reproduces its two
# settings-file writes. A generic entry-count threshold would have to be
# re-guessed every time that split shifts, so this asserts install.sh's own
# three entries by name instead -- an exact regression pin, not a magic number.
if [ "$install_rc" -ne 0 ]; then
    fail "install.sh exits 0" "exit $install_rc"
elif [ ! -f "$CLAUDE_DIR/install.sh" ] || [ ! -f "$CLAUDE_DIR/uninstall.sh" ] \
    || [ ! -f "$CLAUDE_DIR/hooks/lib/config-hash.sh" ]; then
    fail "install.sh populates CLAUDE_HOME" \
        "missing one of install.sh, uninstall.sh, hooks/lib/config-hash.sh; got: $(printf '%s' "$after_install" | tr '\n' ' ')"
else
    pass "install.sh populates CLAUDE_HOME"
fi

run_uninstall
uninstall_rc=$?
after_uninstall="$(entries)"

if [ "$uninstall_rc" -eq 0 ]; then
    pass "uninstall.sh exits 0"
else
    fail "uninstall.sh exits 0" "exit $uninstall_rc"
fi

# The assertion that matters: nothing the installer wrote may outlive the
# uninstaller, except the entries it documents as preserved.
leaked="$(comm -23 <(printf '%s\n' "$after_uninstall" | grep -v '^$') <(printf '%s\n' "$PRESERVED"))"
if [ -z "$leaked" ]; then
    pass "uninstall removes everything install added"
else
    fail "uninstall removes everything install added" \
        "stranded: $(printf '%s' "$leaked" | tr '\n' ' ')"
fi

# And the preserved entries really are preserved, so the fix above cannot be
# "delete more aggressively until the first assertion passes".
missing=""
for keep in settings.json .settings.base.json; do
    [ -e "$CLAUDE_DIR/$keep" ] || missing="$missing $keep"
done
if [ -z "$missing" ]; then
    pass "uninstall preserves user-owned settings"
else
    fail "uninstall preserves user-owned settings" "removed:$missing"
fi

# --- Binary install/uninstall lifecycle -------------------------------------
# install_release_binary and ensure_bin_dir_on_path only run on install.sh's
# network path, never under PLAYBOOK_SRC, so run_install() above never touches
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
run_binary_uninstall() {
    local home="$1" bindir="$2"
    CLAUDE_HOME="$home/.claude" HOME="$home" PLAYBOOK_BIN_DIR="$bindir" \
        bash "$REPO_ROOT/uninstall.sh" --yes --force >/dev/null 2>&1
}
marker_count() {
    local n
    n="$(grep -cF '# playbook binary' "$1" 2>/dev/null || true)"
    printf '%s' "${n:-0}"
}

# 1 & 2. One lifecycle: install, then uninstall that same install. After
# install the binary is executable and the rc file carries exactly one
# marker; after uninstall both are gone.
bin_home="$(mktemp -d "$WORK/bin-home.XXXXXX")"
bin_dir="$WORK/bin-dir"
run_binary_install "$bin_home" "$bin_dir"

if [ -x "$bin_dir/playbook" ] && [ "$(marker_count "$bin_home/.bashrc")" -eq 1 ]; then
    pass "install places the binary and exactly one PATH marker"
else
    fail "install places the binary and exactly one PATH marker" \
        "binary executable: $([ -x "$bin_dir/playbook" ] && echo yes || echo no), markers: $(marker_count "$bin_home/.bashrc")"
fi

run_binary_uninstall "$bin_home" "$bin_dir"

if [ ! -e "$bin_dir/playbook" ] && [ "$(marker_count "$bin_home/.bashrc")" -eq 0 ]; then
    pass "uninstall removes the binary and its PATH marker"
else
    fail "uninstall removes the binary and its PATH marker" \
        "binary present: $([ -e "$bin_dir/playbook" ] && echo yes || echo no), markers: $(marker_count "$bin_home/.bashrc")"
fi

# 3. Idempotence: installing twice must not double the marker.
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

# 4. Surgical: uninstall must strip only the marker block it owns, never a
# user's own PATH edit. strip_rc_binary_path anchors on the "# playbook
# binary" marker specifically so it cannot eat an unrelated export line.
surgical_home="$(mktemp -d "$WORK/bin-surgical.XXXXXX")"
surgical_dir="$WORK/bin-surgical-dir"
mkdir -p "$surgical_home"
USER_LINE='export PATH="/opt/mytools:$PATH"'
printf '%s\n' "$USER_LINE" > "$surgical_home/.bashrc"

run_binary_install "$surgical_home" "$surgical_dir"
run_binary_uninstall "$surgical_home" "$surgical_dir"

if grep -qF "$USER_LINE" "$surgical_home/.bashrc" 2>/dev/null \
    && [ "$(marker_count "$surgical_home/.bashrc")" -eq 0 ]; then
    pass "uninstall preserves a user's own PATH edit"
else
    fail "uninstall preserves a user's own PATH edit" \
        "user line present: $(grep -qF "$USER_LINE" "$surgical_home/.bashrc" 2>/dev/null && echo yes || echo no), markers: $(marker_count "$surgical_home/.bashrc")"
fi

# 5. Uninstall with no binary installed is a clean no-op, not an error.
noop_home="$(mktemp -d "$WORK/bin-noop.XXXXXX")"
noop_dir="$WORK/bin-noop-dir"
run_binary_uninstall "$noop_home" "$noop_dir"
noop_rc=$?

if [ "$noop_rc" -eq 0 ] && [ ! -e "$noop_dir/playbook" ]; then
    pass "uninstall with no binary present is a clean no-op"
else
    fail "uninstall with no binary present is a clean no-op" "exit $noop_rc"
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

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
