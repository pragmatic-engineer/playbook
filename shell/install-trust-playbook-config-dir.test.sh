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
# install.sh runs `playbook trust <dir>` from the installed binary and nothing
# else: no other JSON tool is consulted. A stub `playbook` on a controlled PATH
# stands in for each kind of installed binary (see write_stub_playbook): one
# that supports `trust` and records every call to a shared file, so the suite
# can prove the exact argument and call count, one that predates it, and one
# whose `trust` reports a failure.
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
  --version) printf 'playbook 0.16.0\n' ;;
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
build_minimal_path_dir() {
  local dir="$1"
  mkdir -p "$dir"
  local always=(bash curl tar shasum sha256sum mktemp mv chmod rm cat grep sed awk basename dirname mkdir cp find date uname sort tail)
  local tool real
  for tool in "${always[@]}"; do
    real="$(command -v "$tool" 2>/dev/null)" || continue
    ln -sf "$real" "$dir/$tool"
  done
}

# run_install <src> <home> <bindir> <log> <toolsdir>: runs the real
# installer against a scratch $HOME, skipping the plugin section (no claude
# stub needed) since trust_playbook_config_dir runs unconditionally
# regardless of --skip-plugin. PATH is exactly <bindir>:<toolsdir>, so no
# JSON tool other than the stub binary is ever on it.
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

# new_case <tag> <trust_mode>: builds one scratch install and sets src, home,
# bindir, log, tools, record for the caller.
new_case() {
  local tag="$1" trust_mode="$2" d
  d="$(mktemp -d "$WORK/$tag.XXXXXX")"
  src="$d/src"; home="$d/home"; bindir="$d/bin"; tools="$d/tools"
  log="$d/install.log"; record="$d/trust.record"
  mkdir -p "$src" "$home" "$bindir"
  : > "$record"
  seed_shipped_extras "$src"
  write_stub_playbook "$bindir" "$trust_mode" "$record"
  build_minimal_path_dir "$tools"
}

# (A) ~/.claude.json does not exist at all (a genuinely fresh machine that
# has never run claude): the step must no-op cleanly, not create the file
# itself, not call `trust`, and not fail the installer.
scenario_no_claude_json_is_a_clean_noop() {
  local src home bindir log tools record
  new_case noclaudejson ok

  run_install "$src" "$home" "$bindir" "$log" "$tools"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  [ ! -f "$home/.claude.json" ] || { echo "  ~/.claude.json was created when it should have been left absent: $(cat "$home/.claude.json")"; return 1; }
  [ ! -s "$record" ] || { echo "  trust was called with no ~/.claude.json present: $(cat "$record")"; return 1; }
}

# (B) A binary that supports `trust` and a bare {} ~/.claude.json: install.sh
# must call `trust` exactly once with exactly the config dir, and the entry
# must be created. Proves the trust path needs no other tool on PATH.
scenario_trust_subcommand_is_called_with_the_config_dir() {
  local src home bindir log tools record
  new_case trustcall ok
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
  new_case existing ok
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

# (D) A binary that predates `trust` (it rejects the subcommand): the
# installer must still complete (a best-effort step never fails the install),
# warn once with the manual instruction, record no trust call, and leave
# ~/.claude.json untouched.
scenario_older_binary_warns_but_does_not_fail() {
  local src home bindir log tools record
  new_case oldbinary old
  printf '{}' > "$home/.claude.json"

  run_install "$src" "$home" "$bindir" "$log" "$tools"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc (a best-effort step must not fail the installer): $(cat "$log")"; return 1; }
  [ ! -s "$record" ] || { echo "  an older binary should never record a trust call: $(cat "$record")"; return 1; }
  [ "$(grep -c "has no 'trust' command" "$log")" -eq 1 ] || { echo "  expected exactly one warning about the missing trust command: $(cat "$log")"; return 1; }
  grep -q 'playbook trust' "$log" || { echo "  the warning did not give the manual instruction: $(cat "$log")"; return 1; }
  local unchanged
  unchanged=$(cat "$home/.claude.json")
  [ "$unchanged" = "{}" ] || { echo "  ~/.claude.json was modified by an older binary: $unchanged"; return 1; }
}

# (E) A binary that supports `trust` but whose call reports a failure on
# stderr (the real subcommand always exits 0 and says so on stderr): the
# installer must complete, surface that message in a warning, and leave
# ~/.claude.json untouched.
scenario_trust_failure_is_surfaced_as_a_warning() {
  local src home bindir log tools record
  new_case trustfail fail
  printf '{}' > "$home/.claude.json"

  run_install "$src" "$home" "$bindir" "$log" "$tools"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  grep -q "simulated write failure" "$log" || { echo "  the trust failure message was not surfaced: $(cat "$log")"; return 1; }
  local unchanged
  unchanged=$(cat "$home/.claude.json")
  [ "$unchanged" = "{}" ] || { echo "  ~/.claude.json was modified after a reported trust failure: $unchanged"; return 1; }
}

run_scenario "A: no ~/.claude.json at all -> clean no-op, file stays absent, trust not called" scenario_no_claude_json_is_a_clean_noop
run_scenario "B: trust-capable binary -> trust called once with the config dir, entry created" scenario_trust_subcommand_is_called_with_the_config_dir
run_scenario "C: an existing false entry is flipped without clobbering siblings or other projects" scenario_existing_entry_is_updated_not_replaced
run_scenario "D: binary without trust -> one warning with the manual step, install continues, file untouched" scenario_older_binary_warns_but_does_not_fail
run_scenario "E: trust reports a failure -> warns with its message, install continues, file untouched" scenario_trust_failure_is_surfaced_as_a_warning

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
