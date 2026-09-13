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
# Scenarios B and C each run twice, once with only jq on PATH and once with
# only python3, via build_minimal_path_dir below: which of the two branches
# actually runs must not depend on which JSON tool happens to be installed
# on the machine running this suite (macOS ships no /usr/bin/jq; Linux CI
# images vary on python3), so both are exercised explicitly rather than
# whichever one the host happens to resolve first.
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
command -v python3 >/dev/null 2>&1 || { echo "python3 not found on PATH (needed for the python3-only scenarios)" >&2; exit 2; }

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

# Populates $dir with symlinks to the exact core tools install.sh's own
# preflight and the trust step need, resolved from wherever they REALLY
# live on this host (via `command -v` against the unrestricted PATH this
# test suite itself runs under) before PATH gets restricted for the child
# install.sh process. install_jq / install_python3 ("1" or "0") control
# whether those two specifically get included, so a scenario can pin exactly
# which JSON tool (if any) install.sh sees as available, regardless of
# where the real host happens to keep them (macOS ships no /usr/bin/jq;
# Homebrew's copy lives outside a bare /usr/bin:/bin PATH).
build_minimal_path_dir() {
  local dir="$1" install_jq="$2" install_python3="$3"
  mkdir -p "$dir"
  local always=(bash curl tar shasum sha256sum mktemp mv chmod rm cat grep sed awk basename dirname mkdir cp find date uname sort tail)
  local tool real
  for tool in "${always[@]}"; do
    real="$(command -v "$tool" 2>/dev/null)" || continue
    ln -sf "$real" "$dir/$tool"
  done
  if [ "$install_jq" = "1" ]; then
    real="$(resolve_working_binary jq --version)" && ln -sf "$real" "$dir/jq"
  fi
  if [ "$install_python3" = "1" ]; then
    real="$(resolve_working_binary python3 -c 'pass')" && ln -sf "$real" "$dir/python3"
  fi
}

# Resolves a real, WORKING binary for `tool`, verified by actually running
# it with "$@" under a bare, isolated PATH/HOME (not just found via
# `command -v` under the caller's own full environment, which can point at
# a version-manager shim, e.g. mise or asdf, that resolves on disk and
# happily runs when it can reach the manager binary and the caller's real
# HOME's config, but silently fails once dropped into install.sh's own
# stripped child environment; observed firsthand with mise's python3 shim on
# the machine this suite was authored on, where the same shim passed
# verification under the full environment and then failed with "No version
# is set for shim" once actually invoked under the scratch HOME). Checked in
# priority order: the well-known absolute system locations first (never a
# version-manager shim), `command -v` only as the last resort, and every
# candidate is verified under `env -i PATH=/usr/bin:/bin` specifically so a
# shim that depends on inherited PATH/HOME state to dispatch fails this
# check too, the same way it would fail for real once install.sh runs it
# under a HOME it was never configured for.
resolve_working_binary() {
  local tool="$1"; shift
  local c
  for c in "/usr/bin/$tool" "/usr/local/bin/$tool" "/opt/homebrew/bin/$tool" "/bin/$tool" "/sbin/$tool" "$(command -v "$tool" 2>/dev/null)"; do
    [ -n "$c" ] && [ -x "$c" ] || continue
    if env -i PATH=/usr/bin:/bin "$c" "$@" >/dev/null 2>&1; then
      printf '%s' "$c"
      return 0
    fi
  done
  return 1
}

# run_install <src> <home> <bindir> <log> <toolsdir>: runs the real
# installer against a scratch $HOME, skipping the plugin section (no claude
# stub needed) since trust_playbook_config_dir runs unconditionally
# regardless of --skip-plugin. PATH is exactly <bindir>:<toolsdir>, so
# whether jq/python3 resolve is controlled entirely by what
# build_minimal_path_dir put in <toolsdir>.
run_install() {
  local src="$1" home="$2" bindir="$3" log="$4" toolsdir="$5"
  env -u SHELL PLAYBOOK_SRC="$src" PLAYBOOK_BIN_DIR="$bindir" \
    CLAUDE_HOME="$home/.claude" HOME="$home" PATH="$bindir:$toolsdir" \
    bash "$INSTALL" --yes --skip-plugin >"$log" 2>&1
}

run_scenario() {
  local name="$1" fn="$2"
  shift 2
  if "$fn" "$@"; then pass "$name"; else fail "$name"; fi
}

# (A) ~/.claude.json does not exist at all (a genuinely fresh machine that
# has never run claude): the step must no-op cleanly, not create the file
# itself, and not fail the installer.
scenario_no_claude_json_is_a_clean_noop() {
  local d src home bindir log tools
  d="$(mktemp -d "$WORK/noclaudejson.XXXXXX")"
  src="$d/src"; home="$d/home"; bindir="$d/bin"; tools="$d/tools"
  mkdir -p "$src" "$home" "$bindir"
  seed_shipped_extras "$src"
  write_stub_playbook "$bindir"
  build_minimal_path_dir "$tools" 1 1
  log="$d/install.log"

  run_install "$src" "$home" "$bindir" "$log" "$tools"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  [ ! -f "$home/.claude.json" ] || { echo "  ~/.claude.json was created when it should have been left absent: $(cat "$home/.claude.json")"; return 1; }
}

