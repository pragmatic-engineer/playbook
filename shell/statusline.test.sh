#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# statusline.test.sh: unit tests for pure-helper functions in statusline.sh.
# Sources the target with stdin redirected to /dev/null so the source-guard
# prevents any side effects (no cache dir created, no stdin read, no render).
#
# Run:  bash shell/statusline.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=statusline.sh
source "$SCRIPT_DIR/../statusline.sh" </dev/null

PASS=0
FAIL=0

assert_eq() {
    local name="$1" got="$2" want="$3"
    if [[ "$got" == "$want" ]]; then
        echo "PASS: $name"
        (( PASS++ )) || true
    else
        printf 'FAIL: %s\n  got:  %q\n  want: %q\n' "$name" "$got" "$want" >&2
        (( FAIL++ )) || true
    fi
}

# ── ctx_bar ──────────────────────────────────────────────────────────────────
# Filled cells use █, empty cells use ░. Default width is 10.
# ctx_bar 50 10: filled=(50*10+50)/100=5, empty=5 → █████░░░░░
assert_eq "ctx_bar 50 10 (half fill)"  "$(ctx_bar 50 10)" '█████░░░░░'
# ctx_bar 0: filled=0, empty=10
assert_eq "ctx_bar 0 (empty bar)"      "$(ctx_bar 0)"     '░░░░░░░░░░'
# ctx_bar 100: filled=10, empty=0
assert_eq "ctx_bar 100 (full bar)"     "$(ctx_bar 100)"   '██████████'

# ── fmt_age ──────────────────────────────────────────────────────────────────
assert_eq "fmt_age 5 (seconds)"        "$(fmt_age 5)"     '5s'
assert_eq "fmt_age 90 (minutes)"       "$(fmt_age 90)"    '1m'
assert_eq "fmt_age 3660 (1h 1m)"       "$(fmt_age 3660)"  '1h1m'
assert_eq "fmt_age 7200 (exact hours)" "$(fmt_age 7200)"  '2h'

# ── fmt_ago ──────────────────────────────────────────────────────────────────
# Coarse "N ago" ladder: whole minutes < 1h, whole hours < 1d, whole days else.
# These pin the exact pre-refactor ladder outputs so the dedupe is provably
# behavior-preserving.
assert_eq "fmt_ago 0 (0m)"        "$(fmt_ago 0)"      '0m'
assert_eq "fmt_ago 1800 (30m)"    "$(fmt_ago 1800)"   '30m'
assert_eq "fmt_ago 3600 (1h)"     "$(fmt_ago 3600)"   '1h'
assert_eq "fmt_ago 7200 (2h)"     "$(fmt_ago 7200)"   '2h'
assert_eq "fmt_ago 86400 (1d)"    "$(fmt_ago 86400)"  '1d'
assert_eq "fmt_ago 172800 (2d)"   "$(fmt_ago 172800)" '2d'

# ── cache_hit_pct ─────────────────────────────────────────────────────────────
# Empty when no cache activity (total=0).
assert_eq "cache_hit_pct 0 0 (empty)"      "$(cache_hit_pct 0 0)"    ''
# (150*100)/(50+150) = 75
assert_eq "cache_hit_pct 50 150 (75 pct)"  "$(cache_hit_pct 50 150)" '75'

# ── iso_to_epoch ──────────────────────────────────────────────────────────────
# Fixed UTC date; both GNU and BSD date branches produce the same epoch.
# 2026-07-08T00:00:00Z = 1 783 468 800 (verified: 20642 days × 86400 s).
assert_eq "iso_to_epoch 2026-07-08" \
    "$(iso_to_epoch '2026-07-08T00:00:00Z')" '1783468800'

# ── cache_slug ────────────────────────────────────────────────────────────────
# Non-alphanumeric chars (/, space) become underscores.
assert_eq "cache_slug 'a/b c'" "$(cache_slug 'a/b c')" 'a_b_c'

# ── strip_ansi + visible_len ──────────────────────────────────────────────────
_ansi_str=$'\033[1;32mabc\033[0m'
assert_eq "strip_ansi removes escapes"  "$(strip_ansi "$_ansi_str")"  'abc'
assert_eq "visible_len of ANSI string"  "$(visible_len "$_ansi_str")" '3'

# ── cache_color ───────────────────────────────────────────────────────────────
# Thresholds: >=80 → GREEN, >=50 → YELLOW, <50 → RED.
# printf '%b' interprets the \033 escape in the colour vars to a real ESC byte.
_c_green=$'\033[38;2;166;227;161m'
_c_yellow=$'\033[38;2;249;226;175m'
_c_red=$'\033[38;2;243;139;168m'
assert_eq "cache_color 80 (green)"   "$(cache_color 80)" "$_c_green"
assert_eq "cache_color 60 (yellow)"  "$(cache_color 60)" "$_c_yellow"
assert_eq "cache_color 49 (red)"     "$(cache_color 49)" "$_c_red"

