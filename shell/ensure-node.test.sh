#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# ensure-node.test.sh: scenarios for shell/ensure-node.sh's ensure_node.
#
# ensure_node uses only shell builtins plus node and brew, so each scenario runs
# it in a subshell with a minimal PATH containing just the stubs it should see.
# That lets us simulate "node present" and "node absent" without touching the
# real PATH.
#
# Run:  bash shell/ensure-node.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE="${SCRIPT_DIR}/ensure-node.sh"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; PASS=$(( PASS + 1 )); }
fail() { echo "FAIL: $1"; FAIL=$(( FAIL + 1 )); }

WORK="$(mktemp -d)"
# shellcheck disable=SC2064
trap "rm -rf '${WORK}'" EXIT INT TERM

# write_stub <dir> <name> <body>: create an executable stub on a stub PATH.
write_stub() {
    local dir="$1" name="$2" body="$3"
    mkdir -p "$dir"
    printf '#!/bin/sh\n%s\n' "$body" > "$dir/$name"
    chmod +x "$dir/$name"
}

# run_ensure <path> : source ensure-node.sh and run ensure_node with PATH set to
# exactly <path>, capturing stdout+stderr.
run_ensure() {
    local stub_path="$1"
    PATH="$stub_path" /bin/sh -c '. "$1"; ensure_node' _ "$ENSURE" 2>&1
}

# --- 1: node present -> use it, do NOT install ---
S1="$WORK/s1"
write_stub "$S1" node 'echo v20.11.0'
BREW_REC1="$WORK/brew1.log"
write_stub "$S1" brew "echo brew \"\$@\" >> '$BREW_REC1'"
out1="$(run_ensure "$S1")"
if printf '%s' "$out1" | grep -q 'using the Node already installed' \
   && printf '%s' "$out1" | grep -q 'v20.11.0' \
   && [ ! -s "$BREW_REC1" ]; then
    pass "node present: uses it, brew install not called"
else
    fail "node present (out=[$out1], brew-log=[$(cat "$BREW_REC1" 2>/dev/null)])"
fi

# --- 2: node absent, brew present -> install via brew ---
S2="$WORK/s2"
BREW_REC2="$WORK/brew2.log"
write_stub "$S2" brew "echo \"\$@\" >> '$BREW_REC2'"
out2="$(run_ensure "$S2")"
if printf '%s' "$out2" | grep -q 'installing via Homebrew' \
   && grep -q 'install node' "$BREW_REC2" 2>/dev/null; then
    pass "node absent, brew present: installs node via brew"
else
    fail "node absent+brew (out=[$out2], brew-log=[$(cat "$BREW_REC2" 2>/dev/null)])"
fi

# --- 3: node absent, brew absent -> guidance, no crash, exit 0 ---
S3="$WORK/s3"
mkdir -p "$S3"  # empty stub dir: no node, no brew
out3="$(run_ensure "$S3")"; rc3=$?
if printf '%s' "$out3" | grep -q 'Homebrew is unavailable' && [ "$rc3" -eq 0 ]; then
    pass "node absent, brew absent: prints guidance, exits 0"
else
    fail "node absent+no brew (out=[$out3], rc=$rc3)"
fi

# --- 4: a brew-provided node counts as present (idempotent re-run) ---
S4="$WORK/s4"
write_stub "$S4" node 'echo v22.0.0'
BREW_REC4="$WORK/brew4.log"
write_stub "$S4" brew "echo \"\$@\" >> '$BREW_REC4'"
out4="$(run_ensure "$S4")"
if printf '%s' "$out4" | grep -q 'v22.0.0' && [ ! -s "$BREW_REC4" ]; then
    pass "re-run with node present is a no-op (no reinstall)"
else
    fail "idempotent re-run (out=[$out4], brew-log=[$(cat "$BREW_REC4" 2>/dev/null)])"
fi

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"
[ "$FAIL" -eq 0 ]