# (B) ~/.claude.json exists but is a bare {}: the project entry for
# $HOME/.config/playbook must be created with hasTrustDialogAccepted true.
# Takes which tool(s) to expose on PATH, so it can be run once pinned to
# jq-only and once to python3-only.
scenario_bare_claude_json_gets_the_trust_entry() {
  local install_jq="$1" install_python3="$2"
  local d src home bindir log tools
  d="$(mktemp -d "$WORK/bare.XXXXXX")"
  src="$d/src"; home="$d/home"; bindir="$d/bin"; tools="$d/tools"
  mkdir -p "$src" "$home" "$bindir"
  seed_shipped_extras "$src"
  write_stub_playbook "$bindir"
  build_minimal_path_dir "$tools" "$install_jq" "$install_python3"
  printf '{}' > "$home/.claude.json"
  log="$d/install.log"

  run_install "$src" "$home" "$bindir" "$log" "$tools"
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
# project's entry. Same jq-only / python3-only parametrization as (B).
scenario_existing_entry_is_updated_not_replaced() {
  local install_jq="$1" install_python3="$2"
  local d src home bindir log tools
  d="$(mktemp -d "$WORK/existing.XXXXXX")"
  src="$d/src"; home="$d/home"; bindir="$d/bin"; tools="$d/tools"
  mkdir -p "$src" "$home" "$bindir"
  seed_shipped_extras "$src"
  write_stub_playbook "$bindir"
  build_minimal_path_dir "$tools" "$install_jq" "$install_python3"
  jq -n --arg cfg "$home/.config/playbook" --arg other "$home/some/other/project" '
    {
      projects: {
        ($cfg): {hasTrustDialogAccepted: false, lastCost: 1.23},
        ($other): {hasTrustDialogAccepted: false}
      }
    }
  ' > "$home/.claude.json"
  log="$d/install.log"

  run_install "$src" "$home" "$bindir" "$log" "$tools"
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

# (D) Neither jq nor python3 resolves on PATH at all (build_minimal_path_dir
# with both flags "0", so neither is even symlinked in, not merely shadowed
# by a script that still answers to `command -v`): the installer must still
# complete (this is a best-effort convenience step, never a hard
# requirement), must warn rather than fail silently or crash, and must not
# touch ~/.claude.json.
scenario_no_json_tool_warns_but_does_not_fail() {
  local d src home bindir log tools
  d="$(mktemp -d "$WORK/nojsontool.XXXXXX")"
  src="$d/src"; home="$d/home"; bindir="$d/bin"; tools="$d/tools"
  mkdir -p "$src" "$home" "$bindir"
  seed_shipped_extras "$src"
  write_stub_playbook "$bindir"
  build_minimal_path_dir "$tools" 0 0
  printf '{}' > "$home/.claude.json"
  log="$d/install.log"

  run_install "$src" "$home" "$bindir" "$log" "$tools"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc (a best-effort step must not fail the installer): $(cat "$log")"; return 1; }
  grep -qi "could not update.*trust" "$log" || { echo "  no warning was printed about the skipped trust update: $(cat "$log")"; return 1; }
  local unchanged
  unchanged=$(cat "$home/.claude.json")
  [ "$unchanged" = "{}" ] || { echo "  ~/.claude.json was modified despite no JSON tool being available: $unchanged"; return 1; }
}

# (E) jq resolves on PATH but is broken (a corrupt install, say) and python3
# IS available: the fallback must actually run rather than the installer
# reporting "neither tool usable" while a working python3 sat right there
# unused. Regression pin for a real bug: the original code used `elif`
# between the jq and python3 branches, so a present-but-failing jq never
# fell through to python3 at all.
scenario_broken_jq_falls_through_to_python3() {
  local d src home bindir log tools
  d="$(mktemp -d "$WORK/brokenjq.XXXXXX")"
  src="$d/src"; home="$d/home"; bindir="$d/bin"; tools="$d/tools"
  mkdir -p "$src" "$home" "$bindir"
  seed_shipped_extras "$src"
  write_stub_playbook "$bindir"
  build_minimal_path_dir "$tools" 0 1
  # A `jq` that resolves (command -v succeeds) but always fails, standing in
  # for a corrupt or misconfigured install rather than a genuinely absent one.
  printf '#!/bin/sh\nexit 1\n' > "$tools/jq"
  chmod +x "$tools/jq"
  printf '{}' > "$home/.claude.json"
  log="$d/install.log"

  run_install "$src" "$home" "$bindir" "$log" "$tools"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  local got
  got=$(jq -r --arg p "$home/.config/playbook" '.projects[$p].hasTrustDialogAccepted' "$home/.claude.json" 2>/dev/null)
  [ "$got" = "true" ] || { echo "  python3 fallback did not run despite jq failing: got '$got', file: $(cat "$home/.claude.json"), log: $(cat "$log")"; return 1; }
}

run_scenario "A: no ~/.claude.json at all -> clean no-op, file stays absent" scenario_no_claude_json_is_a_clean_noop
run_scenario "B(jq): bare {} gets the trust entry created via jq" scenario_bare_claude_json_gets_the_trust_entry 1 0
run_scenario "B(python3): bare {} gets the trust entry created via python3" scenario_bare_claude_json_gets_the_trust_entry 0 1
run_scenario "C(jq): an existing false entry is flipped to true without clobbering siblings or other projects, via jq" scenario_existing_entry_is_updated_not_replaced 1 0
run_scenario "C(python3): same, via python3" scenario_existing_entry_is_updated_not_replaced 0 1
run_scenario "D: neither jq nor python3 resolvable -> warns, does not fail, does not touch the file" scenario_no_json_tool_warns_but_does_not_fail
run_scenario "E: jq present but failing falls through to python3, not just a warning" scenario_broken_jq_falls_through_to_python3

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
