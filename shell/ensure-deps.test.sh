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

# write_python_stub <dir> <version> <meets>: a fake python3 that prints <version>
# for the "print version" call and exits <meets> (0 = satisfies the floor) for
# the "sys.exit" version-check call.
write_python_stub() {
    local dir="$1" ver="$2" meets="$3"
    mkdir -p "$dir"
    cat > "$dir/python3" <<STUB
#!/bin/sh
case "\$*" in
  *sys.exit*) exit $meets ;;
  *) echo "$ver" ;;
esac
STUB
    chmod +x "$dir/python3"
}

# --- 4a: python3 present and new enough -> keep it, do NOT install ---
S4A="$WORK/s4a"
write_python_stub "$S4A" "3.13.0" 0
BREW4A="$WORK/brew4a.log"
write_stub "$S4A" brew "echo \"\$@\" >> '$BREW4A'"
out4a="$(PATH="$S4A" /bin/sh -c '. "$1"; ensure_python python@3.13' _ "$ENSURE" 2>&1)"
if printf '%s' "$out4a" | grep -q 'python3 3.13.0 already installed' && [ ! -s "$BREW4A" ]; then
    pass "python present and >= 3.9: kept, not installed"
else
    fail "python present new (out=[$out4a], brew=[$(cat "$BREW4A" 2>/dev/null)])"
fi

# --- 4b: python3 present but too old -> install the formula ---
S4B="$WORK/s4b"
write_python_stub "$S4B" "3.8.0" 1
BREW4B="$WORK/brew4b.log"
write_stub "$S4B" brew "echo \"\$@\" >> '$BREW4B'"
out4b="$(PATH="$S4B" /bin/sh -c '. "$1"; ensure_python python@3.13' _ "$ENSURE" 2>&1)"
if printf '%s' "$out4b" | grep -q 'python3 3.8.0 is older than 3.9' && grep -q 'install python@3.13' "$BREW4B" 2>/dev/null; then
    pass "python present but < 3.9: installs the pinned formula"
else
    fail "python present old (out=[$out4b], brew=[$(cat "$BREW4B" 2>/dev/null)])"
fi

# --- 4c: python3 absent -> install the formula ---
S4C="$WORK/s4c"
BREW4C="$WORK/brew4c.log"
write_stub "$S4C" brew "echo \"\$@\" >> '$BREW4C'"
out4c="$(PATH="$S4C" /bin/sh -c '. "$1"; ensure_python python@3.13' _ "$ENSURE" 2>&1)"
if printf '%s' "$out4c" | grep -q 'python3 not found; installing' && grep -q 'install python@3.13' "$BREW4C" 2>/dev/null; then
    pass "python absent: installs the pinned formula"
else
    fail "python absent (out=[$out4c], brew=[$(cat "$BREW4C" 2>/dev/null)])"
fi

# --- 5: ensure_all_deps installs ONLY the missing formulae ---
# Fake Brewfile with names that are never real commands, plus one made present.
S5="$WORK/s5"
BREW5="$WORK/brew5.log"
write_stub "$S5" brew "echo \"\$@\" >> '$BREW5'"
write_stub "$S5" playbook_faketool_present 'echo present'
# A controlled python3 (new enough) so the python@X route is deterministic
# regardless of the real python3 in the environment.
write_python_stub "$S5" "3.13.0" 0
BF="$WORK/Brewfile.fake"
cat > "$BF" <<'BREWFILE'
# a comment, not a formula
brew "playbook_faketool_present"  # already installed via the stub
brew "python@3.13"                # routes to the version-aware check
brew "playbook_faketool_absent1"
brew "playbook_faketool_absent2"
BREWFILE
# Real PATH kept (for grep/sed) with the stub dir prepended.
out5="$(PATH="$S5:$PATH" /bin/sh -c '. "$1"; ensure_all_deps "$2"' _ "$ENSURE" "$BF" 2>&1)"
installed="$(cat "$BREW5" 2>/dev/null)"
if printf '%s' "$installed" | grep -q 'install playbook_faketool_absent1' \
   && printf '%s' "$installed" | grep -q 'install playbook_faketool_absent2' \
   && ! printf '%s' "$installed" | grep -q 'install playbook_faketool_present' \
   && ! printf '%s' "$installed" | grep -q 'install python@3.13' \
   && printf '%s' "$out5" | grep -q 'playbook_faketool_present already installed' \
   && printf '%s' "$out5" | grep -q 'python3 3.13.0 already installed'; then
    pass "ensure_all_deps installs only missing formulae; python routes to the version check"
else
    fail "ensure_all_deps (installed=[$installed], out=[$out5])"
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
