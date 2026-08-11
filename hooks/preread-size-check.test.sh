#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Behavioral tests for the preread-size-check PreToolUse hook (python port).
# Verifies it denies a full Read of a large file, honours the allowlist and an
# explicit offset/limit, and passes small files and missing paths.
#
# Payloads are built into a variable before the assertion: bash 3.2 mis-parses
# nested double quotes inside `[ ... "$(cmd "...")" ]`, so assign-first is the
# portable idiom here.
#
# Run:  bash hooks/preread-size-check.test.sh
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
HOOK="$HERE/preread-size-check.py"

PASS=0
FAIL=0
ok()  { echo "PASS: $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL: $1"; FAIL=$((FAIL+1)); }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

seq 1 1500 > "$WORK/big.log"           # 1500 lines -> over the line limit
printf 'a\nb\nc\n' > "$WORK/small.txt"  # tiny
cp "$WORK/big.log" "$WORK/package.json" # large but allowlisted
cp "$WORK/big.log" "$WORK/tsconfig.build.json"

run() { printf '%s' "$1" | python3 "$HOOK" 2>/dev/null; }
# payload for a Read of $1 with optional extra tool_input json fragment $2.
payload() { printf '{"tool_input":{"file_path":"%s"%s}}' "$1" "${2:-}"; }

# 1. Large non-allowlisted file -> deny with line/byte counts.
out="$(run "$(payload "$WORK/big.log")")"
if printf '%s' "$out" | grep -q '"permissionDecision":"deny"' && printf '%s' "$out" | grep -q '1500 lines'; then
  ok "large file denied with counts"
else
  bad "large file denied with counts (got: $out)"
fi

# 2. Small file -> allow (silent).
out="$(run "$(payload "$WORK/small.txt")")"
[ -z "$out" ] && ok "small file passes" || bad "small file passes (got: $out)"

# 3. Allowlisted large file (package.json) -> allow.
out="$(run "$(payload "$WORK/package.json")")"
[ -z "$out" ] && ok "allowlisted large file passes" || bad "allowlisted passes (got: $out)"

# 4. Explicit offset -> allow even when large.
out="$(run "$(payload "$WORK/big.log" ',"offset":10')")"
[ -z "$out" ] && ok "explicit offset bypasses the guard" || bad "offset bypass (got: $out)"

# 5. Explicit limit -> allow even when large.
out="$(run "$(payload "$WORK/big.log" ',"limit":50')")"
[ -z "$out" ] && ok "explicit limit bypasses the guard" || bad "limit bypass (got: $out)"

# 6. Missing file -> allow, exit 0.
p="$(payload "$WORK/does-not-exist")"
run "$p" >/dev/null; rc=$?
out="$(run "$p")"
{ [ "$rc" -eq 0 ] && [ -z "$out" ]; } && ok "missing file is a silent no-op" || bad "missing file no-op (rc=$rc out=$out)"

# 7. glob allowlist: tsconfig.build.json matches tsconfig.*.json.
out="$(run "$(payload "$WORK/tsconfig.build.json")")"
[ -z "$out" ] && ok "tsconfig.*.json glob allowlisted" || bad "tsconfig glob allowlist (got: $out)"

TOTAL=$((PASS+FAIL))
echo ""
echo "${PASS}/${TOTAL} cases passed"
[ "$FAIL" -eq 0 ]
