#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# doctor.test.sh: hermetic tests for the bash snippets embedded in
# commands/doctor.md (Layers 2, 5, 6).
#
# commands/doctor.md is markdown, not a shell script: each layer's check lives
# in a fenced ```bash block under a `## Layer N:` heading. This suite EXTRACTS
# those blocks with awk, keyed on the heading and the following fenced block,
# and runs the extracted text through `bash -c`. That is deliberate: the
# snippet bodies are never copied into this file, so what is under test is
# always exactly what ships, and the two cannot drift apart.
#
# Layer 2 is the regression pin for the fail-open defect the note
# hook-rename-lockstep-settings records: a guard named in settings.json but
# missing (or non-executable) on disk used to read as healthy. Layer 5 and
# Layer 6 get their first coverage here too, since the extraction harness
# makes it nearly free once Layer 2 has it.
#
# Run:  bash shell/doctor.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DOCTOR_MD="$SCRIPT_DIR/../commands/doctor.md"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1"; (( FAIL++ )) || true; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

run_scenario() {
  local name="$1" fn="$2"
  if "$fn" 2>&1; then pass "$name"; else fail "$name"; fi
}

# Extract the first fenced ```bash block that follows a `## Layer N:` heading,
# stopping at the block's closing fence. Bails out (prints nothing) if a
# different `## Layer` heading is reached first, so a layer with no bash
# block under it fails loudly rather than silently grabbing a neighbour's.
extract_snippet() {
  local heading="## Layer $1:"
  awk -v heading="$heading" '
  {
    if (found && incode) {
      if ($0 == "```") { exit }
      print
      next
    }
    if (found && index($0, "## Layer") == 1 && index($0, heading) != 1) { exit }
    if (index($0, heading) == 1) { found=1; next }
    if (found && $0 == "```bash") { incode=1; next }
  }
  ' "$DOCTOR_MD"
}

LAYER2="$(extract_snippet 2)"
LAYER5="$(extract_snippet 5)"
LAYER6="$(extract_snippet 6)"
LAYER7="$(extract_snippet 7)"

# A missing extraction would let every scenario below pass vacuously (bash -c
# "" exits 0 and prints nothing, which several assertions read as a match).
# Fail the whole suite up front rather than let that happen quietly.
for pair in "LAYER2:2" "LAYER5:5" "LAYER6:6" "LAYER7:7"; do
  var="${pair%%:*}"; num="${pair##*:}"
  [[ -n "${!var}" ]] || { echo "FATAL: could not extract Layer $num snippet from $DOCTOR_MD" >&2; exit 2; }
done

# ── Layer 2: safety guards wired ────────────────────────────────────────────

GUARDS=(rm-workspace-guard bg-await-guard no-slop-guard precommit-check)

# Args: home dir, then one "name:command" pair per hooks.PreToolUse entry to
# write. Lets a scenario wire a guard to an arbitrary command string (its
# bare binary form, its legacy .sh form, or a near-miss), not just its name.
write_wired_settings_raw() {
  local home="$1"; shift
  mkdir -p "$home/.claude"
  local entries="" pair name cmd
  for pair in "$@"; do
    name="${pair%%:*}"; cmd="${pair#*:}"
    entries="${entries}{\"hooks\":[{\"command\":\"$cmd\"}]},"
  done
  entries="${entries%,}"
  printf '{"hooks":{"PreToolUse":[%s]}}' "$entries" > "$home/.claude/settings.json"
}

# Args: home dir, then the guard names to wire into settings.json's
# PreToolUse hooks in their bare `playbook hook <name>` form. A guard
# omitted here is not wired at all.
write_wired_settings() {
  local home="$1"; shift
  local pairs=() name
  for name in "$@"; do
    pairs+=("$name:playbook hook $name")
  done
  write_wired_settings_raw "$home" "${pairs[@]}"
}

run_layer2() {
  local home="$1"
  HOME="$home" bash -c "$LAYER2" 2>&1
}

# A: all four guards wired in their bare binary form.
scenario_layer2_all_wired() {
  local home="$WORK/l2-a" out
  write_wired_settings "$home" "${GUARDS[@]}"
  out="$(run_layer2 "$home")"
  [[ "$out" == "wired=4/4" ]] || { echo "  got: $out"; return 1; }
}

