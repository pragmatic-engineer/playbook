#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# merge-settings.test.sh: hermetic tests for shell/merge-settings.sh.
# Covers the 3-way merge policy, base refresh (C2 freeze), validation guards,
# and the skip-file (N3). All scenarios use isolated temp dirs; no network, no
# live writes, explicit fixture paths only.
#
# Run:  bash shell/merge-settings.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MERGE="${SCRIPT_DIR}/merge-settings.py"
PASS=0
FAIL=0

if ! command -v jq >/dev/null 2>&1; then
    echo "SKIP: jq not available; merge tests need jq"
    exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1${2:+ -> $2}"; (( FAIL++ )) || true; }

run_merge() {
    # run_merge BASE TEMPLATE USER -> stdout; sets rc=exit code
    local base="$1" tmpl="$2" usr="$3" newbase skip
    newbase="$(mktemp "$WORK/newbase.XXXXXX")"
    skip="$(mktemp "$WORK/skip.XXXXXX")"
    _RUN_NEWBASE="$newbase"
    _RUN_SKIP="$skip"
    _RUN_OUT="$(python3 "$MERGE" "$base" "$tmpl" "$usr" "$newbase" "$skip" 2>/tmp/merge-test-stderr)"
    rc=$?
    _RUN_ERR="$(cat /tmp/merge-test-stderr)"
}

# ---------------------------------------------------------------------------
# 1. User-unchanged key -> template value (product update applied)
# ---------------------------------------------------------------------------
s1() {
    local d; d="$(mktemp -d "$WORK/s1.XXXXXX")"
    printf '{"k":"v1","shared":"base"}' > "$d/base.json"
    printf '{"k":"v2","shared":"tmpl"}' > "$d/tmpl.json"
    printf '{"k":"v1","shared":"base"}' > "$d/user.json"   # user unchanged
    local out rc
    run_merge "$d/base.json" "$d/tmpl.json" "$d/user.json"
    out="$_RUN_OUT"
    [ "$rc" -eq 0 ] || { fail "s1" "exit $rc"; return 1; }
    # k: user unchanged from base -> template value v2
    jq -e '.k == "v2"' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s1" "k not updated to template v2; got $(printf '%s' "$out" | jq -c .)"; return 1; }
    # shared: same logic
    jq -e '.shared == "tmpl"' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s1" "shared not updated"; return 1; }
    pass "s1: user-unchanged key gets template value"
}

# ---------------------------------------------------------------------------
# 2. User-changed key -> user value kept
# ---------------------------------------------------------------------------
s2() {
    local d; d="$(mktemp -d "$WORK/s2.XXXXXX")"
    printf '{"k":"original"}' > "$d/base.json"
    printf '{"k":"updated_by_tmpl"}' > "$d/tmpl.json"
    printf '{"k":"my_custom"}' > "$d/user.json"
    local out rc
    run_merge "$d/base.json" "$d/tmpl.json" "$d/user.json"
    out="$_RUN_OUT"
    [ "$rc" -eq 0 ] || { fail "s2" "exit $rc"; return 1; }
    jq -e '.k == "my_custom"' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s2" "user value not preserved"; return 1; }
    pass "s2: user-changed key is preserved"
}

# ---------------------------------------------------------------------------
# 3. New template key -> added to output
# ---------------------------------------------------------------------------
s3() {
    local d; d="$(mktemp -d "$WORK/s3.XXXXXX")"
    printf '{"existing":"x"}' > "$d/base.json"
    printf '{"existing":"x","newkey":"from_tmpl"}' > "$d/tmpl.json"
    printf '{"existing":"x"}' > "$d/user.json"
    local out rc
    run_merge "$d/base.json" "$d/tmpl.json" "$d/user.json"
    out="$_RUN_OUT"
    [ "$rc" -eq 0 ] || { fail "s3" "exit $rc"; return 1; }
    jq -e 'has("newkey") and .newkey == "from_tmpl"' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s3" "new template key missing from output"; return 1; }
    pass "s3: new template key added to output"
}

