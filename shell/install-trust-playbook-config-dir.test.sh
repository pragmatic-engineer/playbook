#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# install-trust-playbook-config-dir.test.sh: install.sh must mark
# $HOME/.config/playbook as a trusted workspace in ~/.claude.json once it
# exists, so a `claude` session started directly in that folder (to inspect
# or hand-edit statusline.sh, say) does not hit Claude Code's workspace
# trust dialog and silently skip statusLine execution. Regression pin for
# the incident where an untrusted project folder produced exactly that
# failure, logged via `claude --debug` as "Status line command skipped:
# workspace trust not accepted".
#
# Uses the same PLAYBOOK_SRC local-source seam as
# shell/install-claude-version-gate.test.sh, with --skip-plugin so no
# `claude` stub is needed: this suite only cares about the
# trust_playbook_config_dir step, which runs unconditionally regardless of
# whether the plugin section runs.
#
# Run:  bash shell/install-trust-playbook-config-dir.test.sh
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SCRIPT_DIR}/.."
INSTALL="${REPO_ROOT}/install.sh"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1${2:+ -- $2}"; (( FAIL++ )) || true; }

command -v jq >/dev/null 2>&1 || { echo "jq not found on PATH (needed to assert on the result, not just to run install.sh)" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

seed_shipped_extras() {
  local src="$1"
  printf '#!/bin/sh\necho ok\n' > "$src/statusline.sh"
  mkdir -p "$src/shell/bash" "$src/shell/zsh" "$src/shell/shared"
  printf '#!/bin/sh\n' > "$src/shell/bash/cc.sh"
  printf '#!/bin/sh\n' > "$src/shell/zsh/cc.zsh"
}

# A stub `playbook` binary on PATH: install.sh execs it directly by absolute
# path ($PLAYBOOK_BIN_DIR/playbook), so this stands in for the real release
# binary without needing a cargo build. Handles exactly the two subcommands
# install.sh calls: `init` (writes a minimal settings.json so the rest of
# the script has something to report on) and `--version`/`init --help`
# (used by the --aliases/--system-prompt probes, both unused here since this
# suite passes neither flag).
write_stub_playbook() {
  local bindir="$1"
  mkdir -p "$bindir"
  cat > "$bindir/playbook" <<'STUB'
#!/usr/bin/env bash
case "${1:-}" in
  --version) printf 'playbook 0.15.0\n' ;;
  init)
    mkdir -p "$CLAUDE_HOME"
    printf '{}' > "$CLAUDE_HOME/settings.json"
    if [[ "$*" == *--help* ]]; then
      printf -- '--aliases --system-prompt\n'
    fi
    ;;
esac
exit 0
STUB
  chmod +x "$bindir/playbook"
}

# run_install <src> <home> <bindir> <log>: runs the real installer against a
# scratch $HOME, skipping the plugin section (no claude stub needed) since
# trust_playbook_config_dir runs unconditionally regardless of --skip-plugin.
run_install() {
  local src="$1" home="$2" bindir="$3" log="$4"
  env -u SHELL PLAYBOOK_SRC="$src" PLAYBOOK_BIN_DIR="$bindir" \
    CLAUDE_HOME="$home/.claude" HOME="$home" PATH="$bindir:/usr/bin:/bin" \
    bash "$INSTALL" --yes --skip-plugin >"$log" 2>&1
}

run_scenario() {
  local name="$1" fn="$2"
  if "$fn"; then pass "$name"; else fail "$name"; fi
}

# (A) ~/.claude.json does not exist at all (a genuinely fresh machine that
# has never run claude): the step must no-op cleanly, not create the file
# itself, and not fail the installer.
scenario_no_claude_json_is_a_clean_noop() {
  local d src home bindir log
  d="$(mktemp -d "$WORK/noclaudejson.XXXXXX")"
  src="$d/src"; home="$d/home"; bindir="$d/bin"
  mkdir -p "$src" "$home" "$bindir"
  seed_shipped_extras "$src"
  write_stub_playbook "$bindir"
  log="$d/install.log"

  run_install "$src" "$home" "$bindir" "$log"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  [ ! -f "$home/.claude.json" ] || { echo "  ~/.claude.json was created when it should have been left absent: $(cat "$home/.claude.json")"; return 1; }
}

# (B) ~/.claude.json exists but is a bare {}: the project entry for
# $HOME/.config/playbook must be created with hasTrustDialogAccepted true.
scenario_bare_claude_json_gets_the_trust_entry() {
  local d src home bindir log
  d="$(mktemp -d "$WORK/bare.XXXXXX")"
  src="$d/src"; home="$d/home"; bindir="$d/bin"
  mkdir -p "$src" "$home" "$bindir"
  seed_shipped_extras "$src"
  write_stub_playbook "$bindir"
  printf '{}' > "$home/.claude.json"
  log="$d/install.log"

  run_install "$src" "$home" "$bindir" "$log"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  local got
  got=$(jq -r --arg p "$home/.config/playbook" '.projects[$p].hasTrustDialogAccepted' "$home/.claude.json" 2>/dev/null)
  [ "$got" = "true" ] || { echo "  hasTrustDialogAccepted not set to true: got '$got', file: $(cat "$home/.claude.json")"; return 1; }
}