# B: precommit-check is still on its legacy `.sh` command from before this
# fix shipped, not the bare form. Regression pin: a stale, un-migrated entry
# must read as NOT_WIRED, not as wired, since it is not actually running
# from the binary.
scenario_layer2_legacy_command_not_wired() {
  local home="$WORK/l2-b" out
  write_wired_settings_raw "$home" \
    "rm-workspace-guard:playbook hook rm-workspace-guard" \
    "bg-await-guard:playbook hook bg-await-guard" \
    "no-slop-guard:playbook hook no-slop-guard" \
    "precommit-check:~/.claude/hooks/precommit-check.sh"
  out="$(run_layer2 "$home")"
  [[ "$out" == *"precommit-check:NOT_WIRED"* ]] || { echo "  got: $out"; return 1; }
  [[ "$out" == "wired=3/4"* ]] || { echo "  wired count wrong: $out"; return 1; }
}

# C: a near-miss command that merely contains a guard's name as a substring
# must not count as that guard being wired. Pins the exact-match comparison
# in the snippet (`. == $cmd`), not a substring `contains`.
scenario_layer2_near_miss_command_not_wired() {
  local home="$WORK/l2-c" out
  write_wired_settings_raw "$home" \
    "rm-workspace-guard:playbook hook rm-workspace-guard-legacy" \
    "bg-await-guard:playbook hook bg-await-guard" \
    "no-slop-guard:playbook hook no-slop-guard" \
    "precommit-check:playbook hook precommit-check"
  out="$(run_layer2 "$home")"
  [[ "$out" == *"rm-workspace-guard:NOT_WIRED"* ]] || { echo "  got: $out"; return 1; }
}

# D: bg-await-guard is not wired into settings.json at all.
scenario_layer2_not_wired() {
  local home="$WORK/l2-d" out
  write_wired_settings "$home" rm-workspace-guard no-slop-guard precommit-check
  out="$(run_layer2 "$home")"
  [[ "$out" == *"bg-await-guard:NOT_WIRED"* ]] || { echo "  got: $out"; return 1; }
}

# E: the fourth-guard regression, specifically. The previous version of this
# layer matched only three guard names and passed on "3 or more wired", so a
# missing precommit-check read as healthy. Wire only the other three and
# require the count to say 3/4, not 4/4, and to name precommit-check as
# NOT_WIRED. A test that only checked the other three guards would let this
# exact regression back in.
scenario_layer2_precommit_check_counted() {
  local home="$WORK/l2-e" out
  write_wired_settings "$home" rm-workspace-guard bg-await-guard no-slop-guard
  out="$(run_layer2 "$home")"
  [[ "$out" == "wired=3/4"* ]] || { echo "  wired count did not drop: $out"; return 1; }
  [[ "$out" == *"precommit-check:NOT_WIRED"* ]] || { echo "  got: $out"; return 1; }
}

run_scenario "A: all four guards wired in bare form -> wired=4/4"                   scenario_layer2_all_wired
run_scenario "B: guard still on its legacy .sh command -> NOT_WIRED"                scenario_layer2_legacy_command_not_wired
run_scenario "C: a near-miss command must not count as wired (exact-match pin)"     scenario_layer2_near_miss_command_not_wired
run_scenario "D: guard not wired at all -> NOT_WIRED"                               scenario_layer2_not_wired
run_scenario "E: precommit-check is counted, not silently dropped to '3 or more'"   scenario_layer2_precommit_check_counted

# ── Layer 5: status line matches the shipped copy ───────────────────────────

# statusLine.command is written with a literal "$HOME" token (single-quoted
# heredoc, so bash does not expand it here): the snippet itself substitutes
# $HOME at run time, so this fixture matches how a real settings.json seeds
# the value via the installer's home-relative path.
write_statusline_settings() {
  local home="$1" cmd="$2"
  mkdir -p "$home/.claude"
  printf '{"statusLine":{"command":"%s"}}' "$cmd" > "$home/.claude/settings.json"
}