# ---------------------------------------------------------------------------
# 4. Template dropped an unchanged key -> absent from output
# ---------------------------------------------------------------------------
s4() {
    local d; d="$(mktemp -d "$WORK/s4.XXXXXX")"
    printf '{"gone":"was_here","keep":"yes"}' > "$d/base.json"
    printf '{"keep":"yes"}' > "$d/tmpl.json"          # dropped "gone"
    printf '{"gone":"was_here","keep":"yes"}' > "$d/user.json"  # user unchanged
    local out rc
    run_merge "$d/base.json" "$d/tmpl.json" "$d/user.json"
    out="$_RUN_OUT"
    [ "$rc" -eq 0 ] || { fail "s4" "exit $rc"; return 1; }
    jq -e 'has("gone") | not' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s4" "dropped key still present"; return 1; }
    jq -e '.keep == "yes"' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s4" "kept key lost"; return 1; }
    pass "s4: template-dropped unchanged key absent from output"
}

# ---------------------------------------------------------------------------
# 5. Absent base -> additive fallback (user keys kept + new template keys added)
# ---------------------------------------------------------------------------
s5() {
    local d; d="$(mktemp -d "$WORK/s5.XXXXXX")"
    printf '{"added":"from_tmpl","shared":"tmpl_val"}' > "$d/tmpl.json"
    printf '{"mykey":"myval","shared":"user_val"}' > "$d/user.json"
    local out rc
    run_merge "/no/such/base.json" "$d/tmpl.json" "$d/user.json"
    out="$_RUN_OUT"
    [ "$rc" -eq 0 ] || { fail "s5" "exit $rc"; return 1; }
    # user keys all kept
    jq -e '.mykey == "myval"' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s5" "user key lost"; return 1; }
    # new template key added
    jq -e '.added == "from_tmpl"' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s5" "new template key missing"; return 1; }
    # shared: user[k] != base[k](absent/null) -> contested -> user kept
    jq -e '.shared == "user_val"' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s5" "shared not user-val"; return 1; }
    pass "s5: absent base gives additive fallback"
}

# ---------------------------------------------------------------------------
# 6. Conflict (user changed AND template changed differently) -> user kept,
#    exactly one element in skip array for that key
# ---------------------------------------------------------------------------
s6() {
    local d; d="$(mktemp -d "$WORK/s6.XXXXXX")"
    printf '{"k":"base_val"}' > "$d/base.json"
    printf '{"k":"tmpl_val"}' > "$d/tmpl.json"
    printf '{"k":"user_custom"}' > "$d/user.json"
    local out rc
    run_merge "$d/base.json" "$d/tmpl.json" "$d/user.json"
    out="$_RUN_OUT"
    [ "$rc" -eq 0 ] || { fail "s6" "exit $rc"; return 1; }
    jq -e '.k == "user_custom"' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s6" "user value not kept"; return 1; }
    local skip; skip="$_RUN_SKIP"
    jq -e 'type == "array" and length == 1 and .[0].key == "k" and .[0].template_had == "tmpl_val" and .[0].yours == "user_custom"' "$skip" >/dev/null 2>&1 \
        || { fail "s6" "skip array wrong: $(cat "$skip")"; return 1; }
    pass "s6: conflict -> user kept, one skip entry"
}

# ---------------------------------------------------------------------------
# 7. Corrupt TEMPLATE -> exit != 0, empty stdout
# ---------------------------------------------------------------------------
s7() {
    local d; d="$(mktemp -d "$WORK/s7.XXXXXX")"
    printf '{ not valid json ' > "$d/tmpl.json"
    printf '{"k":"v"}' > "$d/user.json"
    local out rc
    out="$(python3 "$MERGE" "/no/base" "$d/tmpl.json" "$d/user.json" "$d/nb" 2>/dev/null)"; rc=$?
    [ "$rc" -ne 0 ] || { fail "s7" "expected non-zero exit, got 0"; return 1; }
    [ -z "$out" ]   || { fail "s7" "expected empty stdout, got: $out"; return 1; }
    pass "s7: corrupt TEMPLATE -> non-zero exit, empty stdout"
}

# ---------------------------------------------------------------------------
# 8. Missing TEMPLATE -> exit != 0, empty stdout
# ---------------------------------------------------------------------------
s8() {
    local d; d="$(mktemp -d "$WORK/s8.XXXXXX")"
    printf '{"k":"v"}' > "$d/user.json"
    local out rc
    out="$(python3 "$MERGE" "/no/base" "$d/no_tmpl.json" "$d/user.json" "$d/nb" 2>/dev/null)"; rc=$?
    [ "$rc" -ne 0 ] || { fail "s8" "expected non-zero exit"; return 1; }
    [ -z "$out" ]   || { fail "s8" "expected empty stdout"; return 1; }
    pass "s8: missing TEMPLATE -> non-zero exit, empty stdout"
}

