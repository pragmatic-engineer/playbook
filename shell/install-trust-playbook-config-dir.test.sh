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
# install.sh prefers `playbook trust <dir>` from the installed binary and
# falls back to python3 only when that binary predates the subcommand. A stub
# `playbook` on a controlled PATH stands in for each kind of installed
# binary (see write_stub_playbook): one that supports `trust` and records
# every call to a shared file, so the suite can prove the exact argument and
# call count, one that predates it, and one whose `trust` reports a failure.
# build_minimal_path_dir pins whether python3 is visible, so a scenario never
# depends on what the host happens to have installed.
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

command -v python3 >/dev/null 2>&1 || { echo "python3 not found on PATH (needed for the python3 fallback scenarios)" >&2; exit 2; }
command -v cargo >/dev/null 2>&1 || { echo "cargo not found on PATH" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

BIN_SRC="${REPO_ROOT}/target/debug/playbook"
if [ ! -x "$BIN_SRC" ]; then
  echo "Building playbook (cargo build)..."
  ( cd "$REPO_ROOT" && cargo build --quiet ) || { echo "cargo build failed" >&2; exit 2; }
fi
PLAYBOOK="$BIN_SRC"

seed_shipped_extras() {
  local src="$1"
  printf '#!/bin/sh\necho ok\n' > "$src/statusline.sh"
  mkdir -p "$src/shell/bash" "$src/shell/zsh" "$src/shell/shared"
  printf '#!/bin/sh\n' > "$src/shell/bash/cc.sh"
  printf '#!/bin/sh\n' > "$src/shell/zsh/cc.zsh"
}

# A stub `playbook` binary: install.sh execs it directly by absolute path
# ($PLAYBOOK_BIN_DIR/playbook). It answers `--version` and `init` like the
# earlier suites, and `trust` according to <trust_mode>:
#   ok   supports `trust`: appends "trust <args>" to <record>, then hands the
#        real work to the freshly built playbook so the file really changes
#   old  a binary that predates the subcommand: any `trust` call is rejected
#        with exit status 2, the way clap rejects an unknown subcommand
#   fail supports `trust` but reports a failure on stderr and exits 0, the
#        way the real subcommand does
write_stub_playbook() {
  local bindir="$1" trust_mode="$2" record="$3"
  mkdir -p "$bindir"
  cat > "$bindir/playbook" <<STUB
#!/usr/bin/env bash
case "\${1:-}" in
  --version) printf 'playbook 0.15.0\n' ;;
  init)
    mkdir -p "\$CLAUDE_HOME"
    printf '{}' > "\$CLAUDE_HOME/settings.json"
    if [[ "\$*" == *--help* ]]; then
      printf -- '--aliases --system-prompt\n'
    fi
    ;;
  trust)
    case "$trust_mode" in
      old)
        echo "error: unrecognized subcommand 'trust'" >&2
        exit 2
        ;;
      fail)
        [ "\${2:-}" = "--help" ] && exit 0
        printf 'trust %s\n' "\${*:2}" >> "$record"
        echo "playbook trust: simulated write failure" >&2
        exit 0
        ;;
      ok)
        [ "\${2:-}" = "--help" ] && exit 0
        printf 'trust %s\n' "\${*:2}" >> "$record"
        exec "$PLAYBOOK" trust "\${@:2}"
        ;;
    esac
    ;;
esac
exit 0
STUB
  chmod +x "$bindir/playbook"
}