# ── cost_per_min ──────────────────────────────────────────────────────────────
# Empty when wall_ms <= 0; otherwise (cost * 60000) / wall_ms formatted %.4f.
assert_eq "cost_per_min 0 0 (empty)"    "$(cost_per_min 0 0)"       ''
assert_eq "cost_per_min 1.0 60000"      "$(cost_per_min 1.0 60000)" '1.0000'

# ── fmt_tokens ────────────────────────────────────────────────────────────────
# Bare integer under 1000; "NNNk" rounded to the nearest thousand at/above it.
assert_eq "fmt_tokens 0 (bare)"              "$(fmt_tokens 0)"      '0'
assert_eq "fmt_tokens 999 (bare, boundary)"  "$(fmt_tokens 999)"    '999'
assert_eq "fmt_tokens 1000 (exact k)"        "$(fmt_tokens 1000)"   '1k'
assert_eq "fmt_tokens 127000 (127k exact)"   "$(fmt_tokens 127000)" '127k'
assert_eq "fmt_tokens 127600 (rounds up)"    "$(fmt_tokens 127600)" '128k'
assert_eq "fmt_tokens 127400 (rounds down)"  "$(fmt_tokens 127400)" '127k'

# ── context_rot_warning ──────────────────────────────────────────────────────
# Empty below the threshold, and on empty input; '1' at and above it.
assert_eq "context_rot_warning 199999 (below)" "$(context_rot_warning 199999)" ''
assert_eq "context_rot_warning 200000 (at)"    "$(context_rot_warning 200000)" '1'
assert_eq "context_rot_warning 250000 (above)" "$(context_rot_warning 250000)" '1'
assert_eq "context_rot_warning '' (no data)"   "$(context_rot_warning '')"     ''

# ── compact_gap ───────────────────────────────────────────────────────────────
# Empty when used < 50%; otherwise (trigger - used), clamped to 0.
# Pin CLAUDE_AUTOCOMPACT_PCT_OVERRIDE so results are deterministic regardless
# of what the calling shell exports.
assert_eq "compact_gap 30 (< 50, empty)" \
    "$(CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=90 compact_gap 30)" ''
assert_eq "compact_gap 80 (gap=10, trigger=90)" \
    "$(CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=90 compact_gap 80)" '10'

# ── telemetry (WU-4) ──────────────────────────────────────────────────────────
# The telemetry write lives inside the run-guard, right after the stdin parse,
# so it only fires when the script is executed, not sourced. These scenarios
# run statusline.sh as a subprocess with an isolated HOME (never the real
# ~/.claude/runtime/) and inspect what it wrote and whether it still rendered.

# Build the minimal stdin payload statusline.sh expects. cwd points at the
# fake HOME (never a real git repo) so the run stays fast and offline.
_telemetry_payload() {
    local sid="$1" used="$2" cost="$3" cwd="$4"
    if [[ -n "$sid" ]]; then
        printf '{"session_id":"%s","cwd":"%s","cost":{"total_cost_usd":%s},"context_window":{"used_percentage":%s}}' \
            "$sid" "$cwd" "$cost" "$used"
    else
        printf '{"cwd":"%s","cost":{"total_cost_usd":%s},"context_window":{"used_percentage":%s}}' \
            "$cwd" "$cost" "$used"
    fi
}

# yes/no so results compose with assert_eq like every other scenario here.
_file_exists() { [[ -f "$1" ]] && echo yes || echo no; }
_file_has_both() {
    local file="$1" needle1="$2" needle2="$3"
    [[ -f "$file" ]] || { echo no; return; }
    if grep -q "$needle1" "$file" 2>/dev/null && grep -q "$needle2" "$file" 2>/dev/null; then
        echo yes
    else
        echo no
    fi
}

# 1. Sample appended: cost and usage both land in one telemetry.jsonl line.
t1_home=$(mktemp -d)
t1_sid="sess-sample"
t1_payload=$(_telemetry_payload "$t1_sid" 42 1.5 "$t1_home")
HOME="$t1_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t1_payload" >/dev/null 2>&1
assert_eq "telemetry sample has cost and usage" \
    "$(_file_has_both "$t1_home/.claude/runtime/$t1_sid/telemetry.jsonl" '"cost_usd":1.5' '"used_pct":42')" \
    "yes"
rm -rf "$t1_home"

# 2. Threshold sets the marker: 75 with the default threshold (70).
t2_home=$(mktemp -d)
t2_sid="sess-over"
t2_payload=$(_telemetry_payload "$t2_sid" 75 0.1 "$t2_home")
HOME="$t2_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t2_payload" >/dev/null 2>&1
assert_eq "capture-due set at 75 percent (default threshold)" \
    "$(_file_exists "$t2_home/.claude/runtime/$t2_sid/capture-due")" "yes"