run_layer5() {
  local home="$1" plugin_root="${2:-}"
  HOME="$home" CLAUDE_PLUGIN_ROOT="$plugin_root" bash -c "$LAYER5" 2>&1
}

# F: statusLine.command names a path that does not exist on disk.
scenario_layer5_missing() {
  local home="$WORK/l5-f" out
  mkdir -p "$home/.claude"
  write_statusline_settings "$home" '$HOME/.claude/statusline.sh'
  out="$(run_layer5 "$home")"
  [[ "$out" == "MISSING $home/.claude/statusline.sh" ]] || { echo "  got: $out"; return 1; }
}

# G: the installed copy exists and is byte-identical to the shipped copy.
scenario_layer5_match() {
  local home="$WORK/l5-g" plugin="$WORK/l5-g-plugin" out
  mkdir -p "$home/.claude" "$plugin"
  printf '#!/usr/bin/env bash\necho hi\n' > "$home/.claude/statusline.sh"
  cp "$home/.claude/statusline.sh" "$plugin/statusline.sh"
  write_statusline_settings "$home" '$HOME/.claude/statusline.sh'
  out="$(run_layer5 "$home" "$plugin")"
  [[ "$out" == "MATCH" ]] || { echo "  got: $out"; return 1; }
}

# G2: the ADR 0012 destination, $HOME/.config/playbook/statusline.sh,
# byte-identical to the shipped copy.
scenario_layer5_match_config_playbook_path() {
  local home="$WORK/l5-g2" plugin="$WORK/l5-g2-plugin" out
  mkdir -p "$home/.claude" "$home/.config/playbook" "$plugin"
  printf '#!/usr/bin/env bash\necho hi\n' > "$home/.config/playbook/statusline.sh"
  cp "$home/.config/playbook/statusline.sh" "$plugin/statusline.sh"
  write_statusline_settings "$home" '$HOME/.config/playbook/statusline.sh'
  out="$(run_layer5 "$home" "$plugin")"
  [[ "$out" == "MATCH" ]] || { echo "  got: $out"; return 1; }
}

# H: the installed copy exists but its bytes differ from the shipped copy.
scenario_layer5_differs() {
  local home="$WORK/l5-h" plugin="$WORK/l5-h-plugin" out
  mkdir -p "$home/.claude" "$plugin"
  printf '#!/usr/bin/env bash\necho old\n' > "$home/.claude/statusline.sh"
  printf '#!/usr/bin/env bash\necho new\n' > "$plugin/statusline.sh"
  write_statusline_settings "$home" '$HOME/.claude/statusline.sh'
  out="$(run_layer5 "$home" "$plugin")"
  [[ "$out" == "DIFFERS $home/.claude/statusline.sh vs $plugin/statusline.sh" ]] || { echo "  got: $out"; return 1; }
}

# I: no statusLine.command at all.
scenario_layer5_not_configured() {
  local home="$WORK/l5-i" out
  mkdir -p "$home/.claude"
  printf '{}' > "$home/.claude/settings.json"
  out="$(run_layer5 "$home")"
  [[ "$out" == "NOT_CONFIGURED" ]] || { echo "  got: $out"; return 1; }
}

run_scenario "F: statusLine.command path does not exist -> MISSING <path>" scenario_layer5_missing
run_scenario "G: installed copy byte-identical to shipped -> MATCH"        scenario_layer5_match
run_scenario "G2: .config/playbook destination, byte-identical -> MATCH"   scenario_layer5_match_config_playbook_path
run_scenario "H: installed copy differs from shipped -> DIFFERS"           scenario_layer5_differs
run_scenario "I: no statusLine.command at all -> NOT_CONFIGURED"           scenario_layer5_not_configured

# ── Layer 6: binary resolves ────────────────────────────────────────────────

# A stub playbook on PATH, isolated to one scenario by prepending its bin dir
# to a fixed, minimal PATH rather than reusing the caller's.
write_stub_binary() {
  local bindir="$1" version_line="$2"
  mkdir -p "$bindir"
  cat > "$bindir/playbook" <<STUB
#!/usr/bin/env bash
[ "\$1" = "--version" ] && printf '%s\n' "$version_line"
STUB
  chmod +x "$bindir/playbook"
}

