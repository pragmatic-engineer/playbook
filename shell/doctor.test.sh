#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0
#
# doctor.test.sh: hermetic tests for the bash snippets embedded in
# commands/doctor.md (Layers 2, 5, 6, 7).
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

# A stub playbook on PATH, isolated to one scenario by prepending its bin dir
# to a fixed, minimal PATH rather than reusing the caller's. Answers
# `--version` from `version_line`, and the four `doctor` subcommands
# `commands/doctor.md` now calls in place of `jq`, by reading the real field(s)
# straight out of whatever file it is pointed at, so a scenario's fixture
# content (written by `write_manifest` / `write_statusline_settings` / the
# Layer 2/7 settings.json fixtures) is the only thing that needs to vary, not
# the stub itself. Every `doctor` branch forces `exit 0` regardless of
# whether the match found anything or the path does not exist, matching the
# real subcommands: they always exit 0, empty output on any failure, never a
# nonzero exit for a merely-missing field, file, or hooks key. `hook-commands`
# and `hook-commands-for-event` are both a coarse `grep` over the whole file,
# not real JSON parsing (neither scopes to the `.hooks` key, and
# `hook-commands-for-event` does not scope further to its `event` argument the
# way the real subcommand does), which is fine here since no fixture in this
# suite has a `"command"` key outside `.hooks`, and no Layer 2 fixture wires
# more than one event.
write_stub_binary() {
  local bindir="$1" version_line="$2"
  local gate_help="${3:-Usage: playbook gate record <PLAN_SLUG> <COMMAND> <PHASE> <INPUT>}"
  mkdir -p "$bindir"
  cat > "$bindir/playbook" <<STUB
#!/usr/bin/env bash
if [ "\$1" = "--version" ]; then
  printf '%s\n' "$version_line"
elif [ "\$1" = "doctor" ] && [ "\$2" = "plugin-version" ]; then
  sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "\$3" 2>/dev/null
  exit 0
elif [ "\$1" = "doctor" ] && [ "\$2" = "statusline-command" ]; then
  sed -n 's/.*"statusLine"[[:space:]]*:[[:space:]]*{[^}]*"command"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "\$3" 2>/dev/null
  exit 0
elif [ "\$1" = "doctor" ] && [ "\$2" = "hook-commands" ]; then
  grep -o '"command"[[:space:]]*:[[:space:]]*"[^"]*"' "\$3" 2>/dev/null | sed 's/.*"command"[[:space:]]*:[[:space:]]*"\([^"]*\)"/\1/'
  exit 0
elif [ "\$1" = "doctor" ] && [ "\$2" = "hook-commands-for-event" ]; then
  settings_file="\$3"
  event="\$4"
  shift 4
  # The settings the scenarios write list PreToolUse, then PostToolUse, on one
  # line, so each event's entries are what comes before or after that key.
  case "\$event" in
    PostToolUse) segment=\$(sed 's/.*"PostToolUse"//' "\$settings_file" 2>/dev/null) ;;
    *) segment=\$(sed 's/"PostToolUse".*//' "\$settings_file" 2>/dev/null) ;;
  esac
  for guard in "\$@"; do
    wanted="playbook hook \$guard"
    count=\$(printf '%s' "\$segment" | grep -o '"command"[[:space:]]*:[[:space:]]*"[^"]*"' | sed 's/.*"command"[[:space:]]*:[[:space:]]*"\([^"]*\)"/\1/' | grep -Fxc -- "\$wanted")
    echo "\$guard=\$count"
  done
  exit 0
elif [ "\$1" = "gate" ] && [ "\$2" = "record" ] && [ "\$3" = "--help" ]; then
  printf '%s\n' "$gate_help"
  exit 0
fi
STUB
  chmod +x "$bindir/playbook"
}

# ── Layer 2: safety guards wired ────────────────────────────────────────────

GUARDS=(rm-workspace-guard bg-await-guard no-slop-guard precommit-check commit-message-sanitizer post/commit-message-sanitizer)

