#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# shellcheck shell=bash
# launcher.test.sh: cross-shell tests for shell/shared/ modules.
#
# Every scenario runs under both bash (/bin/bash) and zsh.
# Run: bash shell/shared/launcher.test.sh
#      zsh  shell/shared/launcher.test.sh
#
# Exit: non-zero if any scenario fails.

set -eo pipefail

PASS_BASH=0; FAIL_BASH=0; PASS_ZSH=0; FAIL_ZSH=0; TOTAL=0

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SHARED_DIR="$SCRIPT_DIR"

TESTHOME=$(mktemp -d)
SCRATCH=$(mktemp -d)
cleanup() { rm -rf "$TESTHOME" "$SCRATCH"; }
trap cleanup EXIT

# ─── fake HOME setup ─────────────────────────────────────────────────────────

mkdir -p "$TESTHOME/.claude/hooks/lib"
mkdir -p "$TESTHOME/.config/playbook/cc-state"
mkdir -p "$TESTHOME/.config/playbook/runtime"
mkdir -p "$TESTHOME/.claude/projects"
printf '{}' > "$TESTHOME/.claude/settings.json"

# Fake config_hash: returns ${_FAKE_HASH} (default fakehash001)
cat > "$TESTHOME/.claude/hooks/lib/config-hash.sh" << 'CFGHASH'
# shellcheck shell=sh
config_hash() { printf '%s\n' "${_FAKE_HASH:-fakehash001}"; }
CFGHASH

# ─── test scenario helpers ───────────────────────────────────────────────────

# Write a temp test script with a common prelude (sources all shared modules).
# Usage: mk_test <outfile> <extra_body>
mk_test() {
    local outfile="$1" body="$2"
    cat > "$outfile" << PREAMBLE
HOME='$TESTHOME'
export HOME
source '$SHARED_DIR/sessions.sh'
source '$SHARED_DIR/bust-cache.sh'
source '$SHARED_DIR/retention.sh'
source '$SHARED_DIR/config-drift.sh'
source '$SHARED_DIR/clean-resume.sh'
source '$SHARED_DIR/worktree.sh'
source '$SHARED_DIR/dispatch.sh'
PREAMBLE
    printf '%s\n' "$body" >> "$outfile"
}

# Run a test script under one shell; update counters.
run_one() {
    local shell_bin="$1" label="$2" script="$3"
    TOTAL=$(( TOTAL + 1 ))
    if "$shell_bin" "$script" >/dev/null 2>&1; then
        printf 'PASS(%s)  %s\n' "$shell_bin" "$label"
        case "$shell_bin" in
            *bash) PASS_BASH=$(( PASS_BASH + 1 )) ;;
            *zsh)  PASS_ZSH=$(( PASS_ZSH + 1 )) ;;
        esac
    else
        printf 'FAIL(%s)  %s\n' "$shell_bin" "$label" >&2
        case "$shell_bin" in
            *bash) FAIL_BASH=$(( FAIL_BASH + 1 )) ;;
            *zsh)  FAIL_ZSH=$(( FAIL_ZSH + 1 )) ;;
        esac
    fi
}

# Run a test script under both shells.
scenario() {
    local label="$1" script="$2"
    run_one /bin/bash "$label" "$script"
    run_one zsh       "$label" "$script"
}

# ─── project/slug helpers for retention scenarios ────────────────────────────

# Compute the Claude project slug for a given absolute path.
# Replace every non-alphanumeric char with '-'. Matches _cc_prune logic.
compute_slug() {
    local path="$1"
    printf '%s' "${path//[^a-zA-Z0-9]/-}"
}

# Known working dir for retention tests: use a path inside TESTHOME.
TESTPWD="$TESTHOME/testproj"
mkdir -p "$TESTPWD"
PROJ_SLUG="$(compute_slug "$TESTPWD")"
PROJ_DIR="$TESTHOME/.claude/projects/$PROJ_SLUG"
mkdir -p "$PROJ_DIR"

# UUID helpers: two UUIDs used across scenarios.
UUID_A="aabbccdd-1111-2222-3333-444455556666"
UUID_B="bbccddee-1111-2222-3333-444455556666"
UUID_C="ccddee00-1111-2222-3333-444455556666"
UUID_D="ddeeff11-1111-2222-3333-444455556666"

# Session lookup project dir (separate from retention dir).
SESSION_PROJ="$TESTHOME/.claude/projects/sessiontest"
mkdir -p "$SESSION_PROJ"

# ─── scenario 1: session lookup finds transcript by customTitle ───────────────

