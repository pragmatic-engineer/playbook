#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# install-claude-version-gate.test.sh: install.sh must skip the plugin step,
# with a clear upgrade message, when the installed claude CLI is older than
# CLAUDE_MIN_VERSION, and must proceed normally at or above it. Uses the same
# PLAYBOOK_SRC local-source seam and staged real `playbook` binary as
# shell/install-seed.test.sh, but runs WITHOUT --no-setup (which would skip
# the plugin section entirely, the one this file exercises) and stubs `claude`
# on PATH so its reported version is controllable.
#
# Run:  bash shell/install-claude-version-gate.test.sh
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SCRIPT_DIR}/.."
INSTALL="${REPO_ROOT}/install.sh"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1"; (( FAIL++ )) || true; }

command -v cargo >/dev/null 2>&1 || { echo "cargo not found on PATH" >&2; exit 2; }

BIN_SRC="${REPO_ROOT}/target/debug/playbook"
if [ ! -x "$BIN_SRC" ]; then
  echo "Building playbook (cargo build)..."
  ( cd "$REPO_ROOT" && cargo build --quiet ) || { echo "cargo build failed" >&2; exit 2; }
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

BIN_DIR="$WORK/bin"
mkdir -p "$BIN_DIR"
cp "$BIN_SRC" "$BIN_DIR/playbook"
chmod 0755 "$BIN_DIR/playbook"

seed_shipped_extras() {
  local src="$1"
  printf '#!/bin/sh\necho ok\n' > "$src/statusline.sh"
  mkdir -p "$src/shell/bash" "$src/shell/zsh" "$src/shell/shared"
  printf '#!/bin/sh\n' > "$src/shell/bash/cc.sh"
  printf '#!/bin/sh\n' > "$src/shell/zsh/cc.zsh"
}

# stub_claude <dir> <version> <call-log>: a fake `claude` on PATH. --version
# prints "<version> (Claude Code)", matching the real CLI's shape (install.sh
# parses the first field). Any `plugin ...` invocation appends its args to
# call-log and exits 0, standing in for a real marketplace add/install/enable
# without touching the network or a real Claude Code install.
stub_claude() {
  local dir="$1" version="$2" call_log="$3"
  cat > "$dir/claude" <<EOF
#!/usr/bin/env bash
case "\${1:-}" in
  --version) printf '%s (Claude Code)\n' "$version" ;;
  plugin) printf '%s\n' "\$*" >> "$call_log" ;;
esac
exit 0
EOF
  chmod +x "$dir/claude"
}

# run_install <src> <home> <log> <claude-dir>: runs the real installer past
# the plugin section (no --no-setup), with the stubbed claude ahead of any
# real one on PATH. $SHELL unset so the rc-file shim step skips cleanly.
run_install() {
  local src="$1" home="$2" log="$3" claude_dir="$4"
  env -u SHELL PLAYBOOK_SRC="$src" PLAYBOOK_BIN_DIR="$BIN_DIR" \
    CLAUDE_HOME="$home/.claude" HOME="$home" PATH="$claude_dir:$PATH" \
    bash "$INSTALL" --yes >"$log" 2>&1
}

run_scenario() {
  local name="$1" fn="$2"
  if "$fn"; then pass "$name"; else fail "$name"; fi
}

# (A) claude below CLAUDE_MIN_VERSION (2.1.121): the plugin step must be
# skipped with a warning naming both the installed and minimum versions, and
# neither marketplace-add nor install may be invoked.
scenario_below_minimum() {
  local d src home log claude_dir call_log
  d="$(mktemp -d "$WORK/below.XXXXXX")"
  src="$d/src"; home="$d/home"; claude_dir="$d/claude-bin"
  mkdir -p "$src" "$home" "$claude_dir"
  seed_shipped_extras "$src"
  log="$d/install.log"
  call_log="$d/claude-calls.log"
  : > "$call_log"
  stub_claude "$claude_dir" "2.0.0" "$call_log"

  run_install "$src" "$home" "$log" "$claude_dir"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  grep -q "2.0.0" "$log" || { echo "  installed version not named: $(cat "$log")"; return 1; }
  grep -q "2.1.121" "$log" || { echo "  minimum version not named: $(cat "$log")"; return 1; }
  [ ! -s "$call_log" ] || { echo "  plugin subcommands were invoked despite the old version: $(cat "$call_log")"; return 1; }
}

# (B) claude at exactly CLAUDE_MIN_VERSION: the boundary is inclusive
# (version_ge, not a strict >), so the plugin step must proceed.
scenario_at_minimum() {
  local d src home log claude_dir call_log
  d="$(mktemp -d "$WORK/atmin.XXXXXX")"
  src="$d/src"; home="$d/home"; claude_dir="$d/claude-bin"
  mkdir -p "$src" "$home" "$claude_dir"
  seed_shipped_extras "$src"
  log="$d/install.log"
  call_log="$d/claude-calls.log"
  : > "$call_log"
  stub_claude "$claude_dir" "2.1.121" "$call_log"

  run_install "$src" "$home" "$log" "$claude_dir"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  grep -q "marketplace add" "$call_log" || { echo "  marketplace add not invoked at the exact minimum: $(cat "$call_log")"; return 1; }
  grep -q "install" "$call_log" || { echo "  plugin install not invoked at the exact minimum: $(cat "$call_log")"; return 1; }
}

# (C) claude above CLAUDE_MIN_VERSION: proceeds normally, same as (B) but
# pins the common case (a real current install) rather than the boundary.
scenario_above_minimum() {
  local d src home log claude_dir call_log
  d="$(mktemp -d "$WORK/above.XXXXXX")"
  src="$d/src"; home="$d/home"; claude_dir="$d/claude-bin"
  mkdir -p "$src" "$home" "$claude_dir"
  seed_shipped_extras "$src"
  log="$d/install.log"
  call_log="$d/claude-calls.log"
  : > "$call_log"
  stub_claude "$claude_dir" "2.1.269" "$call_log"

  run_install "$src" "$home" "$log" "$claude_dir"
  local rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  grep -q "marketplace add" "$call_log" || { echo "  marketplace add not invoked above the minimum: $(cat "$call_log")"; return 1; }
}

run_scenario "A: claude below the minimum version skips the plugin with a clear warning" scenario_below_minimum
run_scenario "B: claude at exactly the minimum version proceeds (inclusive boundary)" scenario_at_minimum
run_scenario "C: claude above the minimum version proceeds normally" scenario_above_minimum

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