# Populates $dir with symlinks to the exact core tools install.sh's own
# preflight needs, resolved from wherever they REALLY live on this host (via
# `command -v` against the unrestricted PATH this test suite itself runs
# under) before PATH gets restricted for the child install.sh process.
# install_python3 ("1" or "0") controls whether python3 is included, so a
# scenario can pin exactly whether install.sh sees it as available.
build_minimal_path_dir() {
  local dir="$1" install_python3="$2"
  mkdir -p "$dir"
  local always=(bash curl tar shasum sha256sum mktemp mv chmod rm cat grep sed awk basename dirname mkdir cp find date uname sort tail)
  local tool real
  for tool in "${always[@]}"; do
    real="$(command -v "$tool" 2>/dev/null)" || continue
    ln -sf "$real" "$dir/$tool"
  done
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
# stripped child environment). Checked in priority order: the well-known
# absolute system locations first (never a version-manager shim),
# `command -v` only as the last resort, and every candidate is verified under
# `env -i PATH=/usr/bin:/bin` specifically so a shim that depends on
# inherited PATH/HOME state to dispatch fails this check too.
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
# whether python3 resolves is controlled entirely by what
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

# new_case <tag> <trust_mode> <python3 flag>: builds one scratch install and
# sets src, home, bindir, log, tools, record for the caller.
new_case() {
  local tag="$1" trust_mode="$2" python3_flag="$3" d
  d="$(mktemp -d "$WORK/$tag.XXXXXX")"
  src="$d/src"; home="$d/home"; bindir="$d/bin"; tools="$d/tools"
  log="$d/install.log"; record="$d/trust.record"
  mkdir -p "$src" "$home" "$bindir"
  : > "$record"
  seed_shipped_extras "$src"
  write_stub_playbook "$bindir" "$trust_mode" "$record"
  build_minimal_path_dir "$tools" "$python3_flag"
}

# (A) ~/.claude.json does not exist at all (a genuinely fresh machine that
# has never run claude): the step must no-op cleanly, not create the file
# itself, not call `trust`, and not fail the installer.
scenario_no_claude_json_is_a_clean_noop() {
  local src home bindir log tools record
  new_case noclaudejson ok 1

  run_install "$src" "$home" "$bindir" "$log" "$tools"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  [ ! -f "$home/.claude.json" ] || { echo "  ~/.claude.json was created when it should have been left absent: $(cat "$home/.claude.json")"; return 1; }
  [ ! -s "$record" ] || { echo "  trust was called with no ~/.claude.json present: $(cat "$record")"; return 1; }
}

# (B) A binary that supports `trust`, on a host with NO python3 at all, and a
# bare {} ~/.claude.json: install.sh must call `trust` exactly once with
# exactly the config dir, and the entry must be created. Proves the trust
# path needs no other JSON tool.
scenario_trust_subcommand_is_called_with_the_config_dir() {
  local src home bindir log tools record
  new_case trustcall ok 0
  printf '{}' > "$home/.claude.json"

  run_install "$src" "$home" "$bindir" "$log" "$tools"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  [ "$(cat "$record")" = "trust $home/.config/playbook" ] || { echo "  trust was not called exactly once with the config dir: '$(cat "$record")'"; return 1; }
  local got
  got=$("$PLAYBOOK" json project-field "$home/.config/playbook" hasTrustDialogAccepted \
    < "$home/.claude.json" 2>/dev/null)
  [ "$got" = "true" ] || { echo "  hasTrustDialogAccepted not set to true: got '$got', file: $(cat "$home/.claude.json")"; return 1; }
}

# (C) ~/.claude.json already has an entry for the config dir with OTHER
# fields set and hasTrustDialogAccepted explicitly false: the entry must
# flip to true WITHOUT clobbering the sibling fields, and an unrelated
# project's entry must stay untouched.
scenario_existing_entry_is_updated_not_replaced() {
  local src home bindir log tools record
  new_case existing ok 0
  printf '{"projects":{"%s":{"hasTrustDialogAccepted":false,"lastCost":1.23},"%s":{"hasTrustDialogAccepted":false}}}' \
    "$home/.config/playbook" "$home/some/other/project" > "$home/.claude.json"

  run_install "$src" "$home" "$bindir" "$log" "$tools"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }

  local trusted sibling other_untouched
  trusted=$("$PLAYBOOK" json project-field "$home/.config/playbook" hasTrustDialogAccepted < "$home/.claude.json")
  sibling=$("$PLAYBOOK" json project-field "$home/.config/playbook" lastCost < "$home/.claude.json")
  other_untouched=$("$PLAYBOOK" json project-field "$home/some/other/project" hasTrustDialogAccepted < "$home/.claude.json")

  [ "$trusted" = "true" ] || { echo "  existing false entry was not flipped to true: $(cat "$home/.claude.json")"; return 1; }
  [ "$sibling" = "1.23" ] || { echo "  sibling field lastCost was clobbered: $(cat "$home/.claude.json")"; return 1; }
  [ "$other_untouched" = "false" ] || { echo "  an unrelated project's trust entry was touched: $(cat "$home/.claude.json")"; return 1; }
}