# ---------------------------------------------------------------------------
# 9. Corrupt USER -> exit != 0, empty stdout
# ---------------------------------------------------------------------------
s9() {
    local d; d="$(mktemp -d "$WORK/s9.XXXXXX")"
    printf '{"k":"v"}' > "$d/tmpl.json"
    printf '{ not valid json ' > "$d/user.json"
    local out rc
    out="$(python3 "$MERGE" "/no/base" "$d/tmpl.json" "$d/user.json" "$d/nb" 2>/dev/null)"; rc=$?
    [ "$rc" -ne 0 ] || { fail "s9" "expected non-zero exit"; return 1; }
    [ -z "$out" ]   || { fail "s9" "expected empty stdout"; return 1; }
    pass "s9: corrupt USER -> non-zero exit, empty stdout"
}

# ---------------------------------------------------------------------------
# 10. USER is a top-level array -> exit != 0 (N2)
# ---------------------------------------------------------------------------
s10() {
    local d; d="$(mktemp -d "$WORK/s10.XXXXXX")"
    printf '{"k":"v"}' > "$d/tmpl.json"
    printf '[1,2,3]' > "$d/user.json"
    local out rc
    out="$(python3 "$MERGE" "/no/base" "$d/tmpl.json" "$d/user.json" "$d/nb" 2>/dev/null)"; rc=$?
    [ "$rc" -ne 0 ] || { fail "s10" "expected non-zero exit for array USER"; return 1; }
    [ -z "$out" ]   || { fail "s10" "expected empty stdout"; return 1; }
    pass "s10: USER is top-level array -> non-zero exit (N2)"
}

# ---------------------------------------------------------------------------
# 11. USER is a scalar -> exit != 0 (N2)
# ---------------------------------------------------------------------------
s11() {
    local d; d="$(mktemp -d "$WORK/s11.XXXXXX")"
    printf '{"k":"v"}' > "$d/tmpl.json"
    printf '"just a string"' > "$d/user.json"
    local out rc
    out="$(python3 "$MERGE" "/no/base" "$d/tmpl.json" "$d/user.json" "$d/nb" 2>/dev/null)"; rc=$?
    [ "$rc" -ne 0 ] || { fail "s11" "expected non-zero exit for scalar USER"; return 1; }
    [ -z "$out" ]   || { fail "s11" "expected empty stdout"; return 1; }
    pass "s11: USER is scalar -> non-zero exit (N2)"
}

# ---------------------------------------------------------------------------
# 12. USER == {} -> output becomes the template
# ---------------------------------------------------------------------------
s12() {
    local d; d="$(mktemp -d "$WORK/s12.XXXXXX")"
    printf '{}' > "$d/base.json"
    printf '{"a":"1","b":"2"}' > "$d/tmpl.json"
    printf '{}' > "$d/user.json"
    local out rc
    run_merge "$d/base.json" "$d/tmpl.json" "$d/user.json"
    out="$_RUN_OUT"
    [ "$rc" -eq 0 ] || { fail "s12" "exit $rc"; return 1; }
    jq -e '.a == "1" and .b == "2"' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s12" "output not template: $out"; return 1; }
    pass "s12: USER == {} -> output equals template"
}

# ---------------------------------------------------------------------------
# 13. Corrupt BASE -> additive fallback + stderr warning (exit 0)
# ---------------------------------------------------------------------------
s13() {
    local d; d="$(mktemp -d "$WORK/s13.XXXXXX")"
    printf 'not json' > "$d/base.json"
    printf '{"newkey":"nv","shared":"tv"}' > "$d/tmpl.json"
    printf '{"mykey":"mv","shared":"uv"}' > "$d/user.json"
    local out stderr_out rc newbase skip
    newbase="$(mktemp "$d/nb.XXXXXX")"
    skip="$(mktemp "$d/skip.XXXXXX")"
    out="$(python3 "$MERGE" "$d/base.json" "$d/tmpl.json" "$d/user.json" "$newbase" "$skip" 2>"$d/err.txt")"; rc=$?
    stderr_out="$(cat "$d/err.txt")"
    [ "$rc" -eq 0 ] || { fail "s13" "expected exit 0 on bad base, got $rc"; return 1; }
    # should warn on stderr
    printf '%s' "$stderr_out" | grep -qi "warning" || { fail "s13" "no warning on stderr: $stderr_out"; return 1; }
    # additive: user keys kept
    jq -e '.mykey == "mv"' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s13" "user key lost"; return 1; }
    jq -e '.newkey == "nv"' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s13" "new tmpl key missing"; return 1; }
    pass "s13: corrupt BASE -> additive fallback + warning"
}

