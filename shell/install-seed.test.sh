#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# install-seed.test.sh: hermetic tests for install.sh's settings.json handling.
# Exercises the PLAYBOOK_SRC local-source seam (no network) to cover the
# first-install seed of settings.json from settings.shared.json, preservation of
# a pre-existing settings.json, the no-template no-op, and the copy-loop skip
# that keeps a shipped settings.json from clobbering the seeded default.
#
# Run:  bash shell/install-seed.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SCRIPT_DIR}/.."
INSTALL="${REPO_ROOT}/install.sh"
TEMPLATE="${REPO_ROOT}/settings.shared.json"

PASS=0
FAIL=0

pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1"; (( FAIL++ )) || true; }

# Single top-level scratch dir; each scenario carves out its own subtree.
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

# Run the real installer against a local source, fully hermetic: the seam skips
# the network path and --no-setup skips brew/zshrc/shell-reload.
# setup-local.sh and merge-settings.py are injected into every test src so
# that install.sh can call setup-local.sh after the file-copy loop (WU-2.1).
run_install() {
  local src="$1" home="$2"
  mkdir -p "$src/shell"
  [ -f "$src/shell/setup-local.sh" ]    || cp "$SCRIPT_DIR/setup-local.sh"    "$src/shell/setup-local.sh"
  [ -f "$src/shell/merge-settings.py" ] || cp "$SCRIPT_DIR/merge-settings.py" "$src/shell/merge-settings.py"
  PLAYBOOK_SRC="$src" CLAUDE_HOME="$home" HOME="$home" \
    bash "$INSTALL" --no-setup >/dev/null 2>&1
}

run_scenario() {
  local name="$1" fn="$2"
  if "$fn"; then pass "$name"; else fail "$name"; fi
}

# (a) Fresh install seeds settings.json from the template.
scenario_fresh() {
  local d src home rc
  d="$(mktemp -d "$WORK/fresh.XXXXXX")"
  src="$d/src"; home="$d/home"
  mkdir -p "$src" "$home"
  cp "$TEMPLATE" "$src/settings.shared.json"
  printf 'placeholder\n' > "$src/CLAUDE.md"

  run_install "$src" "$home"; rc=$?
  [ "$rc" -eq 0 ]                       || { echo "  install rc=$rc"; return 1; }
  [ -f "$home/settings.json" ]          || { echo "  settings.json not created"; return 1; }
  cmp -s "$home/settings.json" "$TEMPLATE" || { echo "  settings.json != template"; return 1; }
}

# (b) A pre-existing settings.json survives byte-for-byte.
scenario_preserve() {
  local d src home rc
  d="$(mktemp -d "$WORK/preserve.XXXXXX")"
  src="$d/src"; home="$d/home"
  mkdir -p "$src" "$home"
  cp "$TEMPLATE" "$src/settings.shared.json"
  printf 'SENTINEL not even json {\n' > "$home/settings.json"
  cp "$home/settings.json" "$d/expected"

  run_install "$src" "$home"; rc=$?
  [ "$rc" -eq 0 ]                          || { echo "  install rc=$rc"; return 1; }
  cmp -s "$home/settings.json" "$d/expected" || { echo "  settings.json was modified"; return 1; }
}

# (c) No template present: exit 0 and create no settings.json.
scenario_no_template() {
  local d src home rc
  d="$(mktemp -d "$WORK/notmpl.XXXXXX")"
  src="$d/src"; home="$d/home"
  mkdir -p "$src" "$home"
  printf 'placeholder\n' > "$src/CLAUDE.md"

  run_install "$src" "$home"; rc=$?
  [ "$rc" -eq 0 ]               || { echo "  install rc=$rc"; return 1; }
  [ ! -e "$home/settings.json" ] || { echo "  settings.json unexpectedly created"; return 1; }
}

# (d) Copy loop skips a shipped settings.json; only the seed step writes it.
scenario_skip_shipped() {
  local d src home rc
  d="$(mktemp -d "$WORK/skip.XXXXXX")"
  src="$d/src"; home="$d/home"
  mkdir -p "$src" "$home"
  cp "$TEMPLATE" "$src/settings.shared.json"
  printf 'SHIPPED SENTINEL must never land\n' > "$src/settings.json"
  cp "$src/settings.json" "$d/sentinel"

  run_install "$src" "$home"; rc=$?
  [ "$rc" -eq 0 ]              || { echo "  install rc=$rc"; return 1; }
  [ -f "$home/settings.json" ] || { echo "  settings.json not created"; return 1; }
  if cmp -s "$home/settings.json" "$d/sentinel"; then
    echo "  copy loop did NOT skip settings.json (sentinel landed)"; return 1
  fi
  cmp -s "$home/settings.json" "$TEMPLATE"        || { echo "  settings.json != template"; return 1; }
  cmp -s "$home/settings.shared.json" "$TEMPLATE" || { echo "  settings.shared.json != template"; return 1; }
}

