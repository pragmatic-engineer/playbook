#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# gen-shared-settings.test.sh: tests for the settings.shared.json generator in
# shell/gen-shared-settings.py. Covers the happy-path transform (canned
# permissions, forced model/skipAutoPermissionPrompt, deleted personal keys,
# product-key passthrough) plus every input guard (bad JSON, missing files,
# degenerate permissions, no args).
#
# Run:  bash shell/gen-shared-settings.test.sh
# Exit: 0 if all scenarios pass, non-zero otherwise.
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GEN="${SCRIPT_DIR}/gen-shared-settings.py"
PASS=0
FAIL=0

if ! command -v jq >/dev/null 2>&1; then
  echo "SKIP: jq not available; generator tests need jq"
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

pass() { echo "PASS: $1"; (( PASS++ )) || true; }
fail() { echo "FAIL: $1${2:+ -> $2}"; (( FAIL++ )) || true; }

# Canned permissions fixture, reused read-only across scenarios.
PERMS="${WORK}/perms.json"
cat > "$PERMS" <<'JSON'
{
  "allow": ["Read", "Bash(git:*)"],
  "deny": ["Read(**/.env)"],
  "ask": ["Bash(curl:*)"],
  "defaultMode": "auto"
}
JSON

# Full live-like source with personal keys, product keys, and an unknown key.
SRC_FULL="${WORK}/src-full.json"
cat > "$SRC_FULL" <<'JSON'
{
  "model": "sonnet",
  "skipAutoPermissionPrompt": true,
  "effortLevel": "xhigh",
  "theme": "dark-daltonized",
  "preferredNotifChannel": "ghostty",
  "prefersReducedMotion": true,
  "permissions": { "allow": ["Bash"], "deny": [], "ask": [], "defaultMode": "auto" },
  "env": { "IS_DEMO": "1", "DISABLE_AUTOUPDATER": "1" },
  "hooks": { "SessionStart": [{ "hooks": [] }] },
  "statusLine": { "type": "command", "command": "bash x" },
  "customUnknownKey": { "keep": "me" }
}
JSON

# A: happy path -> canned perms, no model, forced skipAutoPermissionPrompt,
#    personal keys gone, product + unknown keys pass through.
out="$("$GEN" "$SRC_FULL" "$PERMS" 2>/dev/null)"; rc=$?
if [[ $rc -eq 0 ]] && printf '%s' "$out" | jq -e --slurpfile p "$PERMS" '
      (.permissions == $p[0])
  and (has("model") | not)
  and (.skipAutoPermissionPrompt == false)
  and ((has("effortLevel") or has("theme")
        or has("preferredNotifChannel") or has("prefersReducedMotion")) | not)
  and (.env.IS_DEMO == "1" and .env.DISABLE_AUTOUPDATER == "1")
  and (has("hooks") and has("statusLine"))
  and (.customUnknownKey.keep == "me")
' >/dev/null 2>&1; then
  pass "happy path: canned perms, model stripped, personal keys dropped, passthrough"
else
  fail "happy path" "rc=$rc"
fi

# B: model absent in source -> stays absent.
SRC_NOMODEL="${WORK}/src-nomodel.json"
echo '{"env":{"IS_DEMO":"1"}}' > "$SRC_NOMODEL"
out="$("$GEN" "$SRC_NOMODEL" "$PERMS" 2>/dev/null)"; rc=$?
if [[ $rc -eq 0 ]] && printf '%s' "$out" \
   | jq -e 'has("model") | not' >/dev/null 2>&1; then
  pass "model absent -> stays absent"
else
  fail "model absent -> stays absent" "rc=$rc"
fi

# C: model set in source -> stripped from the template.
SRC_OPUS="${WORK}/src-opus.json"
echo '{"model":"opus","env":{}}' > "$SRC_OPUS"
out="$("$GEN" "$SRC_OPUS" "$PERMS" 2>/dev/null)"; rc=$?
if [[ $rc -eq 0 ]] && printf '%s' "$out" \
   | jq -e 'has("model") | not' >/dev/null 2>&1; then
  pass "model in source -> stripped"
else
  fail "model in source -> stripped" "rc=$rc"
fi

# D: malformed source JSON -> non-zero exit AND empty stdout.
SRC_BAD="${WORK}/src-bad.json"
echo '{ this is not json ' > "$SRC_BAD"
out="$("$GEN" "$SRC_BAD" "$PERMS" 2>/dev/null)"; rc=$?
if [[ $rc -ne 0 && -z "$out" ]]; then
  pass "malformed source -> non-zero exit, empty stdout"