# (C) ~/.claude.json already has an entry for the config dir with OTHER
# fields set (e.g. from a previous claude session started there) and
# hasTrustDialogAccepted explicitly false: the fix must flip it to true
# WITHOUT clobbering the sibling fields, and must not touch an unrelated
# project's entry.
scenario_existing_entry_is_updated_not_replaced() {
  local d src home bindir log
  d="$(mktemp -d "$WORK/existing.XXXXXX")"
  src="$d/src"; home="$d/home"; bindir="$d/bin"
  mkdir -p "$src" "$home" "$bindir"
  seed_shipped_extras "$src"
  write_stub_playbook "$bindir"
  jq -n --arg cfg "$home/.config/playbook" --arg other "$home/some/other/project" '
    {
      projects: {
        ($cfg): {hasTrustDialogAccepted: false, lastCost: 1.23},
        ($other): {hasTrustDialogAccepted: false}
      }
    }
  ' > "$home/.claude.json"
  log="$d/install.log"

  run_install "$src" "$home" "$bindir" "$log"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }

  local trusted sibling other_untouched
  trusted=$(jq -r --arg p "$home/.config/playbook" '.projects[$p].hasTrustDialogAccepted' "$home/.claude.json")
  sibling=$(jq -r --arg p "$home/.config/playbook" '.projects[$p].lastCost' "$home/.claude.json")
  other_untouched=$(jq -r --arg p "$home/some/other/project" '.projects[$p].hasTrustDialogAccepted' "$home/.claude.json")

  [ "$trusted" = "true" ] || { echo "  existing false entry was not flipped to true: $(cat "$home/.claude.json")"; return 1; }
  [ "$sibling" = "1.23" ] || { echo "  sibling field lastCost was clobbered: $(cat "$home/.claude.json")"; return 1; }
  [ "$other_untouched" = "false" ] || { echo "  an unrelated project's trust entry was touched: $(cat "$home/.claude.json")"; return 1; }
}

# (D) Neither jq nor python3 on PATH: the installer must still complete
# (this is a best-effort convenience step, never a hard requirement) and
# must warn rather than fail silently or crash.
scenario_no_json_tool_warns_but_does_not_fail() {
  local d src home bindir log
  d="$(mktemp -d "$WORK/nojsontool.XXXXXX")"
  src="$d/src"; home="$d/home"; bindir="$d/bin"
  mkdir -p "$src" "$home" "$bindir"
  seed_shipped_extras "$src"
  write_stub_playbook "$bindir"
  printf '{}' > "$home/.claude.json"
  log="$d/install.log"

  # A minimal PATH with only the stub playbook and the shell builtins this
  # script's own preflight needs (curl/tar/shasum), deliberately excluding
  # jq and python3. Since the real system almost certainly has both, name
  # them explicitly as absent by shadowing them ahead of the real ones with
  # scripts that exit 127 (command not found), rather than trying to strip
  # a minimal PATH down to nothing usable.
  local shadow="$d/shadow"
  mkdir -p "$shadow"
  for absent in jq python3; do
    printf '#!/bin/sh\nexit 127\n' > "$shadow/$absent"
    chmod +x "$shadow/$absent"
  done

  env -u SHELL PLAYBOOK_SRC="$src" PLAYBOOK_BIN_DIR="$bindir" \
    CLAUDE_HOME="$home/.claude" HOME="$home" \
    PATH="$shadow:$bindir:/usr/bin:/bin" \
    bash "$INSTALL" --yes --skip-plugin >"$log" 2>&1
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc (a best-effort step must not fail the installer): $(cat "$log")"; return 1; }
  grep -qi "could not update.*trust" "$log" || { echo "  no warning was printed about the skipped trust update: $(cat "$log")"; return 1; }
  local unchanged
  unchanged=$(cat "$home/.claude.json")
  [ "$unchanged" = "{}" ] || { echo "  ~/.claude.json was modified despite no JSON tool being available: $unchanged"; return 1; }
}

run_scenario "A: no ~/.claude.json at all -> clean no-op, file stays absent" scenario_no_claude_json_is_a_clean_noop
run_scenario "B: bare {} gets the trust entry created" scenario_bare_claude_json_gets_the_trust_entry
run_scenario "C: an existing false entry is flipped to true without clobbering siblings or other projects" scenario_existing_entry_is_updated_not_replaced
run_scenario "D: neither jq nor python3 available -> warns, does not fail, does not touch the file" scenario_no_json_tool_warns_but_does_not_fail

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