# ---------------------------------------------------------------------------
# 14. Type-mismatch on contested key -> user's whole value kept, non-corrupting
# ---------------------------------------------------------------------------
s14() {
    local d; d="$(mktemp -d "$WORK/s14.XXXXXX")"
    printf '{"k":"scalar_base"}' > "$d/base.json"
    printf '{"k":"scalar_tmpl"}' > "$d/tmpl.json"
    printf '{"k":{"nested":"obj"}}' > "$d/user.json"   # user changed scalar -> object
    local out rc
    run_merge "$d/base.json" "$d/tmpl.json" "$d/user.json"
    out="$_RUN_OUT"
    [ "$rc" -eq 0 ] || { fail "s14" "exit $rc"; return 1; }
    jq -e '.k | type == "object" and .nested == "obj"' <(printf '%s' "$out") >/dev/null 2>&1 \
        || { fail "s14" "user object value not kept: $out"; return 1; }
    pass "s14: type-mismatch on contested key -> user value kept"
}

# ---------------------------------------------------------------------------
# 15. Base refresh freeze: contested key's NEWBASE value == OLD base value
# ---------------------------------------------------------------------------
s15() {
    local d; d="$(mktemp -d "$WORK/s15.XXXXXX")"
    printf '{"k":"old_base"}' > "$d/base.json"
    printf '{"k":"tmpl_update"}' > "$d/tmpl.json"
    printf '{"k":"user_custom"}' > "$d/user.json"
    local out rc newbase
    newbase="$(mktemp "$d/nb.XXXXXX")"
    out="$(python3 "$MERGE" "$d/base.json" "$d/tmpl.json" "$d/user.json" "$newbase" 2>/dev/null)"; rc=$?
    [ "$rc" -eq 0 ] || { fail "s15" "exit $rc"; return 1; }
    # merged: user value
    jq -e '.k == "user_custom"' <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s15" "user value not in merged"; return 1; }
    # NEWBASE: OLD base value, NOT template value
    jq -e '.k == "old_base"' "$newbase" >/dev/null 2>&1 \
        || { fail "s15" "NEWBASE not frozen to old base; got $(jq -c . "$newbase")"; return 1; }
    pass "s15: contested key frozen to OLD base value in NEWBASE"
}

# ---------------------------------------------------------------------------
# 16. C2 coincidence sequence: 3 cycles, key stays user-controlled through
#     the cycle where template coincidentally equals user's value
# ---------------------------------------------------------------------------
s16() {
    local d; d="$(mktemp -d "$WORK/s16.XXXXXX")"

    # Initial state (seeded from template v1)
    printf '{"k":"original"}' > "$d/base0.json"   # .settings.base.json after seed
    printf '{"k":"original"}' > "$d/user.json"
    # User customizes k
    printf '{"k":"my_custom"}' > "$d/user.json"

    # --- Cycle 1: template ships v2 (different from original) ---
    printf '{"k":"tmpl_v2"}' > "$d/tmpl1.json"
    local nb1; nb1="$(mktemp "$d/nb1.XXXXXX")"
    local merged1
    merged1="$(python3 "$MERGE" "$d/base0.json" "$d/tmpl1.json" "$d/user.json" "$nb1" 2>/dev/null)"
    # merged: user kept
    jq -e '.k == "my_custom"' <(printf '%s' "$merged1") >/dev/null 2>&1 \
        || { fail "s16" "cycle1 user value lost: $merged1"; return 1; }
    # newbase1: frozen to OLD base ("original"), NOT tmpl_v2
    jq -e '.k == "original"' "$nb1" >/dev/null 2>&1 \
        || { fail "s16" "cycle1 newbase not frozen: $(jq -c . "$nb1")"; return 1; }

    # --- Cycle 2: template ships k == user's value (coincidence) ---
    printf '{"k":"my_custom"}' > "$d/tmpl2.json"   # template now matches user!
    local nb2; nb2="$(mktemp "$d/nb2.XXXXXX")"
    local merged2
    merged2="$(python3 "$MERGE" "$nb1" "$d/tmpl2.json" "$d/user.json" "$nb2" 2>/dev/null)"
    # C2 fix: base is frozen "original", user is "my_custom" -> still contested -> user kept
    jq -e '.k == "my_custom"' <(printf '%s' "$merged2") >/dev/null 2>&1 \
        || { fail "s16" "cycle2 C2 fail: k should be user value, got: $merged2"; return 1; }
    # newbase2: still frozen to "original"
    jq -e '.k == "original"' "$nb2" >/dev/null 2>&1 \
        || { fail "s16" "cycle2 newbase not still frozen: $(jq -c . "$nb2")"; return 1; }

    pass "s16: C2 coincidence -> user value preserved through coincidental template match"
}