write_manifest() {
  local plugin_root="$1" version="$2"
  mkdir -p "$plugin_root/.claude-plugin"
  printf '{"version":"%s"}' "$version" > "$plugin_root/.claude-plugin/plugin.json"
}

run_layer6() {
  local home="$1" path="$2" plugin_root="${3:-}"
  HOME="$home" PATH="$path" CLAUDE_PLUGIN_ROOT="$plugin_root" bash -c "$LAYER6" 2>&1
}

# J: playbook is absent from PATH entirely.
scenario_layer6_missing() {
  local home="$WORK/l6-j" out
  mkdir -p "$home"
  out="$(run_layer6 "$home" "/usr/bin:/bin")"
  [[ "$out" == "MISSING" ]] || { echo "  got: $out"; return 1; }
}

# K: the stub reports 0.10.0 and the manifest agrees -> MATCH.
scenario_layer6_match() {
  local home="$WORK/l6-k" bin="$WORK/l6-k-bin" plugin="$WORK/l6-k-plugin" out
  mkdir -p "$home"
  write_stub_binary "$bin" "playbook 0.10.0"
  write_manifest "$plugin" "0.10.0"
  out="$(run_layer6 "$home" "$bin:/usr/bin:/bin" "$plugin")"
  [[ "$out" == "MATCH 0.10.0" ]] || { echo "  got: $out"; return 1; }
}

# L: same stub, manifest reports a different version -> SKEW, both named.
scenario_layer6_skew() {
  local home="$WORK/l6-l" bin="$WORK/l6-l-bin" plugin="$WORK/l6-l-plugin" out
  mkdir -p "$home"
  write_stub_binary "$bin" "playbook 0.10.0"
  write_manifest "$plugin" "0.9.1"
  out="$(run_layer6 "$home" "$bin:/usr/bin:/bin" "$plugin")"
  [[ "$out" == "SKEW binary=0.10.0 plugin=0.9.1" ]] || { echo "  got: $out"; return 1; }
}

# M: the stub prints nothing for --version, so the resolved binary is not the
# one this plugin expects (stale shim or a name collision).
scenario_layer6_no_version() {
  local home="$WORK/l6-m" bin="$WORK/l6-m-bin" out
  mkdir -p "$home"
  write_stub_binary "$bin" ""
  out="$(run_layer6 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "NO_VERSION" ]] || { echo "  got: $out"; return 1; }
}

# N: the stub resolves but no manifest can be found to compare against.
scenario_layer6_present_no_baseline() {
  local home="$WORK/l6-n" bin="$WORK/l6-n-bin" out
  mkdir -p "$home"
  write_stub_binary "$bin" "playbook 0.10.0"
  out="$(run_layer6 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "PRESENT_NO_BASELINE 0.10.0" ]] || { echo "  got: $out"; return 1; }
}

run_scenario "J: playbook absent from PATH -> MISSING"                         scenario_layer6_missing
run_scenario "K: binary and manifest agree -> MATCH <ver>"                     scenario_layer6_match
run_scenario "L: binary and manifest disagree -> SKEW binary=.. plugin=.."     scenario_layer6_skew
run_scenario "M: --version prints nothing -> NO_VERSION"                       scenario_layer6_no_version
run_scenario "N: binary present, no manifest found -> PRESENT_NO_BASELINE"     scenario_layer6_present_no_baseline

# ── Layer 7: no hook command points at a missing file ───────────────────────

run_layer7() {
  local home="$1"
  HOME="$home" bash -c "$LAYER7" 2>&1
}

# O: every hook command is the bare `playbook hook <name>` form, across two
# different events. None of them look like a path, so nothing is checked.
scenario_layer7_all_bare_healthy() {
  local home="$WORK/l7-o" out
  mkdir -p "$home/.claude"
  printf '{"hooks":{"PreToolUse":[{"hooks":[{"command":"playbook hook rm-workspace-guard"}]}],"Stop":[{"hooks":[{"command":"playbook hook memory-capture"}]}]}}' \
    > "$home/.claude/settings.json"
  out="$(run_layer7 "$home")"
  [[ "$out" == "checked=0 dangling=" ]] || { echo "  got: $out"; return 1; }
}

