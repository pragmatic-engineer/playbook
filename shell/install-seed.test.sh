#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# install-seed.test.sh: hermetic tests for install.sh's settings.json handling
# now that the seam is `playbook init`, not shell/setup-local.sh. Exercises the
# PLAYBOOK_SRC local-source seam (no network), with a real `playbook` binary
# built once and staged at a scratch PLAYBOOK_BIN_DIR, since install.sh hands
# off to that binary by absolute path.
#
# `playbook init`'s settings step always goes through merge::merge, even on a
# first-ever install, and merge::merge serialises with sorted keys, so a byte
# comparison against settings.shared.json is the wrong assertion here: compare
# parsed JSON instead. The full merge matrix (customised keys, absent base,
# malformed user JSON, and so on) is already covered by tests/init_merge.rs;
# this file only covers what is specific to the install.sh integration seam.
#
# Run:  bash shell/install-seed.test.sh
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SCRIPT_DIR}/.."
INSTALL="${REPO_ROOT}/install.sh"
TEMPLATE="${REPO_ROOT}/settings.shared.json"

PASS=0
FAIL=0
pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1"; (( FAIL++ )) || true; }

command -v jq >/dev/null 2>&1 || { echo "jq not found on PATH" >&2; exit 2; }
command -v cargo >/dev/null 2>&1 || { echo "cargo not found on PATH" >&2; exit 2; }

BIN_SRC="${REPO_ROOT}/target/debug/playbook"
if [ ! -x "$BIN_SRC" ]; then
  echo "Building playbook (cargo build)..."
  ( cd "$REPO_ROOT" && cargo build --quiet ) || { echo "cargo build failed" >&2; exit 2; }
fi

# Single top-level scratch dir; each scenario carves out its own subtree.
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

BIN_DIR="$WORK/bin"
mkdir -p "$BIN_DIR"
cp "$BIN_SRC" "$BIN_DIR/playbook"
chmod 0755 "$BIN_DIR/playbook"

# seed_shipped_extras <src>: the files every scenario needs `playbook init`
# to complete WITHOUT a guards/statusline failure -- the 4 guard scripts and a
# stub statusline.sh -- so a scenario's own assertions are not drowned out by
# an unrelated step failing. Scenarios that want to test a missing template
# skip calling this for settings.shared.json specifically.
seed_shipped_extras() {
  local src="$1"
  mkdir -p "$src/hooks"
  for g in rm-workspace-guard bg-await-guard no-dash-guard precommit-check; do
    cp "$REPO_ROOT/hooks/$g.sh" "$src/hooks/$g.sh"
  done
  printf '#!/bin/sh\necho ok\n' > "$src/statusline.sh"
}

# run_install <src> <home> <log>: runs the real installer against a local
# source, fully hermetic (the PLAYBOOK_SRC seam skips the network path, and
# --no-setup skips the plugin). $SHELL is unset so the shim step skips
# cleanly instead of failing on a launcher runtime this suite does not ship.
run_install() {
  local src="$1" home="$2" log="$3"
  env -u SHELL PLAYBOOK_SRC="$src" PLAYBOOK_BIN_DIR="$BIN_DIR" \
    CLAUDE_HOME="$home/.claude" HOME="$home" \
    bash "$INSTALL" --no-setup --yes >"$log" 2>&1
}

run_scenario() {
  local name="$1" fn="$2"
  if "$fn"; then pass "$name"; else fail "$name"; fi
}