rm -rf "$t2_home"

# 3. Below threshold: 40 leaves no marker.
t3_home=$(mktemp -d)
t3_sid="sess-under"
t3_payload=$(_telemetry_payload "$t3_sid" 40 0.1 "$t3_home")
HOME="$t3_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t3_payload" >/dev/null 2>&1
assert_eq "capture-due absent at 40 percent" \
    "$(_file_exists "$t3_home/.claude/runtime/$t3_sid/capture-due")" "no"
rm -rf "$t3_home"

# 4. Threshold is overridable: CC_CAPTURE_AT=30 with usage of 40.
t4_home=$(mktemp -d)
t4_sid="sess-override"
t4_payload=$(_telemetry_payload "$t4_sid" 40 0.1 "$t4_home")
CC_CAPTURE_AT=30 HOME="$t4_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t4_payload" >/dev/null 2>&1
assert_eq "capture-due honours CC_CAPTURE_AT override" \
    "$(_file_exists "$t4_home/.claude/runtime/$t4_sid/capture-due")" "yes"
rm -rf "$t4_home"

# 4b. REGRESSION: staying above the threshold fires exactly ONCE, not per render.
#
# The marker used to be re-dropped on every render while usage sat at or above
# the threshold, while the memory-capture hook consumes it every Stop. Past 70%
# that cost a turn every turn for the rest of the session, and the hook's own
# message claims it "fires once per threshold crossing". Observed firing four
# times in a row with a byte identical edited-files list.
t4b_home=$(mktemp -d)
t4b_sid="sess-latch"
t4b_dir="$t4b_home/.claude/runtime/$t4b_sid"
t4b_payload=$(_telemetry_payload "$t4b_sid" 75 0.1 "$t4b_home")

HOME="$t4b_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t4b_payload" >/dev/null 2>&1
assert_eq "first render above threshold drops capture-due" \
    "$(_file_exists "$t4b_dir/capture-due")" "yes"

# Consume it the way the Stop hook does, then render again while still above.
rm -f "$t4b_dir/capture-due"
HOME="$t4b_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t4b_payload" >/dev/null 2>&1
assert_eq "second render above threshold does NOT re-drop capture-due" \
    "$(_file_exists "$t4b_dir/capture-due")" "no"

HOME="$t4b_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t4b_payload" >/dev/null 2>&1
assert_eq "third render above threshold still does NOT re-drop capture-due" \
    "$(_file_exists "$t4b_dir/capture-due")" "no"

# Dropping back under the line re-arms, so a genuine second crossing still fires.
t4b_low=$(_telemetry_payload "$t4b_sid" 40 0.1 "$t4b_home")
HOME="$t4b_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t4b_low" >/dev/null 2>&1
HOME="$t4b_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t4b_payload" >/dev/null 2>&1
assert_eq "crossing again after dropping under the line fires once more" \
    "$(_file_exists "$t4b_dir/capture-due")" "yes"
rm -rf "$t4b_home"

# 4c. capture-crossings tallies one per threshold CROSSING, not per render.
t4c_home=$(mktemp -d)
t4c_sid="sess-crossings"
t4c_dir="$t4c_home/.claude/runtime/$t4c_sid"
t4c_high=$(_telemetry_payload "$t4c_sid" 75 0.1 "$t4c_home")
t4c_low=$(_telemetry_payload "$t4c_sid" 40 0.1 "$t4c_home")

for _ in 1 2 3; do
    HOME="$t4c_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t4c_high" >/dev/null 2>&1
    HOME="$t4c_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t4c_high" >/dev/null 2>&1
    HOME="$t4c_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t4c_low" >/dev/null 2>&1
done
assert_eq "capture-crossings reads exactly 3 after three cross/drop/re-cross cycles" \
    "$(cat "$t4c_dir/capture-crossings" 2>/dev/null)" "3"
rm -rf "$t4c_home"

# 5. Render survives an unwritable session dir (the ADR's safety pin).
t5_home=$(mktemp -d)
t5_sid="sess-unwritable"
t5_dir="$t5_home/.claude/runtime/$t5_sid"
mkdir -p "$t5_dir"
chmod 500 "$t5_dir"
t5_payload=$(_telemetry_payload "$t5_sid" 80 2.0 "$t5_home")
t5_out=$(HOME="$t5_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t5_payload" 2>&1)
t5_status=$?
assert_eq "render exits 0 with an unwritable session dir" "$t5_status" "0"
assert_eq "render still prints with an unwritable session dir" \
    "$( [[ -n "$t5_out" ]] && echo yes || echo no )" "yes"
chmod 700 "$t5_dir"
rm -rf "$t5_home"

