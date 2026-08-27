#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# ensure-deps.test.sh: scenarios for shell/ensure-deps.sh.
#
# ensure_dep uses only shell builtins plus the tool and brew, so each scenario
# runs it with a minimal PATH of stubs. ensure_all_deps also needs grep and sed,
# so its scenario keeps the real PATH but uses fake formula names that are never
# on PATH, plus one stubbed "present" tool, to prove only the missing ones are
# installed.
#
# Run:  bash shell/ensure-deps.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENSURE="${SCRIPT_DIR}/ensure-deps.sh"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; PASS=$(( PASS + 1 )); }
fail() { echo "FAIL: $1"; FAIL=$(( FAIL + 1 )); }

WORK="$(mktemp -d)"
# shellcheck disable=SC2064
trap "rm -rf '${WORK}'" EXIT INT TERM

write_stub() {
    local dir="$1" name="$2" body="$3"
    mkdir -p "$dir"
    printf '#!/bin/sh\n%s\n' "$body" > "$dir/$name"
    chmod +x "$dir/$name"
}

# --- 1: dep present -> keep it, do NOT install ---
S1="$WORK/s1"
write_stub "$S1" jq 'echo jq-1.7'
BREW1="$WORK/brew1.log"
write_stub "$S1" brew "echo \"\$@\" >> '$BREW1'"
out1="$(PATH="$S1" /bin/sh -c '. "$1"; ensure_dep jq jq' _ "$ENSURE" 2>&1)"
if printf '%s' "$out1" | grep -q 'jq already installed' && [ ! -s "$BREW1" ]; then
    pass "dep present: kept, not installed"
else
    fail "dep present (out=[$out1], brew=[$(cat "$BREW1" 2>/dev/null)])"
fi

# --- 2: dep absent, brew present -> install the formula ---
S2="$WORK/s2"
BREW2="$WORK/brew2.log"
write_stub "$S2" brew "echo \"\$@\" >> '$BREW2'"
out2="$(PATH="$S2" /bin/sh -c '. "$1"; ensure_dep gh gh' _ "$ENSURE" 2>&1)"
if printf '%s' "$out2" | grep -q 'installing gh via Homebrew' && grep -q 'install gh' "$BREW2" 2>/dev/null; then
    pass "dep absent, brew present: installs via brew"
else
    fail "dep absent+brew (out=[$out2], brew=[$(cat "$BREW2" 2>/dev/null)])"
fi

# --- 3: dep absent, brew absent -> guidance, exit 0 ---
S3="$WORK/s3"; mkdir -p "$S3"
out3="$(PATH="$S3" /bin/sh -c '. "$1"; ensure_dep rtk rtk' _ "$ENSURE" 2>&1)"; rc3=$?
if printf '%s' "$out3" | grep -q 'Homebrew is unavailable' && [ "$rc3" -eq 0 ]; then
    pass "dep absent, no brew: guidance, exit 0"
else
    fail "dep absent+no brew (out=[$out3], rc=$rc3)"
fi

# --- 5: ensure_all_deps installs ONLY the missing formulae ---
# Fake Brewfile with names that are never real commands, plus one made present.
S5="$WORK/s5"
BREW5="$WORK/brew5.log"
write_stub "$S5" brew "echo \"\$@\" >> '$BREW5'"
write_stub "$S5" playbook_faketool_present 'echo present'
BF="$WORK/Brewfile.fake"
cat > "$BF" <<'BREWFILE'
# a comment, not a formula
brew "playbook_faketool_present"  # already installed via the stub
brew "playbook_faketool_absent1"
brew "playbook_faketool_absent2"
BREWFILE
# Real PATH kept (for grep/sed) with the stub dir prepended.
out5="$(PATH="$S5:$PATH" /bin/sh -c '. "$1"; ensure_all_deps "$2"' _ "$ENSURE" "$BF" 2>&1)"
installed="$(cat "$BREW5" 2>/dev/null)"
if printf '%s' "$installed" | grep -q 'install playbook_faketool_absent1' \
   && printf '%s' "$installed" | grep -q 'install playbook_faketool_absent2' \
   && ! printf '%s' "$installed" | grep -q 'install playbook_faketool_present' \
   && printf '%s' "$out5" | grep -q 'playbook_faketool_present already installed'; then
    pass "ensure_all_deps installs only missing formulae"
else
    fail "ensure_all_deps (installed=[$installed], out=[$out5])"
fi

