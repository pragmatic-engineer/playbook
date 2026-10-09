#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# plugin-bin-shim.test.sh: bin/playbook, the shim the marketplace plugin puts
# on the Bash tool's PATH, runs a real binary when one exists, bootstraps it
# through the shipped install.sh when none does, and never loops.
#
# Run:  bash shell/plugin-bin-shim.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1${2:+ -- $2}"; (( FAIL++ )) || true; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

# A plugin tree: bin/playbook (the real shim), a stub install.sh, plugin.json.
PLUGIN="$WORK/plugin"
mkdir -p "$PLUGIN/bin" "$PLUGIN/.claude-plugin"
cp "$REPO_ROOT/bin/playbook" "$PLUGIN/bin/playbook"
chmod +x "$PLUGIN/bin/playbook"
printf '{\n  "name": "playbook",\n  "version": "9.9.9"\n}\n' > "$PLUGIN/.claude-plugin/plugin.json"

# The stub installer records how it was called. With INSTALL_WRITES=1 it
# installs a fake binary into PLAYBOOK_BIN_DIR, like the real one does.
cat > "$PLUGIN/install.sh" <<'STUB'
#!/usr/bin/env bash
echo "called" >> "$STUB_LOG"
echo "ref=${PLAYBOOK_REF:-} args=$*" >> "$STUB_LOG"
echo "guard=${PLAYBOOK_SHIM_BOOTSTRAP:-}" >> "$STUB_LOG"
if [ "${INSTALL_WRITES:-0}" = "1" ]; then
    mkdir -p "$PLAYBOOK_BIN_DIR"
    printf '#!/usr/bin/env bash\necho "real playbook $*"\n' > "$PLAYBOOK_BIN_DIR/playbook"
    chmod +x "$PLAYBOOK_BIN_DIR/playbook"
fi
exit "${INSTALL_EXIT:-0}"
STUB

make_real() {
    mkdir -p "$1"
    printf '#!/usr/bin/env bash\necho "real playbook $*"\n' > "$1/playbook"
    chmod +x "$1/playbook"
}

run_shim() {
    # $1 = home dir; remaining args go to the shim. PATH has the shim dir and
    # system dirs only, unless the caller adds more through EXTRA_PATH.
    local home="$1"; shift
    HOME="$home" PLAYBOOK_BIN_DIR="$home/.local/bin" STUB_LOG="$WORK/stub.log" \
        PATH="$PLUGIN/bin:${EXTRA_PATH:+$EXTRA_PATH:}/usr/bin:/bin" \
        bash "$PLUGIN/bin/playbook" "$@" 2>"$WORK/stderr.txt"
}

# The shim also looks in fixed Homebrew locations. A dev machine that has the
# real binary there would hide the "no binary" scenarios, so skip them then.
HAVE_SYSTEM_BINARY=0
for c in /opt/homebrew/bin/playbook /usr/local/bin/playbook /home/linuxbrew/.linuxbrew/bin/playbook; do
    [ -x "$c" ] && HAVE_SYSTEM_BINARY=1
done

# 1. A binary in PLAYBOOK_BIN_DIR runs, with the arguments, and nothing installs.
h1="$WORK/h1"; make_real "$h1/.local/bin"; : > "$WORK/stub.log"
out="$(run_shim "$h1" mode status)"
if [ "$out" = "real playbook mode status" ] && [ ! -s "$WORK/stub.log" ]; then
    pass "runs the real binary with its arguments and installs nothing"
else
    fail "runs the real binary with its arguments and installs nothing" "out: $out, log: $(cat "$WORK/stub.log")"
fi

# 2. The shim's own directory on PATH is never taken for the real binary.
h2="$WORK/h2"; mkdir -p "$h2"; make_real "$WORK/elsewhere"; : > "$WORK/stub.log"
out="$(EXTRA_PATH="$WORK/elsewhere" run_shim "$h2" --version)"
if [ "$out" = "real playbook --version" ]; then
    pass "skips its own directory and finds the real binary further down PATH"
else
    fail "skips its own directory and finds the real binary further down PATH" "out: $out"
fi

if [ "$HAVE_SYSTEM_BINARY" -eq 0 ]; then
    # 3. No binary: the stub installer runs once, pinned to the plugin version,
    # then the installed binary runs.
    h3="$WORK/h3"; mkdir -p "$h3"; : > "$WORK/stub.log"
    out="$(INSTALL_WRITES=1 run_shim "$h3" init)"
    log="$(cat "$WORK/stub.log")"
    if [ "$out" = "real playbook init" ] \
        && [ "$(grep -c '^called$' "$WORK/stub.log")" -eq 1 ] \
        && printf '%s' "$log" | grep -q 'ref=v9.9.9 args=--yes --binary-only' \
        && printf '%s' "$log" | grep -q 'guard=1'; then
        pass "bootstraps through install.sh pinned to the plugin version, then runs the binary"
    else
        fail "bootstraps through install.sh pinned to the plugin version, then runs the binary" "out: $out, log: $log"
    fi

    # 4. The installer fails: exit 127, one attempt, a hint on stderr.
    h4="$WORK/h4"; mkdir -p "$h4"; : > "$WORK/stub.log"
    INSTALL_EXIT=1 run_shim "$h4" init >/dev/null; rc=$?
    if [ "$rc" -eq 127 ] && [ "$(grep -c '^called$' "$WORK/stub.log")" -eq 1 ] \
        && grep -q 'install.sh' "$WORK/stderr.txt"; then
        pass "a failed install exits 127 after one attempt and prints the manual command"
    else
        fail "a failed install exits 127 after one attempt and prints the manual command" "rc=$rc, calls: $(grep -c '^called$' "$WORK/stub.log")"
    fi

    # 5. The installer succeeds but installs nothing: still one attempt, no loop.
    h5="$WORK/h5"; mkdir -p "$h5"; : > "$WORK/stub.log"
    run_shim "$h5" init >/dev/null; rc=$?
    if [ "$rc" -eq 127 ] && [ "$(grep -c '^called$' "$WORK/stub.log")" -eq 1 ]; then
        pass "an installer that places nothing does not loop"
    else
        fail "an installer that places nothing does not loop" "rc=$rc, calls: $(grep -c '^called$' "$WORK/stub.log")"
    fi

    # 6. The bootstrap guard set by an outer call stops a second install.
    h6="$WORK/h6"; mkdir -p "$h6"; : > "$WORK/stub.log"
    PLAYBOOK_SHIM_BOOTSTRAP=1 run_shim "$h6" init >/dev/null; rc=$?
    if [ "$rc" -eq 127 ] && [ ! -s "$WORK/stub.log" ]; then
        pass "the guard variable blocks a nested install"
    else
        fail "the guard variable blocks a nested install" "rc=$rc, log: $(cat "$WORK/stub.log")"
    fi
else
    echo "SKIP: a playbook binary exists in a fixed system location, bootstrap scenarios skipped"
fi

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