# (D) A binary that predates `trust` (it rejects the subcommand), python3
# available: install.sh must fall through to the python3 path and still
# create the entry, and must not have recorded a successful trust call.
scenario_older_binary_falls_through_to_python3() {
  local src home bindir log tools record
  new_case oldbinary old 1
  printf '{}' > "$home/.claude.json"

  run_install "$src" "$home" "$bindir" "$log" "$tools"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  [ ! -s "$record" ] || { echo "  an older binary should never record a trust call: $(cat "$record")"; return 1; }
  local got
  got=$("$PLAYBOOK" json project-field "$home/.config/playbook" hasTrustDialogAccepted \
    < "$home/.claude.json" 2>/dev/null)
  [ "$got" = "true" ] || { echo "  python3 fallback did not set the entry: got '$got', file: $(cat "$home/.claude.json"), log: $(cat "$log")"; return 1; }
}

# (E) An older binary AND no python3: nothing can do the edit. The installer
# must still complete (a best-effort step never fails the install), must
# warn, and must not touch ~/.claude.json.
scenario_no_usable_tool_warns_but_does_not_fail() {
  local src home bindir log tools record
  new_case notool old 0
  printf '{}' > "$home/.claude.json"

  run_install "$src" "$home" "$bindir" "$log" "$tools"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc (a best-effort step must not fail the installer): $(cat "$log")"; return 1; }
  grep -qi "could not update.*trust" "$log" || { echo "  no warning was printed about the skipped trust update: $(cat "$log")"; return 1; }
  local unchanged
  unchanged=$(cat "$home/.claude.json")
  [ "$unchanged" = "{}" ] || { echo "  ~/.claude.json was modified with no usable tool: $unchanged"; return 1; }
}

# (F) An older binary and a python3 that resolves but always fails (a corrupt
# install, say): the installer must still complete, must warn, and must leave
# ~/.claude.json untouched.
scenario_failing_python3_warns_but_does_not_fail() {
  local src home bindir log tools record
  new_case brokenpython3 old 0
  printf '#!/bin/sh\nexit 1\n' > "$tools/python3"
  chmod +x "$tools/python3"
  printf '{}' > "$home/.claude.json"

  run_install "$src" "$home" "$bindir" "$log" "$tools"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc (a failing python3 must not fail the installer): $(cat "$log")"; return 1; }
  grep -qi "could not update.*trust" "$log" || { echo "  no warning was printed about the failed trust update: $(cat "$log")"; return 1; }
  local unchanged
  unchanged=$(cat "$home/.claude.json")
  [ "$unchanged" = "{}" ] || { echo "  ~/.claude.json was modified by a failing python3: $unchanged"; return 1; }
}

# (G) A binary that supports `trust` but whose call reports a failure on
# stderr (the real subcommand always exits 0 and says so on stderr): the
# installer must complete, surface that message in a warning, and leave
# ~/.claude.json untouched. python3 is deliberately available to prove it is
# NOT consulted once the subcommand exists.
scenario_trust_failure_is_surfaced_as_a_warning() {
  local src home bindir log tools record
  new_case trustfail fail 1
  printf '{}' > "$home/.claude.json"

  run_install "$src" "$home" "$bindir" "$log" "$tools"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  grep -q "simulated write failure" "$log" || { echo "  the trust failure message was not surfaced: $(cat "$log")"; return 1; }
  local unchanged
  unchanged=$(cat "$home/.claude.json")
  [ "$unchanged" = "{}" ] || { echo "  python3 ran after a supported trust reported failure: $unchanged"; return 1; }
}

run_scenario "A: no ~/.claude.json at all -> clean no-op, file stays absent, trust not called" scenario_no_claude_json_is_a_clean_noop
run_scenario "B: trust-capable binary, no python3 -> trust called once with the config dir, entry created" scenario_trust_subcommand_is_called_with_the_config_dir
run_scenario "C: an existing false entry is flipped without clobbering siblings or other projects" scenario_existing_entry_is_updated_not_replaced
run_scenario "D: binary without trust falls through to python3" scenario_older_binary_falls_through_to_python3
run_scenario "E: no trust and no python3 -> warns, does not fail, does not touch the file" scenario_no_usable_tool_warns_but_does_not_fail
run_scenario "F: no trust and a failing python3 -> warns, does not fail, does not touch the file" scenario_failing_python3_warns_but_does_not_fail
run_scenario "G: trust reports a failure -> warns with its message, python3 not consulted" scenario_trust_failure_is_surfaced_as_a_warning

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