run_scenario "A: fresh install seeds settings.json from template" scenario_fresh
run_scenario "B: existing settings.json is preserved"            scenario_preserve
run_scenario "C: missing template is a no-op (exit 0)"           scenario_no_template
run_scenario "D: copy loop skips a shipped settings.json"        scenario_skip_shipped

# ---------------------------------------------------------------------------
# Helpers for the merge scenarios (WU-2)
# ---------------------------------------------------------------------------
MERGE="${SCRIPT_DIR}/merge-settings.py"

# run_install_rc: run install, capture stderr to a file, return exit code via rc.
# Sets _INSTALL_RC and _INSTALL_ERR_FILE.
run_install_full() {
  local src="$1" home="$2" errfile="$3"
  mkdir -p "$src/shell"
  [ -f "$src/shell/setup-local.sh" ]    || cp "$SCRIPT_DIR/setup-local.sh"    "$src/shell/setup-local.sh"
  [ -f "$src/shell/merge-settings.py" ] || cp "$SCRIPT_DIR/merge-settings.py" "$src/shell/merge-settings.py"
  PLAYBOOK_SRC="$src" CLAUDE_HOME="$home" HOME="$home" \
    bash "$INSTALL" --no-setup >/dev/null 2>"$errfile"
  _INSTALL_RC=$?
}

# make_merge_src: create a minimal src dir that includes merge-settings.py so
# install.sh can find it at $CLAUDE_HOME/shell/merge-settings.py after the copy
# loop runs.
make_merge_src() {
  local src="$1"
  mkdir -p "$src/shell"
  cp "$MERGE" "$src/shell/merge-settings.py"
}

# (a) Fresh install: settings.json seeded AND .settings.base.json written == template.
scenario_merge_fresh() {
  local d src home rc
  d="$(mktemp -d "$WORK/ma.XXXXXX")"
  src="$d/src"; home="$d/home"
  mkdir -p "$src" "$home"
  make_merge_src "$src"
  cp "$TEMPLATE" "$src/settings.shared.json"

  run_install "$src" "$home"; rc=$?
  [ "$rc" -eq 0 ]                              || { echo "  install rc=$rc"; return 1; }
  [ -f "$home/settings.json" ]                 || { echo "  settings.json not created"; return 1; }
  cmp -s "$home/settings.json" "$TEMPLATE"     || { echo "  settings.json != template"; return 1; }
  [ -f "$home/.settings.base.json" ]           || { echo "  .settings.base.json not created"; return 1; }
  cmp -s "$home/.settings.base.json" "$TEMPLATE" || { echo "  .settings.base.json != template"; return 1; }
}

# (b) Existing settings.json + base + customised key + unchanged product key:
#   customised key preserved, unchanged product key updated, snapshot dir
#   exists, base refreshed with contested key frozen.
scenario_merge_update() {
  local d src home rc
  d="$(mktemp -d "$WORK/mb.XXXXXX")"
  src="$d/src"; home="$d/home"
  mkdir -p "$src" "$home"
  make_merge_src "$src"

  # Minimal template (new version): custom_key changed by template, product_key also changed.
  printf '{"custom_key":"new_tmpl","product_key":"new_prod","other":"other"}' \
    > "$src/settings.shared.json"

  # Base: what was shipped on last install.
  printf '{"custom_key":"original","product_key":"original","other":"other"}' \
    > "$home/.settings.base.json"

  # User settings: custom_key customised, product_key unchanged from base.
  printf '{"custom_key":"my_custom","product_key":"original","other":"other"}' \
    > "$home/settings.json"

  run_install "$src" "$home"; rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc"; return 1; }

  # Customised key preserved.
  jq -e '.custom_key == "my_custom"' "$home/settings.json" >/dev/null 2>&1 \
    || { echo "  customised key not preserved: $(jq -c . "$home/settings.json")"; return 1; }

  # Unchanged product key updated to new template value.
  jq -e '.product_key == "new_prod"' "$home/settings.json" >/dev/null 2>&1 \
    || { echo "  product key not updated: $(jq -c . "$home/settings.json")"; return 1; }

  # Snapshot dir exists (setup-local.sh creates setup-* dirs for settings changes).
  local snapdirs
  snapdirs="$(find "$home/backups" -maxdepth 1 -type d -name 'setup-*' 2>/dev/null | wc -l | tr -d ' ')"
  [ "${snapdirs:-0}" -ge 1 ] \
    || { echo "  no snapshot dir found under $home/backups"; return 1; }

  # Base refreshed: product_key updated, custom_key frozen to OLD base value.
  jq -e '.product_key == "new_prod"' "$home/.settings.base.json" >/dev/null 2>&1 \
    || { echo "  base product_key not refreshed: $(jq -c . "$home/.settings.base.json")"; return 1; }
  jq -e '.custom_key == "original"' "$home/.settings.base.json" >/dev/null 2>&1 \
    || { echo "  base custom_key not frozen to old value: $(jq -c . "$home/.settings.base.json")"; return 1; }
}