# Args: home dir, then one "name:command" pair per hook entry to write: under
# hooks.PreToolUse, or under hooks.PostToolUse for a name that starts with
# "post/". Lets a scenario wire a guard to an arbitrary command string (its
# bare binary form, its legacy .sh form, or a near-miss), not just its name.
write_wired_settings_raw() {
  local home="$1"; shift
  mkdir -p "$home/.claude"
  local pre="" post="" pair name cmd entry
  for pair in "$@"; do
    name="${pair%%:*}"; cmd="${pair#*:}"
    entry="{\"hooks\":[{\"command\":\"$cmd\"}]},"
    case "$name" in
      post/*) post="${post}${entry}" ;;
      *) pre="${pre}${entry}" ;;
    esac
  done
  printf '{"hooks":{"PreToolUse":[%s],"PostToolUse":[%s]}}' "${pre%,}" "${post%,}" > "$home/.claude/settings.json"
}

# Args: home dir, then the guard names to wire into settings.json in their
# bare `playbook hook <name>` form, a "post/" name on PostToolUse. A guard
# omitted here is not wired at all.
write_wired_settings() {
  local home="$1"; shift
  local pairs=() name
  for name in "$@"; do
    pairs+=("$name:playbook hook ${name#post/}")
  done
  write_wired_settings_raw "$home" "${pairs[@]}"
}

run_layer2() {
  local home="$1" path="$2"
  HOME="$home" PATH="$path" bash -c "$LAYER2" 2>&1
}

# A: all five guards and the sanitizer's backstop wired in their bare binary form.
scenario_layer2_all_wired() {
  local home="$WORK/l2-a" bin="$WORK/l2-a-bin" out
  write_wired_settings "$home" "${GUARDS[@]}"
  write_stub_binary "$bin" ""
  out="$(run_layer2 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "wired=6/6" ]] || { echo "  got: $out"; return 1; }
}

# B: precommit-check is still on its legacy `.sh` command from before this
# fix shipped, not the bare form. Regression pin: a stale, un-migrated entry
# must read as NOT_WIRED, not as wired, since it is not actually running
# from the binary.
scenario_layer2_legacy_command_not_wired() {
  local home="$WORK/l2-b" bin="$WORK/l2-b-bin" out
  write_wired_settings_raw "$home" \
    "rm-workspace-guard:playbook hook rm-workspace-guard" \
    "bg-await-guard:playbook hook bg-await-guard" \
    "no-slop-guard:playbook hook no-slop-guard" \
    "precommit-check:~/.claude/hooks/precommit-check.sh" \
    "commit-message-sanitizer:playbook hook commit-message-sanitizer" \
    "post/commit-message-sanitizer:playbook hook commit-message-sanitizer"
  write_stub_binary "$bin" ""
  out="$(run_layer2 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == *"precommit-check:NOT_WIRED"* ]] || { echo "  got: $out"; return 1; }
  [[ "$out" == "wired=5/6"* ]] || { echo "  wired count wrong: $out"; return 1; }
}

# C: a near-miss command that merely contains a guard's name as a substring
# must not count as that guard being wired. Pins the exact-match comparison
# in the snippet (`. == $cmd`), not a substring `contains`.
scenario_layer2_near_miss_command_not_wired() {
  local home="$WORK/l2-c" bin="$WORK/l2-c-bin" out
  write_wired_settings_raw "$home" \
    "rm-workspace-guard:playbook hook rm-workspace-guard-legacy" \
    "bg-await-guard:playbook hook bg-await-guard" \
    "no-slop-guard:playbook hook no-slop-guard" \
    "precommit-check:playbook hook precommit-check" \
    "commit-message-sanitizer:playbook hook commit-message-sanitizer" \
    "post/commit-message-sanitizer:playbook hook commit-message-sanitizer"
  write_stub_binary "$bin" ""
  out="$(run_layer2 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == *"rm-workspace-guard:NOT_WIRED"* ]] || { echo "  got: $out"; return 1; }
}

# D: bg-await-guard is not wired into settings.json at all.
scenario_layer2_not_wired() {
  local home="$WORK/l2-d" bin="$WORK/l2-d-bin" out
  write_wired_settings "$home" rm-workspace-guard no-slop-guard precommit-check commit-message-sanitizer post/commit-message-sanitizer
  write_stub_binary "$bin" ""
  out="$(run_layer2 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == *"bg-await-guard:NOT_WIRED"* ]] || { echo "  got: $out"; return 1; }
}

# E: the fourth-guard regression, specifically. The previous version of this
# layer matched only three guard names and passed on "3 or more wired", so a
# missing precommit-check read as healthy. Wire everything but that one and
# require the count to say 5/6, not 6/6, and to name precommit-check as
# NOT_WIRED. A test that only checked the other three guards would let this
# exact regression back in.
scenario_layer2_precommit_check_counted() {
  local home="$WORK/l2-e" bin="$WORK/l2-e-bin" out
  write_wired_settings "$home" rm-workspace-guard bg-await-guard no-slop-guard commit-message-sanitizer post/commit-message-sanitizer
  write_stub_binary "$bin" ""
  out="$(run_layer2 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "wired=5/6"* ]] || { echo "  wired count did not drop: $out"; return 1; }
  [[ "$out" == *"precommit-check:NOT_WIRED"* ]] || { echo "  got: $out"; return 1; }
}

# F: the commit-message-sanitizer is part of the count too. Leave only its
# PreToolUse entry out and require 5/6 and a NOT_WIRED naming it.
scenario_layer2_commit_message_guard_counted() {
  local home="$WORK/l2-f" bin="$WORK/l2-f-bin" out
  write_wired_settings "$home" rm-workspace-guard bg-await-guard no-slop-guard precommit-check post/commit-message-sanitizer
  write_stub_binary "$bin" ""
  out="$(run_layer2 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "wired=5/6"* ]] || { echo "  wired count did not drop: $out"; return 1; }
  [[ "$out" == *" commit-message-sanitizer:NOT_WIRED"* ]] || { echo "  got: $out"; return 1; }
}

# G: the backstop is checked on PostToolUse. Wire the five PreToolUse guards
# and not the backstop: 5/6 and a NOT_WIRED that names the event, never a
# pass because the same command is wired on PreToolUse.
scenario_layer2_backstop_counted() {
  local home="$WORK/l2-g" bin="$WORK/l2-g-bin" out
  write_wired_settings "$home" rm-workspace-guard bg-await-guard no-slop-guard precommit-check commit-message-sanitizer
  write_stub_binary "$bin" ""
  out="$(run_layer2 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "wired=5/6"* ]] || { echo "  wired count did not drop: $out"; return 1; }
  [[ "$out" == *"commit-message-sanitizer(PostToolUse):NOT_WIRED"* ]] || { echo "  got: $out"; return 1; }
  [[ "$out" != *" commit-message-sanitizer:NOT_WIRED"* ]] || { echo "  the PreToolUse entry is wired: $out"; return 1; }
}

run_scenario "A: all guards and the backstop wired in bare form -> wired=6/6"       scenario_layer2_all_wired
run_scenario "B: guard still on its legacy .sh command -> NOT_WIRED"                scenario_layer2_legacy_command_not_wired
run_scenario "C: a near-miss command must not count as wired (exact-match pin)"     scenario_layer2_near_miss_command_not_wired
run_scenario "D: guard not wired at all -> NOT_WIRED"                               scenario_layer2_not_wired
run_scenario "E: precommit-check is counted, not silently dropped to '5 or more'"   scenario_layer2_precommit_check_counted
run_scenario "F: commit-message-sanitizer is counted -> NOT_WIRED when missing"     scenario_layer2_commit_message_guard_counted
run_scenario "G: the PostToolUse backstop is counted -> NOT_WIRED(PostToolUse)"     scenario_layer2_backstop_counted

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

# A stub playbook that only understands `--version`, simulating a real
# binary built before the `doctor` subcommand shipped: any other invocation
# exits 2, matching clap's behavior for an unrecognized subcommand.
write_stub_binary_without_doctor() {
  local bindir="$1" version_line="$2"
  mkdir -p "$bindir"
  cat > "$bindir/playbook" <<STUB
#!/usr/bin/env bash
if [ "\$1" = "--version" ]; then
  printf '%s\n' "$version_line"
  exit 0
fi
exit 2
STUB
  chmod +x "$bindir/playbook"
}

run_layer5() {
  local home="$1" path="$2" plugin_root="${3:-}"
  HOME="$home" PATH="$path" CLAUDE_PLUGIN_ROOT="$plugin_root" bash -c "$LAYER5" 2>&1
}

# F: statusLine.command names a path that does not exist on disk.
scenario_layer5_missing() {
  local home="$WORK/l5-f" bin="$WORK/l5-f-bin" out
  mkdir -p "$home/.claude"
  write_statusline_settings "$home" '$HOME/.claude/statusline.sh'
  write_stub_binary "$bin" ""
  out="$(run_layer5 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "MISSING $home/.claude/statusline.sh" ]] || { echo "  got: $out"; return 1; }
}

# G: a custom command whose file exists is reported as CUSTOM.
scenario_layer5_match() {
  local home="$WORK/l5-g" plugin="$WORK/l5-g-plugin" bin="$WORK/l5-g-bin" out
  mkdir -p "$home/.claude" "$plugin"
  printf '#!/usr/bin/env bash\necho hi\n' > "$home/.claude/statusline.sh"
  cp "$home/.claude/statusline.sh" "$plugin/statusline.sh"
  write_statusline_settings "$home" '$HOME/.claude/statusline.sh'
  write_stub_binary "$bin" ""
  out="$(run_layer5 "$home" "$bin:/usr/bin:/bin" "$plugin")"
  [[ "$out" == 'CUSTOM $HOME/.claude/statusline.sh' ]] || { echo "  got: $out"; return 1; }
}

# G2: the old bash command at $HOME/.config/playbook/statusline.sh is flagged
# OUTDATED even when the script is byte-identical to the shipped copy.
scenario_layer5_outdated_bash_command() {
  local home="$WORK/l5-g2" plugin="$WORK/l5-g2-plugin" bin="$WORK/l5-g2-bin" out
  mkdir -p "$home/.claude" "$home/.config/playbook" "$plugin"
  printf '#!/usr/bin/env bash\necho hi\n' > "$home/.config/playbook/statusline.sh"
  cp "$home/.config/playbook/statusline.sh" "$plugin/statusline.sh"
  write_statusline_settings "$home" 'bash $HOME/.config/playbook/statusline.sh'
  write_stub_binary "$bin" ""
  out="$(run_layer5 "$home" "$bin:/usr/bin:/bin" "$plugin")"
  [[ "$out" == 'OUTDATED bash $HOME/.config/playbook/statusline.sh' ]] || { echo "  got: $out"; return 1; }
}

# G2b: the absolute-home and tilde forms of the old command are OUTDATED too.
scenario_layer5_outdated_other_forms() {
  local home="$WORK/l5-g2b" bin="$WORK/l5-g2b-bin" out cmd
  mkdir -p "$home/.claude"
  write_stub_binary "$bin" ""
  for cmd in "bash $home/.config/playbook/statusline.sh" 'bash ~/.config/playbook/statusline.sh'; do
    write_statusline_settings "$home" "$cmd"
    out="$(run_layer5 "$home" "$bin:/usr/bin:/bin")"
    [[ "$out" == "OUTDATED $cmd" ]] || { echo "  got: $out"; return 1; }
  done
}

# G3: the Rust command is the healthy value.
scenario_layer5_rust() {
  local home="$WORK/l5-g3" bin="$WORK/l5-g3-bin" out
  mkdir -p "$home/.claude"
  write_statusline_settings "$home" 'playbook statusline'
  write_stub_binary "$bin" ""
  out="$(run_layer5 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "RUST" ]] || { echo "  got: $out"; return 1; }
}

# H: playbook ships no copy, so a differing plugin file does not change the result.
scenario_layer5_differs() {
  local home="$WORK/l5-h" plugin="$WORK/l5-h-plugin" bin="$WORK/l5-h-bin" out
  mkdir -p "$home/.claude" "$plugin"
  printf '#!/usr/bin/env bash\necho old\n' > "$home/.claude/statusline.sh"
  printf '#!/usr/bin/env bash\necho new\n' > "$plugin/statusline.sh"
  write_statusline_settings "$home" '$HOME/.claude/statusline.sh'
  write_stub_binary "$bin" ""
  out="$(run_layer5 "$home" "$bin:/usr/bin:/bin" "$plugin")"
  [[ "$out" == 'CUSTOM $HOME/.claude/statusline.sh' ]] || { echo "  got: $out"; return 1; }
}

# I: no statusLine.command at all.
scenario_layer5_not_configured() {
  local home="$WORK/l5-i" bin="$WORK/l5-i-bin" out
  mkdir -p "$home/.claude"
  printf '{}' > "$home/.claude/settings.json"
  write_stub_binary "$bin" ""
  out="$(run_layer5 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "NOT_CONFIGURED" ]] || { echo "  got: $out"; return 1; }
}

# U: playbook absent from PATH entirely -> UNKNOWN, not silently NOT_CONFIGURED.
scenario_layer5_playbook_missing() {
  local home="$WORK/l5-u" out
  write_statusline_settings "$home" '$HOME/.claude/statusline.sh'
  out="$(run_layer5 "$home" "/usr/bin:/bin")"
  [[ "$out" == "UNKNOWN, playbook too old or missing, see Layer 6" ]] || { echo "  got: $out"; return 1; }
}

# V: playbook resolves but predates the `doctor` subcommand -> the same
# UNKNOWN, not silently NOT_CONFIGURED. A different root cause than U, same
# observable outcome, both worth pinning since they exercise different code
# paths (command not found vs. unrecognized subcommand).
scenario_layer5_playbook_too_old() {
  local home="$WORK/l5-v" bin="$WORK/l5-v-bin" out
  write_statusline_settings "$home" '$HOME/.claude/statusline.sh'
  write_stub_binary_without_doctor "$bin" "playbook 0.12.0"
  out="$(run_layer5 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "UNKNOWN, playbook too old or missing, see Layer 6" ]] || { echo "  got: $out"; return 1; }
}

run_scenario "F: statusLine.command path does not exist -> MISSING <path>" scenario_layer5_missing
run_scenario "G: custom command, file present -> CUSTOM"        scenario_layer5_match
run_scenario "G2: old bash command on .config/playbook path -> OUTDATED"   scenario_layer5_outdated_bash_command
run_scenario "G2b: absolute and tilde forms of the old command -> OUTDATED" scenario_layer5_outdated_other_forms
run_scenario "G3: playbook statusline -> RUST"                             scenario_layer5_rust
run_scenario "H: shipped copy differs -> still CUSTOM"           scenario_layer5_differs
run_scenario "I: no statusLine.command at all -> NOT_CONFIGURED"           scenario_layer5_not_configured
run_scenario "U: playbook absent from PATH -> UNKNOWN"                     scenario_layer5_playbook_missing
run_scenario "V: playbook too old for doctor subcommand -> UNKNOWN"        scenario_layer5_playbook_too_old

# ── Layer 6: binary resolves ────────────────────────────────────────────────
#
# `write_stub_binary` and `write_stub_binary_without_doctor` are defined above,
# in the Layer 5 section: both layers need the same stubbed `playbook`.

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
  [[ "$out" == $'MATCH 0.10.0\nGATE_SOURCE=MISSING' ]] || { echo "  got: $out"; return 1; }
}

# L: same stub, manifest reports a different version -> SKEW, both named.
scenario_layer6_skew() {
  local home="$WORK/l6-l" bin="$WORK/l6-l-bin" plugin="$WORK/l6-l-plugin" out
  mkdir -p "$home"
  write_stub_binary "$bin" "playbook 0.10.0"
  write_manifest "$plugin" "0.9.1"
  out="$(run_layer6 "$home" "$bin:/usr/bin:/bin" "$plugin")"
  [[ "$out" == $'SKEW binary=0.10.0 plugin=0.9.1\nGATE_SOURCE=MISSING' ]] || { echo "  got: $out"; return 1; }
}

# M: the stub prints nothing for --version, so the resolved binary is not the
# one this plugin expects (stale shim or a name collision).
scenario_layer6_no_version() {
  local home="$WORK/l6-m" bin="$WORK/l6-m-bin" out
  mkdir -p "$home"
  write_stub_binary "$bin" ""
  out="$(run_layer6 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == $'NO_VERSION\nGATE_SOURCE=MISSING' ]] || { echo "  got: $out"; return 1; }
}

# N: the stub resolves but no manifest can be found to compare against.
scenario_layer6_present_no_baseline() {
  local home="$WORK/l6-n" bin="$WORK/l6-n-bin" out
  mkdir -p "$home"
  write_stub_binary "$bin" "playbook 0.10.0"
  out="$(run_layer6 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == $'PRESENT_NO_BASELINE 0.10.0\nGATE_SOURCE=MISSING' ]] || { echo "  got: $out"; return 1; }
}

# W: the binary resolves and reports a version, but predates the `doctor`
# subcommand, so it cannot be asked for the manifest's version -> TOO_OLD, not
# silently PRESENT_NO_BASELINE (a manifest may well exist here, unlike N).
scenario_layer6_too_old() {
  local home="$WORK/l6-w" bin="$WORK/l6-w-bin" plugin="$WORK/l6-w-plugin" out
  mkdir -p "$home"
  write_stub_binary_without_doctor "$bin" "playbook 0.12.0"
  write_manifest "$plugin" "0.12.0"
  out="$(run_layer6 "$home" "$bin:/usr/bin:/bin" "$plugin")"
  [[ "$out" == $'TOO_OLD 0.12.0\nGATE_SOURCE=MISSING' ]] || { echo "  got: $out"; return 1; }
}

# AA: the stub's `gate record --help` output lists `--source`, simulating a
# binary that already supports gate staleness enforcement -> GATE_SOURCE=OK.
scenario_layer6_gate_source_ok() {
  local home="$WORK/l6-aa" bin="$WORK/l6-aa-bin" plugin="$WORK/l6-aa-plugin" out
  mkdir -p "$home"
  write_stub_binary "$bin" "playbook 0.10.0" \
    "Usage: playbook gate record --source <SOURCE> <PLAN_SLUG> <COMMAND> <PHASE> <INPUT>"
  write_manifest "$plugin" "0.10.0"
  out="$(run_layer6 "$home" "$bin:/usr/bin:/bin" "$plugin")"
  [[ "$out" == $'MATCH 0.10.0\nGATE_SOURCE=OK' ]] || { echo "  got: $out"; return 1; }
}

# AB: the stub's `gate record --help` output has no `--source` at all,
# simulating a binary built before it shipped -> GATE_SOURCE=MISSING,
# independent of an otherwise-matching version (regression pin: this must
# fire even when SKEW itself would report MATCH, per doctor.md's own note
# that the two are independent verdicts).
scenario_layer6_gate_source_missing() {
  local home="$WORK/l6-ab" bin="$WORK/l6-ab-bin" plugin="$WORK/l6-ab-plugin" out
  mkdir -p "$home"
  write_stub_binary "$bin" "playbook 0.10.0"
  write_manifest "$plugin" "0.10.0"
  out="$(run_layer6 "$home" "$bin:/usr/bin:/bin" "$plugin")"
  [[ "$out" == $'MATCH 0.10.0\nGATE_SOURCE=MISSING' ]] || { echo "  got: $out"; return 1; }
}

run_scenario "J: playbook absent from PATH -> MISSING"                         scenario_layer6_missing
run_scenario "K: binary and manifest agree -> MATCH <ver>"                     scenario_layer6_match
run_scenario "L: binary and manifest disagree -> SKEW binary=.. plugin=.."     scenario_layer6_skew
run_scenario "M: --version prints nothing -> NO_VERSION"                       scenario_layer6_no_version
run_scenario "N: binary present, no manifest found -> PRESENT_NO_BASELINE"     scenario_layer6_present_no_baseline
run_scenario "W: binary predates doctor subcommand -> TOO_OLD <ver>"           scenario_layer6_too_old
run_scenario "AA: gate record --help lists --source -> GATE_SOURCE=OK"        scenario_layer6_gate_source_ok
run_scenario "AB: gate record --help has no --source -> GATE_SOURCE=MISSING"  scenario_layer6_gate_source_missing

# ── Layer 7: no hook command points at a missing file ───────────────────────

run_layer7() {
  local home="$1" path="$2"
  HOME="$home" PATH="$path" bash -c "$LAYER7" 2>&1
}

# O: every hook command is the bare `playbook hook <name>` form, across two
# different events. None of them look like a path, so nothing is checked.
scenario_layer7_all_bare_healthy() {
  local home="$WORK/l7-o" bin="$WORK/l7-o-bin" out
  mkdir -p "$home/.claude"
  printf '{"hooks":{"PreToolUse":[{"hooks":[{"command":"playbook hook rm-workspace-guard"}]}],"Stop":[{"hooks":[{"command":"playbook hook memory-capture"}]}]}}' \
    > "$home/.claude/settings.json"
  write_stub_binary "$bin" ""
  out="$(run_layer7 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "status=OK checked=0 dangling=" ]] || { echo "  got: $out"; return 1; }
}

# P: the exact shape the orphaned memory_context.py incident had, a leftover
# Python hook command naming a file that is no longer on disk. Regression pin
# for that incident.
scenario_layer7_dangling_python_hook() {
  local home="$WORK/l7-p" bin="$WORK/l7-p-bin" out
  mkdir -p "$home/.claude"
  printf '{"hooks":{"PreToolUse":[{"hooks":[{"command":"python3 ~/.claude/hooks/memory_context.py"}]}]}}' \
    > "$home/.claude/settings.json"
  write_stub_binary "$bin" ""
  out="$(run_layer7 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "status=OK checked=1 dangling=python3 ~/.claude/hooks/memory_context.py" ]] || { echo "  got: $out"; return 1; }
}

# Q: a legacy `.sh` command left over from before a hook was ported, on a
# machine where the script itself is gone. A second, independent path shape.
scenario_layer7_dangling_legacy_guard() {
  local home="$WORK/l7-q" bin="$WORK/l7-q-bin" out
  mkdir -p "$home/.claude"
  printf '{"hooks":{"Stop":[{"hooks":[{"command":"~/.claude/hooks/retired-guard.sh"}]}]}}' \
    > "$home/.claude/settings.json"
  write_stub_binary "$bin" ""
  out="$(run_layer7 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "status=OK checked=1 dangling=~/.claude/hooks/retired-guard.sh" ]] || { echo "  got: $out"; return 1; }
}

# R: a path-shaped command whose file genuinely exists must not be flagged.
scenario_layer7_existing_path_not_flagged() {
  local home="$WORK/l7-r" bin="$WORK/l7-r-bin" out
  mkdir -p "$home/.claude/hooks"
  touch "$home/.claude/hooks/real.sh"
  printf '{"hooks":{"PreToolUse":[{"hooks":[{"command":"%s/.claude/hooks/real.sh"}]}]}}' "$home" \
    > "$home/.claude/settings.json"
  write_stub_binary "$bin" ""
  out="$(run_layer7 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "status=OK checked=1 dangling=" ]] || { echo "  got: $out"; return 1; }
}

# S: the same dangling command wired under two different events, the real
# session-clean-exit precedent (one hook name legitimately wired on both Stop
# and SessionEnd), must be reported once, not twice, even though both count
# toward `checked`.
scenario_layer7_duplicate_across_events_deduped() {
  local home="$WORK/l7-s" bin="$WORK/l7-s-bin" out
  mkdir -p "$home/.claude"
  printf '{"hooks":{"Stop":[{"hooks":[{"command":"~/.claude/hooks/gone.sh"}]}],"SessionEnd":[{"hooks":[{"command":"~/.claude/hooks/gone.sh"}]}]}}' \
    > "$home/.claude/settings.json"
  write_stub_binary "$bin" ""
  out="$(run_layer7 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "status=OK checked=2 dangling=~/.claude/hooks/gone.sh" ]] || { echo "  got: $out"; return 1; }
}

# T: a command whose path still contains an unresolved environment variable
# after ~/$HOME substitution must be skipped, not falsely flagged as missing.
scenario_layer7_unresolved_var_skipped() {
  local home="$WORK/l7-t" bin="$WORK/l7-t-bin" out
  mkdir -p "$home/.claude"
  printf '{"hooks":{"PreToolUse":[{"hooks":[{"command":"bash $CLAUDE_PLUGIN_ROOT/hooks/foo.sh"}]}]}}' \
    > "$home/.claude/settings.json"
  write_stub_binary "$bin" ""
  out="$(run_layer7 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "status=OK checked=0 dangling=" ]] || { echo "  got: $out"; return 1; }
}

# X: playbook absent from PATH entirely -> status=UNKNOWN, not a silent PASS
# with checked=0 (the exact fail-open shape issue #381 reported: no jq used
# to read the same as zero commands, all healthy).
scenario_layer7_playbook_missing() {
  local home="$WORK/l7-x" out
  mkdir -p "$home/.claude"
  printf '{"hooks":{"PreToolUse":[{"hooks":[{"command":"~/.claude/hooks/gone.sh"}]}]}}' \
    > "$home/.claude/settings.json"
  out="$(run_layer7 "$home" "/usr/bin:/bin")"
  [[ "$out" == "status=UNKNOWN checked=0 dangling=" ]] || { echo "  got: $out"; return 1; }
}

# Y: playbook resolves but predates the `doctor` subcommand entirely (the
# same stub Layer 5/6's own "too old" scenarios, U and W, already share) ->
# status=UNKNOWN, same observable outcome as X, different root cause
# (command not found vs. unrecognized subcommand). Does not separately pin
# the narrower in-between case (a build with `plugin-version`/
# `statusline-command` but not yet `hook-commands`): no existing Layer 5/6
# scenario does either, and the stub would need its own variant to tell
# `doctor hook-commands` apart from `doctor plugin-version`.
scenario_layer7_playbook_too_old() {
  local home="$WORK/l7-y" bin="$WORK/l7-y-bin" out
  mkdir -p "$home/.claude"
  printf '{"hooks":{"PreToolUse":[{"hooks":[{"command":"~/.claude/hooks/gone.sh"}]}]}}' \
    > "$home/.claude/settings.json"
  write_stub_binary_without_doctor "$bin" "playbook 0.12.0"
  out="$(run_layer7 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "status=UNKNOWN checked=0 dangling=" ]] || { echo "  got: $out"; return 1; }
}

# Z: ~/.claude/settings.json itself does not exist -> status=UNKNOWN, not a
# silent PASS with checked=0. `hook-commands` on a missing file returns an
# empty list the same way it would for a legitimately empty `.hooks`, so
# without this guard the layer could not tell "nothing to check" apart from
# "couldn't even find the file to check."
scenario_layer7_settings_json_missing() {
  local home="$WORK/l7-z" bin="$WORK/l7-z-bin" out
  mkdir -p "$home/.claude" "$bin"
  write_stub_binary "$bin" ""
  out="$(run_layer7 "$home" "$bin:/usr/bin:/bin")"
  [[ "$out" == "status=UNKNOWN checked=0 dangling=" ]] || { echo "  got: $out"; return 1; }
}

run_scenario "O: every command is the bare form -> nothing checked, nothing dangling" scenario_layer7_all_bare_healthy
run_scenario "P: leftover Python hook command, file gone -> dangling (memory_context.py regression pin)" scenario_layer7_dangling_python_hook
run_scenario "Q: leftover legacy .sh guard command, file gone -> dangling"     scenario_layer7_dangling_legacy_guard
run_scenario "R: path-shaped command whose file exists -> not flagged"        scenario_layer7_existing_path_not_flagged
run_scenario "S: same dangling command on two events -> reported once"        scenario_layer7_duplicate_across_events_deduped
run_scenario "T: unresolved \$VAR in path -> skipped, not flagged"            scenario_layer7_unresolved_var_skipped
run_scenario "X: playbook absent from PATH -> status=UNKNOWN, not a silent PASS" scenario_layer7_playbook_missing
run_scenario "Y: playbook too old for hook-commands subcommand -> status=UNKNOWN" scenario_layer7_playbook_too_old
run_scenario "Z: ~/.claude/settings.json missing -> status=UNKNOWN, not a silent PASS" scenario_layer7_settings_json_missing

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