# (A) Fresh install: settings.json is valid JSON, every non-hooks key matches
# the template (compared as parsed JSON, not bytes: merge::merge serialises
# with sorted keys, so a raw template copy is never byte-identical to it once
# a real settings.json exists), and the 11 ported hooks are wired on top.
scenario_fresh() {
  local d src home log rc
  d="$(mktemp -d "$WORK/fresh.XXXXXX")"
  src="$d/src"; home="$d/home"
  mkdir -p "$src" "$home"
  cp "$TEMPLATE" "$src/settings.shared.json"
  seed_shipped_extras "$src"
  log="$d/install.log"

  run_install "$src" "$home" "$log"; rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  local settings="$home/.claude/settings.json"
  [ -f "$settings" ] || { echo "  settings.json not created"; return 1; }
  jq -e . "$settings" >/dev/null 2>&1 || { echo "  settings.json is not valid JSON"; return 1; }
  diff <(jq -S 'del(.hooks)' "$settings") <(jq -S 'del(.hooks)' "$TEMPLATE") >/dev/null \
    || { echo "  non-hooks keys differ from the template"; return 1; }
  local n_ported
  n_ported="$(jq '[.hooks[]?[]?.hooks[]?.command] | map(select(startswith("playbook hook "))) | length' "$settings")"
  [ "$n_ported" = "12" ] || { echo "  expected 12 ported hook commands (11 distinct), got $n_ported"; return 1; }
}

# (B) A user-authored hook entry in a pre-existing settings.json survives a
# re-install untouched, alongside the ported hooks `playbook init` wires in
# next to it. This is the clobber risk both merge::merge and wire::wire guard
# against, and the scenario the ADR's "settings-merge clobber" fixture matrix
# exists for at the unit level; here it is proven through the real install.sh
# entry point.
scenario_preserve_user_hook() {
  local d src home log rc
  d="$(mktemp -d "$WORK/preserve.XXXXXX")"
  src="$d/src"; home="$d/home"
  mkdir -p "$src" "$home/.claude"
  cp "$TEMPLATE" "$src/settings.shared.json"
  seed_shipped_extras "$src"
  log="$d/install.log"
  cat > "$home/.claude/settings.json" <<'EOF'
{
  "hooks": {
    "Notification": [
      { "hooks": [ { "type": "command", "command": "/opt/my-custom-notify.sh" } ] }
    ]
  }
}
EOF

  run_install "$src" "$home" "$log"; rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  local settings="$home/.claude/settings.json"
  jq -e '.hooks.Notification[0].hooks[0].command == "/opt/my-custom-notify.sh"' "$settings" >/dev/null 2>&1 \
    || { echo "  user-authored hook entry lost: $(jq -c .hooks.Notification "$settings" 2>/dev/null)"; return 1; }
  local n_ported
  n_ported="$(jq '[.hooks[]?[]?.hooks[]?.command] | map(select(startswith("playbook hook "))) | length' "$settings")"
  [ "$n_ported" = "12" ] || { echo "  ported hooks not wired alongside the user entry (got $n_ported)"; return 1; }
}

# (C) install.sh's own file-copy loop still skips a settings.json shipped in
# $SRC: only `playbook init`'s seed/merge step is allowed to write the
# installed one. Unchanged by this seam swap, but real behaviour worth
# pinning since a future edit to that loop could silently reintroduce the
# clobber.
scenario_copy_loop_skips_shipped_settings() {
  local d src home log rc
  d="$(mktemp -d "$WORK/skip.XXXXXX")"
  src="$d/src"; home="$d/home"
  mkdir -p "$src" "$home"
  cp "$TEMPLATE" "$src/settings.shared.json"
  seed_shipped_extras "$src"
  printf 'SHIPPED SENTINEL must never land\n' > "$src/settings.json"
  log="$d/install.log"

  run_install "$src" "$home" "$log"; rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc: $(cat "$log")"; return 1; }
  local settings="$home/.claude/settings.json"
  [ -f "$settings" ] || { echo "  settings.json not created"; return 1; }
  grep -q 'SHIPPED SENTINEL' "$settings" && { echo "  copy loop did NOT skip settings.json (sentinel landed)"; return 1; }
  jq -e . "$settings" >/dev/null 2>&1 || { echo "  settings.json is not valid JSON"; return 1; }
  return 0
}

run_scenario "A: fresh install seeds settings.json and wires the ported hooks" scenario_fresh
run_scenario "B: a user-authored hook entry survives install, ported hooks wired alongside it" scenario_preserve_user_hook
run_scenario "C: the copy loop skips a settings.json shipped in \$SRC" scenario_copy_loop_skips_shipped_settings

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