printf '{"type":"summary","customTitle":"myname"}\n' \
    > "$SESSION_PROJ/$UUID_A.jsonl"

T1="$SCRATCH/s1.sh"
mk_test "$T1" "
proj_dir='$SESSION_PROJ'
result=\$(_cc_find_session_by_title \"\$proj_dir\" 'myname')
[ \"\$result\" = '$UUID_A' ] || exit 1
"
scenario "session lookup finds transcript by customTitle" "$T1"

# ─── scenario 2: session lookup miss returns nothing, exit 0 ─────────────────

EMPTY_PROJ="$TESTHOME/.claude/projects/emptytest"
mkdir -p "$EMPTY_PROJ"

T2="$SCRATCH/s2.sh"
mk_test "$T2" "
result=\$(_cc_find_session_by_title '$EMPTY_PROJ' 'noname')
[ -z \"\$result\" ] || exit 1
"
scenario "session lookup miss returns nothing, exit 0" "$T2"

# ─── scenario 3: UUID check accepts valid uuid, rejects 'memory' ─────────────

T3="$SCRATCH/s3.sh"
mk_test "$T3" "
_cc_is_uuid '$UUID_A' || exit 1
_cc_is_uuid 'memory' && exit 1
_cc_is_uuid ''       && exit 1
exit 0
"
scenario "UUID check accepts valid uuid, rejects non-uuid" "$T3"

# ─── scenario 4: enumeration lists newest first ───────────────────────────────

ENUM_PROJ="$TESTHOME/.claude/projects/enumtest"
mkdir -p "$ENUM_PROJ"

# Create two jsonl files; set mtimes so UUID_B is newer than UUID_A.
printf '{"type":"summary","customTitle":"oldsession"}\n' \
    > "$ENUM_PROJ/$UUID_A.jsonl"
printf '{"type":"summary","customTitle":"newsession"}\n' \
    > "$ENUM_PROJ/$UUID_B.jsonl"

touch -t 202501010000 "$ENUM_PROJ/$UUID_A.jsonl"
touch -t 202601010000 "$ENUM_PROJ/$UUID_B.jsonl"

T4="$SCRATCH/s4.sh"
mk_test "$T4" "
first=\$(_cc_enumerate_sessions '$ENUM_PROJ' | head -1 | cut -f2)
[ \"\$first\" = '$UUID_B' ] || exit 1
"
scenario "enumeration lists newest first" "$T4"

# ─── scenario 5: retention floor keeps at least 2 ────────────────────────────

# Set up the retention project dir with 3 jsonl files and matching runtime dirs.
KEEP1_DIR="$TESTHOME/.claude/projects/$(compute_slug "$TESTHOME/keep1test")"
mkdir -p "$TESTHOME/keep1test"
mkdir -p "$KEEP1_DIR"

for u in "$UUID_A" "$UUID_B" "$UUID_C"; do
    printf '{"type":"summary"}\n' > "$KEEP1_DIR/$u.jsonl"
done
touch -t 202401010000 "$KEEP1_DIR/$UUID_A.jsonl"
touch -t 202501010000 "$KEEP1_DIR/$UUID_B.jsonl"
touch -t 202601010000 "$KEEP1_DIR/$UUID_C.jsonl"

T5a="$SCRATCH/s5a.sh"
mk_test "$T5a" "
cd '$TESTHOME/keep1test'
CCD_KEEP=1 _cc_prune
# Floor of 2: all three files exist before, after prune at least 2 must remain.
count=\$(find '$KEEP1_DIR' -maxdepth 1 -name '*.jsonl' -type f 2>/dev/null | wc -l | tr -d ' ')
[ \"\$count\" -ge 2 ] || exit 1
"
scenario "retention floor: CCD_KEEP=1 keeps at least 2" "$T5a"

T5b="$SCRATCH/s5b.sh"
mk_test "$T5b" "
cd '$TESTHOME/keep1test'
CCD_KEEP=0 _cc_prune
count=\$(find '$KEEP1_DIR' -maxdepth 1 -name '*.jsonl' -type f 2>/dev/null | wc -l | tr -d ' ')
[ \"\$count\" -ge 2 ] || exit 1
"
scenario "retention floor: CCD_KEEP=0 deletes nothing" "$T5b"

# ─── scenario 6: retention deletes oldest beyond keep ────────────────────────

PRUNE_DIR="$TESTHOME/.claude/projects/$(compute_slug "$TESTHOME/prunetest")"
mkdir -p "$TESTHOME/prunetest"
mkdir -p "$PRUNE_DIR"

for u in "$UUID_A" "$UUID_B" "$UUID_C" "$UUID_D"; do
    printf '{"type":"summary"}\n' > "$PRUNE_DIR/$u.jsonl"
done
# Set mtimes: A=oldest, D=newest.
touch -t 202201010000 "$PRUNE_DIR/$UUID_A.jsonl"
touch -t 202301010000 "$PRUNE_DIR/$UUID_B.jsonl"
touch -t 202401010000 "$PRUNE_DIR/$UUID_C.jsonl"
touch -t 202501010000 "$PRUNE_DIR/$UUID_D.jsonl"

T6="$SCRATCH/s6.sh"
mk_test "$T6" "
cd '$TESTHOME/prunetest'
CCD_KEEP=2 _cc_prune
# Oldest two (UUID_A, UUID_B) must be gone; newest two (UUID_C, UUID_D) remain.
[ ! -f '$PRUNE_DIR/$UUID_A.jsonl' ] || exit 1
[ ! -f '$PRUNE_DIR/$UUID_B.jsonl' ] || exit 1
[ -f '$PRUNE_DIR/$UUID_C.jsonl' ] || exit 1
[ -f '$PRUNE_DIR/$UUID_D.jsonl' ] || exit 1
"
scenario "retention deletes oldest beyond keep" "$T6"

# ─── scenario 7: config-drift stamp and detection ────────────────────────────

DRIFT_TESTPWD="$TESTHOME/drifttest"
mkdir -p "$DRIFT_TESTPWD"

T7="$SCRATCH/s7.sh"
mk_test "$T7" "
cd '$DRIFT_TESTPWD'
_FAKE_HASH=hash001
export _FAKE_HASH

# Redefine config_hash using the env var so in-process changes take effect.
config_hash() { printf '%s\n' \"\${_FAKE_HASH:-fakehash001}\"; }

_cc_config_stamp

# Right after stamp: no drift expected.
result=\$(_cc_config_drifted)
[ -z \"\$result\" ] || exit 1

# Change the hash: drift expected.
_FAKE_HASH=hash999
result=\$(_cc_config_drifted)
[ -n \"\$result\" ] || exit 1
"
scenario "config-drift: stamp then detect hash change" "$T7"

# ─── scenario 8: smoke test all shared modules source without error ───────────

T8="$SCRATCH/s8.sh"
cat > "$T8" << SMOKE
HOME='$TESTHOME'
export HOME
source '$SHARED_DIR/sessions.sh'   || exit 1
source '$SHARED_DIR/bust-cache.sh' || exit 1
source '$SHARED_DIR/retention.sh'  || exit 1
source '$SHARED_DIR/config-drift.sh' || exit 1
source '$SHARED_DIR/clean-resume.sh' || exit 1
source '$SHARED_DIR/worktree.sh'   || exit 1
source '$SHARED_DIR/dispatch.sh'   || exit 1
type _cc_find_session_by_title >/dev/null 2>&1 || exit 1
type _cc_is_uuid               >/dev/null 2>&1 || exit 1
type _cc_enumerate_sessions    >/dev/null 2>&1 || exit 1
type _cc_list_sessions         >/dev/null 2>&1 || exit 1
type _cc_bust_cache            >/dev/null 2>&1 || exit 1
type _cc_prune                 >/dev/null 2>&1 || exit 1
type _cc_config_stamp          >/dev/null 2>&1 || exit 1
type _cc_config_drifted        >/dev/null 2>&1 || exit 1
type _cc_clean_resume          >/dev/null 2>&1 || exit 1
type _cc_worktree              >/dev/null 2>&1 || exit 1
type _claude                   >/dev/null 2>&1 || exit 1
type _cc_opt_takes_value       >/dev/null 2>&1 || exit 1
SMOKE
scenario "smoke: all shared modules source without error and define their functions" "$T8"

# ─── final summary ───────────────────────────────────────────────────────────

TOTAL_PASS=$(( PASS_BASH + PASS_ZSH ))
TOTAL_FAIL=$(( FAIL_BASH + FAIL_ZSH ))

printf '\n'
printf 'bash: %d passed, %d failed\n' "$PASS_BASH" "$FAIL_BASH"
printf 'zsh:  %d passed, %d failed\n' "$PASS_ZSH"  "$FAIL_ZSH"
printf '%d/%d scenarios passed\n' "$TOTAL_PASS" "$TOTAL"

[ "$TOTAL_FAIL" -eq 0 ] || exit 1