else
  fail "malformed source" "rc=$rc out='${out:0:40}'"
fi

# E: missing source file -> non-zero exit.
out="$("$GEN" "${WORK}/does-not-exist.json" "$PERMS" 2>/dev/null)"; rc=$?
if [[ $rc -ne 0 && -z "$out" ]]; then
  pass "missing source -> non-zero exit"
else
  fail "missing source" "rc=$rc"
fi

# F: missing permissions file -> non-zero exit.
out="$("$GEN" "$SRC_FULL" "${WORK}/no-perms.json" 2>/dev/null)"; rc=$?
if [[ $rc -ne 0 && -z "$out" ]]; then
  pass "missing permissions -> non-zero exit"
else
  fail "missing permissions" "rc=$rc"
fi

# G: degenerate permissions {} (no allow array) -> guard rejects.
PERMS_EMPTY="${WORK}/perms-empty.json"
echo '{}' > "$PERMS_EMPTY"
out="$("$GEN" "$SRC_FULL" "$PERMS_EMPTY" 2>/dev/null)"; rc=$?
if [[ $rc -ne 0 && -z "$out" ]]; then
  pass "degenerate permissions {} -> guard rejects"
else
  fail "degenerate permissions {}" "rc=$rc"
fi

# H: permissions with an empty allow array -> guard rejects.
PERMS_NOALLOW="${WORK}/perms-noallow.json"
echo '{"allow":[],"deny":[],"ask":[]}' > "$PERMS_NOALLOW"
out="$("$GEN" "$SRC_FULL" "$PERMS_NOALLOW" 2>/dev/null)"; rc=$?
if [[ $rc -ne 0 && -z "$out" ]]; then
  pass "empty allow array -> guard rejects"
else
  fail "empty allow array" "rc=$rc"
fi

# I: no arguments -> guard rejects.
out="$("$GEN" 2>/dev/null)"; rc=$?
if [[ $rc -ne 0 && -z "$out" ]]; then
  pass "no arguments -> guard rejects"
else
  fail "no arguments" "rc=$rc"
fi

# J: hooks reduced to the safety guards only; functional hooks and rtk dropped.
SRC_HOOKS="${WORK}/src-hooks.json"
cat > "$SRC_HOOKS" <<'JSON'
{
  "env": {},
  "hooks": {
    "SessionStart": [{ "hooks": [{ "type": "command", "command": "~/.claude/hooks/session-init.sh" }] }],
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [
        { "type": "command", "command": "~/.claude/hooks/rm-workspace-guard.sh", "if": "Bash(rm:*)", "timeout": 10 },
        { "type": "command", "command": "~/.claude/hooks/bg-await-guard.sh", "timeout": 10 },
        { "type": "command", "command": "~/.claude/hooks/no-dash-guard.sh", "timeout": 10 },
        { "type": "command", "command": "rtk hook claude" }
      ] },
      { "matcher": "Read", "hooks": [{ "type": "command", "command": "~/.claude/hooks/preread-size-check.sh" }] }
    ],
    "PostToolUse": [{ "matcher": "Edit", "hooks": [{ "type": "command", "command": "~/.claude/hooks/post-edit-track.sh" }] }]
  }
}
JSON
out="$("$GEN" "$SRC_HOOKS" "$PERMS" 2>/dev/null)"; rc=$?
if [[ $rc -eq 0 ]] && printf '%s' "$out" | jq -e '
      ((.hooks | keys) == ["PreToolUse"])
  and ((.hooks.PreToolUse | length) == 1)
  and ([.hooks.PreToolUse[0].hooks[].command]
        == ["~/.claude/hooks/rm-workspace-guard.sh",
            "~/.claude/hooks/bg-await-guard.sh",
            "~/.claude/hooks/no-dash-guard.sh"])
' >/dev/null 2>&1; then
  pass "hooks reduced to the three safety guards only"
else
  fail "hooks safety-only filter" "rc=$rc hooks=$(printf '%s' "$out" | jq -c '.hooks' 2>/dev/null)"
fi

TOTAL=$(( PASS + FAIL ))
echo ""
echo "${PASS}/${TOTAL} scenarios passed"

[[ $FAIL -eq 0 ]]