# --- 7: tap lines are processed, and BEFORE the formulae that need them ---
# A formula from a third-party tap cannot install until its tap is added, so the
# tap must be tapped first. The stub logs every brew call in order.
S7="$WORK/s7"
BREW7="$WORK/brew7.log"
# `brew tap` with no args lists taps; here it lists one unrelated tap, so the
# Brewfile's tap counts as absent and must be added.
write_stub "$S7" brew "if [ \"\$1\" = tap ] && [ \$# -eq 1 ]; then echo other/tap; fi; echo \"\$@\" >> '$BREW7'"
BF7="$WORK/Brewfile.tap"
cat > "$BF7" <<'BREWFILE'
tap "playbook_faketap/thing"
brew "playbook_faketool_tapped"
BREWFILE
out7="$(PATH="$S7:$PATH" /bin/sh -c '. "$1"; ensure_all_deps "$2"' _ "$ENSURE" "$BF7" 2>&1)"
log7="$(cat "$BREW7" 2>/dev/null)"
tap_line="$(printf '%s\n' "$log7" | grep -n '^tap playbook_faketap/thing$' | head -1 | cut -d: -f1)"
inst_line="$(printf '%s\n' "$log7" | grep -n '^install playbook_faketool_tapped$' | head -1 | cut -d: -f1)"
if [ -n "$tap_line" ] && [ -n "$inst_line" ] && [ "$tap_line" -lt "$inst_line" ] \
   && printf '%s' "$out7" | grep -q 'adding tap playbook_faketap/thing'; then
    pass "ensure_all_deps taps before installing a tapped formula"
else
    fail "tap ordering (log=[$log7], out=[$out7])"
fi

# --- 8: an already-present tap is not re-tapped ---
S8="$WORK/s8"
BREW8="$WORK/brew8.log"
write_stub "$S8" brew "if [ \"\$1\" = tap ] && [ \$# -eq 1 ]; then echo playbook_faketap/thing; fi; echo \"\$@\" >> '$BREW8'"
out8="$(PATH="$S8:$PATH" /bin/sh -c '. "$1"; ensure_all_deps "$2"' _ "$ENSURE" "$BF7" 2>&1)"
if printf '%s' "$out8" | grep -q 'tap playbook_faketap/thing already present' \
   && ! grep -q '^tap playbook_faketap/thing$' "$BREW8" 2>/dev/null; then
    pass "ensure_all_deps skips a tap that is already present"
else
    fail "tap idempotence (log=[$(cat "$BREW8" 2>/dev/null)], out=[$out8])"
fi

# --- 9: a tapped formula checks PATH for its LAST path segment ---
# `atlassian/acli/acli` provides the command `acli`, so a present `acli` must
# stop the install even though the formula name is fully qualified.
S9="$WORK/s9"
BREW9="$WORK/brew9.log"
write_stub "$S9" brew "echo \"\$@\" >> '$BREW9'"
write_stub "$S9" playbook_faketool_qualified 'echo present'
BF9="$WORK/Brewfile.qualified"
cat > "$BF9" <<'BREWFILE'
brew "playbook_faketap/thing/playbook_faketool_qualified"
BREWFILE
out9="$(PATH="$S9:$PATH" /bin/sh -c '. "$1"; ensure_all_deps "$2"' _ "$ENSURE" "$BF9" 2>&1)"
if printf '%s' "$out9" | grep -q 'playbook_faketool_qualified already installed' \
   && ! grep -q 'install playbook_faketap/thing/playbook_faketool_qualified' "$BREW9" 2>/dev/null; then
    pass "tapped formula resolves its command from the last path segment"
else
    fail "qualified formula command name (log=[$(cat "$BREW9" 2>/dev/null)], out=[$out9])"
fi

# --- 10: a failed install of a tapped formula points at brew trust ---
# Homebrew refuses formulae from an untrusted tap. The script must surface the
# trust command rather than running it, and must report the failure.
S10="$WORK/s10"; mkdir -p "$S10"
write_stub "$S10" brew 'if [ "$1" = install ]; then exit 1; fi; exit 0'
out10="$(PATH="$S10:$PATH" /bin/sh -c '. "$1"; ensure_dep thing playbook_faketap/thing' _ "$ENSURE" 2>&1)"; rc10=$?
if printf '%s' "$out10" | grep -q 'brew trust playbook_faketap' && [ "$rc10" -ne 0 ]; then
    pass "failed tapped install surfaces brew trust and reports failure"
else
    fail "trust hint (out=[$out10], rc=$rc10)"
fi

# --- 6: ensure_all_deps with a missing Brewfile is a clean no-op ---
out6="$(/bin/sh -c '. "$1"; ensure_all_deps "$2"' _ "$ENSURE" "$WORK/does-not-exist" 2>&1)"; rc6=$?
if printf '%s' "$out6" | grep -q 'no Brewfile at' && [ "$rc6" -eq 0 ]; then
    pass "ensure_all_deps missing Brewfile: no-op, exit 0"
else
    fail "ensure_all_deps missing Brewfile (out=[$out6], rc=$rc6)"
fi

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"
[ "$FAIL" -eq 0 ]