# P: the exact shape the orphaned memory_context.py incident had, a leftover
# Python hook command naming a file that is no longer on disk. Regression pin
# for that incident.
scenario_layer7_dangling_python_hook() {
  local home="$WORK/l7-p" out
  mkdir -p "$home/.claude"
  printf '{"hooks":{"PreToolUse":[{"hooks":[{"command":"python3 ~/.claude/hooks/memory_context.py"}]}]}}' \
    > "$home/.claude/settings.json"
  out="$(run_layer7 "$home")"
  [[ "$out" == "checked=1 dangling=python3 ~/.claude/hooks/memory_context.py" ]] || { echo "  got: $out"; return 1; }
}

# Q: a legacy `.sh` command left over from before a hook was ported, on a
# machine where the script itself is gone. A second, independent path shape.
scenario_layer7_dangling_legacy_guard() {
  local home="$WORK/l7-q" out
  mkdir -p "$home/.claude"
  printf '{"hooks":{"Stop":[{"hooks":[{"command":"~/.claude/hooks/retired-guard.sh"}]}]}}' \
    > "$home/.claude/settings.json"
  out="$(run_layer7 "$home")"
  [[ "$out" == "checked=1 dangling=~/.claude/hooks/retired-guard.sh" ]] || { echo "  got: $out"; return 1; }
}

# R: a path-shaped command whose file genuinely exists must not be flagged.
scenario_layer7_existing_path_not_flagged() {
  local home="$WORK/l7-r" out
  mkdir -p "$home/.claude/hooks"
  touch "$home/.claude/hooks/real.sh"
  printf '{"hooks":{"PreToolUse":[{"hooks":[{"command":"%s/.claude/hooks/real.sh"}]}]}}' "$home" \
    > "$home/.claude/settings.json"
  out="$(run_layer7 "$home")"
  [[ "$out" == "checked=1 dangling=" ]] || { echo "  got: $out"; return 1; }
}

# S: the same dangling command wired under two different events, the real
# session-clean-exit precedent (one hook name legitimately wired on both Stop
# and SessionEnd), must be reported once, not twice, even though both count
# toward `checked`.
scenario_layer7_duplicate_across_events_deduped() {
  local home="$WORK/l7-s" out
  mkdir -p "$home/.claude"
  printf '{"hooks":{"Stop":[{"hooks":[{"command":"~/.claude/hooks/gone.sh"}]}],"SessionEnd":[{"hooks":[{"command":"~/.claude/hooks/gone.sh"}]}]}}' \
    > "$home/.claude/settings.json"
  out="$(run_layer7 "$home")"
  [[ "$out" == "checked=2 dangling=~/.claude/hooks/gone.sh" ]] || { echo "  got: $out"; return 1; }
}

# T: a command whose path still contains an unresolved environment variable
# after ~/$HOME substitution must be skipped, not falsely flagged as missing.
scenario_layer7_unresolved_var_skipped() {
  local home="$WORK/l7-t" out
  mkdir -p "$home/.claude"
  printf '{"hooks":{"PreToolUse":[{"hooks":[{"command":"bash $CLAUDE_PLUGIN_ROOT/hooks/foo.sh"}]}]}}' \
    > "$home/.claude/settings.json"
  out="$(run_layer7 "$home")"
  [[ "$out" == "checked=0 dangling=" ]] || { echo "  got: $out"; return 1; }
}

run_scenario "O: every command is the bare form -> nothing checked, nothing dangling" scenario_layer7_all_bare_healthy
run_scenario "P: leftover Python hook command, file gone -> dangling (memory_context.py regression pin)" scenario_layer7_dangling_python_hook
run_scenario "Q: leftover legacy .sh guard command, file gone -> dangling"     scenario_layer7_dangling_legacy_guard
run_scenario "R: path-shaped command whose file exists -> not flagged"        scenario_layer7_existing_path_not_flagged
run_scenario "S: same dangling command on two events -> reported once"        scenario_layer7_duplicate_across_events_deduped
run_scenario "T: unresolved \$VAR in path -> skipped, not flagged"            scenario_layer7_unresolved_var_skipped

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