# 6. Missing session_id is harmless: no write attempted, render still succeeds.
t6_home=$(mktemp -d)
t6_payload=$(_telemetry_payload "" 55 0.5 "$t6_home")
t6_out=$(HOME="$t6_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t6_payload" 2>&1)
t6_status=$?
assert_eq "render exits 0 with no session_id" "$t6_status" "0"
assert_eq "render still prints with no session_id" \
    "$( [[ -n "$t6_out" ]] && echo yes || echo no )" "yes"
assert_eq "no runtime dir created with no session_id" \
    "$( [[ -d "$t6_home/.claude/runtime" ]] && echo yes || echo no )" "no"
rm -rf "$t6_home"

# 7. A HOME holding glob metacharacters still collapses to ~ in the rendered
#    path. The strip pattern in ${display_path#"$HOME"} has to be quoted: left
#    bare, a HOME like /tmp/xxx/a[b]c is read as a PATTERN, matches the literal
#    "abc" rather than itself, strips nothing, and the status line renders the
#    whole absolute path. Scenarios 1 to 6 all use mktemp -d names, which never
#    contain [, * or ?, so none of them can see this.
t7_base=$(mktemp -d)
t7_home="$t7_base/a[b]c"
mkdir -p "$t7_home"
t7_payload=$(_telemetry_payload "sess-glob-home" 40 0.1 "$t7_home")
t7_out=$(HOME="$t7_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$t7_payload" 2>&1)
# Three claims, not one. "The absolute path is absent" on its own is satisfied
# by a render that printed nothing at all, so a crash here would read as a pass.
# Check that it rendered, that the path did not leak, and that the ~ collapse
# actually happened, and name each failure mode so a red suite says which.
if   [[ -z "$t7_out" ]];              then t7_verdict="empty-render"
elif [[ "$t7_out" == *"$t7_home"* ]]; then t7_verdict="leaked"
elif [[ "$t7_out" != *"~"* ]];        then t7_verdict="no-tilde"
else                                       t7_verdict="collapsed"
fi
assert_eq "glob-metachar HOME collapses to ~ in the rendered path" \
    "$t7_verdict" "collapsed"
rm -rf "$t7_base"

# 8. Context-rot warning renders at/above 200k total input tokens, and is
#    absent below it. Full payload (session_id, rate limits, cost) so line 2
#    and line 3 both have real content to assert on.
_full_payload() {
    local cwd="$1" tokens="$2" window="$3"
    printf '{"session_id":"sess-rot","cwd":"%s","model":{"display_name":"Sonnet 4.5"},"context_window":{"used_percentage":50,"total_input_tokens":%s,"context_window_size":%s,"current_usage":{"cache_creation_input_tokens":1000,"cache_read_input_tokens":1000}},"cost":{"total_cost_usd":0.1,"total_duration_ms":60000},"rate_limits":{"five_hour":{"used_percentage":10}}}' \
        "$cwd" "$tokens" "$window"
}

t8_home=$(mktemp -d)
t8_out=$(HOME="$t8_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$(_full_payload "$t8_home" 250000 300000)" 2>&1)
assert_eq "context-rot warning shown at 250k tokens" \
    "$( [[ "$t8_out" == *"context rot risk"* ]] && echo yes || echo no )" "yes"
assert_eq "token count shown as 250k/300k" \
    "$( [[ "$t8_out" == *"250k/300k"* ]] && echo yes || echo no )" "yes"
rm -rf "$t8_home"

t9_home=$(mktemp -d)
t9_out=$(HOME="$t9_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$(_full_payload "$t9_home" 45000 200000)" 2>&1)
assert_eq "context-rot warning absent at 45k tokens" \
    "$( [[ "$t9_out" == *"context rot risk"* ]] && echo yes || echo no )" "no"
rm -rf "$t9_home"

# 10. Line split: line 2 (model + context) never carries the 5h quota; line 3
#     carries it instead. Asserted on the actual two printed lines, not a
#     substring search over the whole output, so a regression that put 5h back
#     on line 2 fails here even though the text "5h" is present somewhere.
t10_home=$(mktemp -d)
t10_out=$(HOME="$t10_home" bash "$SCRIPT_DIR/../statusline.sh" <<< "$(_full_payload "$t10_home" 45000 200000)" 2>&1)
t10_line2=$(sed -n '2p' <<< "$t10_out")
t10_line3=$(sed -n '3p' <<< "$t10_out")
assert_eq "line 2 has no 5h quota" \
    "$( [[ "$t10_line2" == *"5h"* ]] && echo yes || echo no )" "no"
assert_eq "line 3 carries the 5h quota" \
    "$( [[ "$t10_line3" == *"5h"* ]] && echo yes || echo no )" "yes"
rm -rf "$t10_home"

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
