#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Behavioral tests for rm-workspace-guard.sh.
#
# The guard reads a Bash tool-call JSON on stdin and, for `rm` targets outside
# ~/Workspace/** and ~/.claude/**, emits a deny decision. A target is "blocked"
# when the guard prints a JSON object; "allowed" when it prints nothing.
#
# Run:  bash hooks/rm-workspace-guard.test.sh
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
GUARD="$HERE/rm-workspace-guard.sh"
pass=0
fail=0

# run <expect: allow|block> <command-string>
run() {
  local expect="$1" cmd="$2" out
  out="$(printf '{"tool_input":{"command":%s}}' "$(json_str "$cmd")" | bash "$GUARD" 2>/dev/null)"
  local got="allow"
  [[ -n "$out" ]] && got="block"
  if [[ "$got" == "$expect" ]]; then
    pass=$((pass + 1))
  else
    fail=$((fail + 1))
    printf 'FAIL: expected %s, got %s for: %s\n' "$expect" "$got" "$cmd" >&2
  fi
}

# Minimal JSON string encoder for the command field. Must escape newline and
# tab too: a raw control character inside a JSON string is invalid JSON and
# makes jq fail closed, which would hide the multi-line/tab scenarios below.
json_str() {
  local s="$1"
  s="${s//\\/\\\\}"
  s="${s//\"/\\\"}"
  s="${s//$'\n'/\\n}"
  s="${s//$'\t'/\\t}"
  printf '"%s"' "$s"
}

# --- Allowed: inside the two safe roots ---
run allow "rm -rf $HOME/Workspace/proj/build"
run allow "rm $HOME/Workspace/a.txt"
run allow "rm -rf ~/Workspace/proj/node_modules"
run allow "rm -rf $HOME/.claude/cache/x"

# --- Blocked: plainly outside ---
run block "rm -rf /etc/passwd"
run block "rm $HOME/secrets.txt"
run block "rm -rf /"

# --- Blocked: `..` traversal escaping the allowlist (the bug this closes) ---
run block "rm -rf ~/Workspace/../.ssh"
run block "rm -rf $HOME/Workspace/../../../etc/passwd"
run block "rm -rf $HOME/.claude/../.aws/credentials"

# --- Blocked: relative target after a cd (unresolvable, conservative block) ---
run block "cd /tmp && rm -rf foo"

# --- Allowed: `..` that stays inside the allowlist ---
run allow "rm -rf $HOME/Workspace/proj/sub/../build"

# --- Blocked: rm on the second line (read only tokenizes one line) ---
run block "$(printf 'echo hi\nrm -rf /etc/passwd')"

# --- Blocked: tab-separated command hits the same blind spot as newlines ---
run block "$(printf 'echo hi\trm -rf /etc/passwd')"

# --- Blocked: rm reached through command substitution, blocked conservatively
# since the tokenizer cannot evaluate what the substitution expands to ---
run block 'rm -rf $(echo /etc)'

# --- Allowed: multi-line command with no rm on any line ---
run allow "$(printf 'echo hi\necho there')"

# --- Allowed: rm on line 2 targeting an allowed root ---
run allow "$(printf 'echo hi\nrm -rf %s/Workspace/proj/build' "$HOME")"

# --- Blocked: heredoc body containing the literal words `rm -rf /etc/example`
# but no rm command actually runs. ACCEPTED false positive: the tokenizer can't
# tell a heredoc body from a real command, so this blocks even though nothing
# is being removed. The trade-off is intentional, pinned here so it's not
# "discovered" as a bug later. ---
run block "$(printf 'cat <<EOF\nold script did rm -rf /etc/example here\nEOF')"

printf '\nrm-workspace-guard: %d passed, %d failed\n' "$pass" "$fail"
[[ "$fail" -eq 0 ]]