# (c) Existing settings.json + NO base -> additive (all user keys kept).
scenario_merge_no_base() {
  local d src home rc
  d="$(mktemp -d "$WORK/mc.XXXXXX")"
  src="$d/src"; home="$d/home"
  mkdir -p "$src" "$home"
  make_merge_src "$src"

  printf '{"tmpl_key":"tv","shared":"ts"}' > "$src/settings.shared.json"
  # No .settings.base.json in home (first ever run after manual setup).
  printf '{"my_key":"mv","shared":"us"}' > "$home/settings.json"

  run_install "$src" "$home"; rc=$?
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc"; return 1; }

  # User keys all kept (base absent -> additive, every user key is contested).
  jq -e '.my_key == "mv"' "$home/settings.json" >/dev/null 2>&1 \
    || { echo "  user key lost: $(jq -c . "$home/settings.json")"; return 1; }

  # New template key added.
  jq -e '.tmpl_key == "tv"' "$home/settings.json" >/dev/null 2>&1 \
    || { echo "  new tmpl key missing: $(jq -c . "$home/settings.json")"; return 1; }
}

# (d) Merge failure: settings.json is a top-level array -> byte-identical after
#   install, warning emitted, install exits 0.
scenario_merge_failure() {
  local d src home errfile rc
  d="$(mktemp -d "$WORK/md.XXXXXX")"
  src="$d/src"; home="$d/home"; errfile="$d/stderr.txt"
  mkdir -p "$src" "$home"
  make_merge_src "$src"

  cp "$TEMPLATE" "$src/settings.shared.json"
  printf '[1,2,3]' > "$home/settings.json"
  cp "$home/settings.json" "$d/expected"

  run_install_full "$src" "$home" "$errfile"; rc="$_INSTALL_RC"
  [ "$rc" -eq 0 ] || { echo "  install rc=$rc (expected 0)"; return 1; }
  cmp -s "$home/settings.json" "$d/expected" \
    || { echo "  settings.json was modified (expected byte-identical)"; return 1; }
  grep -qi 'warning' "$errfile" \
    || { echo "  no warning in stderr: $(cat "$errfile")"; return 1; }
}

# (e) Idempotent second run: settings.json byte-identical AND base unchanged.
#
# Three-run strategy: run 1 seeds (cp format), run 2 is the first merge run
# (reformats to jq-sorted order + updates base), run 3 should be a strict
# no-op vs the state run 2 produced. We test idempotency between runs 2 and 3
# because the seed-to-merge format transition between runs 1 and 2 is expected.
scenario_merge_idempotent() {
  local d src home rc
  d="$(mktemp -d "$WORK/me.XXXXXX")"
  src="$d/src"; home="$d/home"
  mkdir -p "$src" "$home"
  make_merge_src "$src"
  # Use a small template so the test is fast.
  printf '{"model":"default","theme":"dark"}' > "$src/settings.shared.json"

  # Run 1: fresh install (seeds settings.json via cp; base written too).
  run_install "$src" "$home"; rc=$?
  [ "$rc" -eq 0 ] || { echo "  run1 rc=$rc"; return 1; }

  # Run 2: first merge run (normalises to jq-sorted format, refreshes base).
  run_install "$src" "$home"; rc=$?
  [ "$rc" -eq 0 ] || { echo "  run2 rc=$rc"; return 1; }

  # Capture state after run 2 as the expected baseline.
  cp "$home/settings.json"       "$d/settings_after2.json"
  cp "$home/.settings.base.json" "$d/base_after2.json"

  # Run 3: should be a no-op (idempotent).
  run_install "$src" "$home"; rc=$?
  [ "$rc" -eq 0 ] || { echo "  run3 rc=$rc"; return 1; }

  cmp -s "$home/settings.json" "$d/settings_after2.json" \
    || { echo "  settings.json changed on idempotent run"; return 1; }
  cmp -s "$home/.settings.base.json" "$d/base_after2.json" \
    || { echo "  .settings.base.json changed on idempotent run"; return 1; }
}

run_scenario "E (merge-a): fresh install seeds settings.json + .settings.base.json" scenario_merge_fresh
run_scenario "E (merge-b): merge preserves customisations, updates product keys"     scenario_merge_update
run_scenario "E (merge-c): absent base -> additive (all user keys kept)"             scenario_merge_no_base
run_scenario "E (merge-d): merge failure -> settings.json unchanged, warning, exit 0" scenario_merge_failure
run_scenario "E (merge-e): idempotent second run -> settings.json and base unchanged" scenario_merge_idempotent

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
