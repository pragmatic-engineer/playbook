#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# config-drift.test.sh: wiring tests for shell/shared/config-drift.sh after the
# dedupe refactor. Runs as a bash harness driving zsh subprocesses, HOME-isolated
# to a scratch .claude tree.
#
# Tests:
#   PARITY  : config_hash via bash (config-hash.sh) and via zsh (config-drift.zsh)
#             return the same 16-char hash for the same tree.
#   DRIFT   : _cc_config_drifted returns non-empty when a hook script changed.
#   STAMP   : _cc_config_stamp + _cc_config_drifted is quiet when nothing changed.
#
# Run:  bash shell/config-drift.test.sh
# Exit: 0 if all cases pass, non-zero otherwise.
set -u

if ! command -v zsh >/dev/null 2>&1; then
  echo "SKIP: zsh not available"
  exit 0
fi

if ! command -v shasum >/dev/null 2>&1; then
  echo "SKIP: shasum not available"
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DRIFT_ZSH="${SCRIPT_DIR}/shared/config-drift.sh"
CONFIG_HASH_SH="${SCRIPT_DIR}/../hooks/lib/config-hash.sh"
# shellcheck source=hooks/lib/config-hash.sh
source "$CONFIG_HASH_SH"

PASS=0
FAIL=0

ok()   { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1"; (( FAIL++ )) || true; }

# ── Scratch HOME ──────────────────────────────────────────────────────────────
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT INT TERM

mkdir -p "$SCRATCH/.claude/hooks/lib"
mkdir -p "$SCRATCH/.claude/cc-state"
printf '{"model":"claude-opus-4-5"}\n' > "$SCRATCH/.claude/settings.json"
printf '#!/bin/sh\n# hook a\n'         > "$SCRATCH/.claude/hooks/a.sh"
printf '#!/bin/sh\n# hook b\n'         > "$SCRATCH/.claude/hooks/b.sh"
# Copy the shared lib so config-drift.zsh can source it from $HOME/.claude/hooks/lib/
cp "$CONFIG_HASH_SH" "$SCRATCH/.claude/hooks/lib/config-hash.sh"

# Fake project dir so _cc_config_marker produces a stable path.
mkdir -p "$SCRATCH/project"

# ── PARITY: bash and zsh produce the same hash ────────────────────────────────
bash_hash="$(HOME="$SCRATCH" config_hash)"

# shellcheck disable=SC2016
zsh_hash="$(HOME="$SCRATCH" zsh -c 'source "$1"; config_hash' _ "$DRIFT_ZSH")"

if [[ -n "$bash_hash" && "$bash_hash" == "$zsh_hash" ]]; then
  ok "PARITY: bash and zsh emit identical hash for same tree"
else
  fail "PARITY: bash='$bash_hash' zsh='$zsh_hash'"
fi

# ── STAMP: stamp then drifted is quiet ────────────────────────────────────────
# Use a dedicated project dir so STAMP and DRIFT tests have separate markers.
mkdir -p "$SCRATCH/proj-stamp"

# shellcheck disable=SC2016
HOME="$SCRATCH" zsh -c 'cd "$2"; source "$1"; _cc_config_stamp' \
  _ "$DRIFT_ZSH" "$SCRATCH/proj-stamp"

# shellcheck disable=SC2016
stamp_result="$(HOME="$SCRATCH" zsh -c 'cd "$2"; source "$1"; _cc_config_drifted' \
  _ "$DRIFT_ZSH" "$SCRATCH/proj-stamp")"

if [[ -z "$stamp_result" ]]; then
  ok "STAMP: _cc_config_drifted is quiet after stamp with no change"
else
  fail "STAMP: expected empty output, got '$stamp_result'"
fi

# ── DRIFT: drifted returns non-empty after a hook script changed ───────────────
mkdir -p "$SCRATCH/proj-drift"

# shellcheck disable=SC2016
HOME="$SCRATCH" zsh -c 'cd "$2"; source "$1"; _cc_config_stamp' \
  _ "$DRIFT_ZSH" "$SCRATCH/proj-drift"

# Mutate a hook script: this should change the hash.
printf '# drift marker\n' >> "$SCRATCH/.claude/hooks/a.sh"

# shellcheck disable=SC2016
drift_result="$(HOME="$SCRATCH" zsh -c 'cd "$2"; source "$1"; _cc_config_drifted' \
  _ "$DRIFT_ZSH" "$SCRATCH/proj-drift")"

if [[ -n "$drift_result" ]]; then
  ok "DRIFT: _cc_config_drifted returns non-empty after hook change"
else
  fail "DRIFT: expected non-empty output after hook change, got empty"
fi

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} cases passed"
[[ $FAIL -eq 0 ]]