# ---------------------------------------------------------------------------
# 17. Skip file parses as valid JSON array
# ---------------------------------------------------------------------------
s17() {
    local d; d="$(mktemp -d "$WORK/s17.XXXXXX")"
    printf '{"k":"base_val","u":"bu"}' > "$d/base.json"
    printf '{"k":"tmpl_val","u":"tu"}' > "$d/tmpl.json"
    printf '{"k":"user_val","u":"bu"}' > "$d/user.json"   # k contested, u unchanged
    local skip rc
    skip="$(mktemp "$d/skip.XXXXXX")"
    python3 "$MERGE" "$d/base.json" "$d/tmpl.json" "$d/user.json" "$d/nb" "$skip" >/dev/null 2>/dev/null; rc=$?
    [ "$rc" -eq 0 ] || { fail "s17" "exit $rc"; return 1; }
    jq 'type == "array"' "$skip" >/dev/null 2>&1 || { fail "s17" "skip file not an array: $(cat "$skip")"; return 1; }
    jq empty "$skip" >/dev/null 2>&1 || { fail "s17" "skip file invalid JSON"; return 1; }
    pass "s17: skip file is valid JSON array"
}

# ---------------------------------------------------------------------------
# 18. jq empty on merged stdout and NEWBASE_OUT
# ---------------------------------------------------------------------------
s18() {
    local d; d="$(mktemp -d "$WORK/s18.XXXXXX")"
    printf '{"a":"1"}' > "$d/base.json"
    printf '{"a":"2","b":"3"}' > "$d/tmpl.json"
    printf '{"a":"custom","b":"3"}' > "$d/user.json"
    local out rc nb
    nb="$(mktemp "$d/nb.XXXXXX")"
    out="$(python3 "$MERGE" "$d/base.json" "$d/tmpl.json" "$d/user.json" "$nb" 2>/dev/null)"; rc=$?
    [ "$rc" -eq 0 ] || { fail "s18" "exit $rc"; return 1; }
    jq empty <(printf '%s' "$out") >/dev/null 2>&1 || { fail "s18" "merged stdout not valid JSON"; return 1; }
    jq empty "$nb" >/dev/null 2>&1 || { fail "s18" "NEWBASE not valid JSON"; return 1; }
    pass "s18: merged stdout and NEWBASE are valid JSON"
}

# ---------------------------------------------------------------------------
# 19. Zero withheld keys -> skip file is []
# ---------------------------------------------------------------------------
s19() {
    local d; d="$(mktemp -d "$WORK/s19.XXXXXX")"
    printf '{"k":"v"}' > "$d/base.json"
    printf '{"k":"v2"}' > "$d/tmpl.json"
    printf '{"k":"v"}' > "$d/user.json"   # user unchanged -> template update, no conflict
    local skip rc
    skip="$(mktemp "$d/skip.XXXXXX")"
    python3 "$MERGE" "$d/base.json" "$d/tmpl.json" "$d/user.json" "$d/nb" "$skip" >/dev/null 2>/dev/null; rc=$?
    [ "$rc" -eq 0 ] || { fail "s19" "exit $rc"; return 1; }
    jq -e '. == []' "$skip" >/dev/null 2>&1 || { fail "s19" "skip not [] when no conflict: $(cat "$skip")"; return 1; }
    pass "s19: zero withheld keys -> skip file is []"
}

# ---------------------------------------------------------------------------
# Run scenarios
# ---------------------------------------------------------------------------
s1
s2
s3
s4
s5
s6
s7
s8
s9
s10
s11
s12
s13
s14
s15
s16
s17
s18
s19

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
