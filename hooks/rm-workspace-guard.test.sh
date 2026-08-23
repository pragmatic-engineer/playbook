#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# Behavioral tests for rm-workspace-guard.sh.
#
# The guard reads a Bash tool-call JSON on stdin and, for `rm` targets outside
# PLAYBOOK_SAFE_ROOTS/** and ~/.claude/**, emits a deny decision. A target is
# "blocked" when the guard prints a JSON object; "allowed" when it prints
# nothing.
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
# These four cases predate PLAYBOOK_SAFE_ROOTS and assume the historical
# ~/Workspace root. Configure it explicitly so their original meaning (a
# configured root, plus the always-on ~/.claude, are allowed) still holds
# now that the root is no longer hardcoded.
export PLAYBOOK_SAFE_ROOTS="$HOME/Workspace"
run allow "rm -rf $HOME/Workspace/proj/build"
run allow "rm $HOME/Workspace/a.txt"
run allow "rm -rf ~/Workspace/proj/node_modules"
run allow "rm -rf $HOME/.claude/cache/x"
unset PLAYBOOK_SAFE_ROOTS

# --- Blocked: plainly outside ---
run block "rm -rf /etc/passwd"
run block "rm $HOME/secrets.txt"
run block "rm -rf /"

# --- Blocked: `..` traversal escaping the allowlist (the bug this closes) ---
# The variable is deliberately left unset for these three: they are the
# regression proof that the zero-config default is no weaker than the old
# hardcoded ~/Workspace, i.e. it must NOT default to $HOME (that would
# unblock ~/.ssh and ~/.aws).
run block "rm -rf ~/Workspace/../.ssh"
run block "rm -rf $HOME/Workspace/../../../etc/passwd"
run block "rm -rf $HOME/.claude/../.aws/credentials"

# --- Blocked: relative target after a cd (unresolvable, conservative block) ---
# Still blocked even though /tmp is an allowed root: the block is about the
# unresolvable relative target `foo`, not about where the cd points.
run block "cd /tmp && rm -rf foo"

# --- Allowed: scratch space, whatever PLAYBOOK_SAFE_ROOTS says ---
# Both spellings, because canon() is lexical and macOS /tmp is a symlink to
# /private/tmp, so matching one would allow /tmp/x and block /private/tmp/x.
run allow "rm -rf /tmp/scratch-file"
run allow "rm -rf /private/tmp/scratch-file"

# --- Blocked: the temp root ITSELF, unlike a configured safe root ---
# Wiping it takes out sockets and runtime state live processes depend on.
run block "rm -rf /tmp"
run block "rm -rf /private/tmp"

# --- Allowed: $HOME/.cache is regenerable scratch, and holds our fixtures ---
run allow "rm -rf $HOME/.cache/playbook-tests/scratch"
run allow "rm -rf ~/.cache/some-probe"

# --- Blocked: the cache ROOT itself, and anything the exemption must not reach ---
run block "rm -rf $HOME/.cache"
run block "rm -rf $HOME/.cache/../.ssh"
run block "rm -rf $HOME/.cachefoo/x"

# --- Blocked: the temp exemption is by resolved path, not by spelling ---
run block "rm -rf /tmp/../etc/passwd"
run block "rm -rf /tmpfoo/file"

# --- Allowed: a quoted mention is prose, not a command ---
# A live annoyance, not a theoretical one: it blocked commits and PR creation
# whose text merely named the guard.
export PLAYBOOK_SAFE_ROOTS="$HOME/Workspace"
# Each case must FAIL under the old tokenizer or it proves nothing. A message
# whose other words are relative paths is not enough: those canonicalise inside
# the safe root and were allowed anyway. So each carries an absolute path the
# old code judged as a target, or a substitution, which is the shape that
# actually blocked an agent mid-commit.
run allow "git commit -m \"fix: stop using rm on /etc/passwd\""
run allow "git commit -m 'docs: rm on /etc/passwd is fatal'"
run allow "git commit -m \"note: rm is risky\" && echo \$(date)"

# --- Blocked: quoting is not an escape hatch ---
# Outside quotes nothing changed, so the wrapper forms stay caught, and inside
# quotes a separator puts the token back in command position.
run block "sudo rm -rf /etc/passwd"
run block "xargs rm -rf /etc/passwd"
run block "echo hi && rm -rf /etc/passwd"
run block "sh -c \"cd /x && rm -rf /etc/passwd\""
unset PLAYBOOK_SAFE_ROOTS

# --- Blocked: an unexpanded variable is unresolvable, so it fails closed ---
# Regression for a FAIL-OPEN: canon() treated a leading `$` as a relative path
# and joined it to the cwd, so these landed inside the repo root and were
# ALLOWED, while the shell expanded them to real paths outside the workspace.
# Single-quoted here so the test harness passes the literal `$` to the guard.
run block 'rm -rf "$HOME/.cache/x"'
run block 'rm -rf $HOME/.cache/x'
run block 'rm -rf "${BUILD_DIR}/out"'
run block 'rm -rf `echo /etc`/passwd'
# Accepted cost: a variable pointing INSIDE the workspace blocks too. Failing
# closed is the correct direction, and a literal path still works.
run block 'rm -rf "$REPO/target"'

# --- Allowed: `..` that stays inside the allowlist ---
export PLAYBOOK_SAFE_ROOTS="$HOME/Workspace"
run allow "rm -rf $HOME/Workspace/proj/sub/../build"
unset PLAYBOOK_SAFE_ROOTS

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
export PLAYBOOK_SAFE_ROOTS="$HOME/Workspace"
run allow "$(printf 'echo hi\nrm -rf %s/Workspace/proj/build' "$HOME")"
unset PLAYBOOK_SAFE_ROOTS

# --- Allowed: a heredoc body is DATA, so a mention inside it is prose ---
# REVISED 2026-08-23. This was pinned as an accepted false positive on the
# grounds that the tokenizer could not tell a heredoc body from a command. It
# now can, and the trade-off was revised deliberately rather than rediscovered,
# because commit messages written through a heredoc were blocking real work.
run allow "$(printf 'cat <<EOF\nold script did rm -rf /etc/example here\nEOF')"
# The exact shape that blocked three commits: prose in the body, plus a
# substitution elsewhere in the same command.
run allow "$(printf 'git commit --file - <<EOF\nfix: block rm targets\nEOF\necho $(date)')"
# An apostrophe in the body must not flip quote state for everything after it.
run allow "$(printf "git commit --file - <<EOF\ndon't let rm escape\nEOF")"

# --- Blocked: the body is data, not a free pass ---
# A line that STARTS with rm is in command position and really does delete when
# the heredoc is fed to a shell.
run block "$(printf 'bash <<EOF\nrm -rf /etc/passwd\nEOF')"
# An unterminated heredoc is not treated as a body at all. The mention here is
# MID-LINE on purpose: a line-start deletion is in command position either way,
# so it would pass whether or not the terminator rule exists and pin nothing.
run block "$(printf 'bash <<EOF\nfoo rm -rf /etc/passwd')"
# `<<` in arithmetic is not a heredoc, so nothing after it becomes data.
run block "$(printf 'echo $((1 << 2))\nrm -rf /etc/passwd')"

# --- PLAYBOOK_SAFE_ROOTS: configurable safe roots (WU-15) ---
#
# Every fixture below lives under a throwaway sandbox, never the developer's
# real ~/.claude or real repos. One trap cleans all of them up on exit.
#
# The sandbox is deliberately NOT under mktemp's default directory. The guard
# now always allows paths inside /tmp, and on Linux `mktemp -d` returns
# /tmp/..., so an "allowed" fixture placed there would be allowed no matter what
# the root derivation did. Those assertions would pass with root handling
# completely broken, while still looking meaningful on macOS, where mktemp uses
# $TMPDIR (/var/folders).
ORIG_DIR="$(pwd)"
SCRATCH_BASE="$HOME/.cache/playbook-tests"
mkdir -p "$SCRATCH_BASE"

# A path outside every safe root AND outside the temp tree, used as the negative
# half of the default-root cases. It must NOT come from mktemp: on Linux mktemp
# returns /tmp/..., which the guard now always allows, so a temp-based fixture
# would quietly stop proving anything there while still passing on macOS, where
# mktemp uses $TMPDIR (/var/folders). Nothing here is ever created or deleted;
# the guard resolves paths lexically without touching the filesystem.
OUTSIDE="$HOME/outside-every-safe-root"
# Same reasoning for the traversal case: the root has to sit outside /tmp, or
# `$root/../.ssh` collapses to /tmp/.ssh and is legitimately allowed now.
TRAV_ROOT="$HOME/Workspace/trav-root"

repo_dir="$(mktemp -d "$SCRATCH_BASE/XXXXXX")"
# mktemp's own output can be a symlinked path (e.g. macOS /var -> /private/var).
# `git rev-parse --show-toplevel` always reports the resolved path, so resolve
# repo_dir up front too or an allow case here would false-fail on path form
# alone, not on guard logic.
repo_dir="$(cd "$repo_dir" && pwd -P)"
plain_dir="$(mktemp -d "$SCRATCH_BASE/XXXXXX")"
root_a="$(mktemp -d "$SCRATCH_BASE/XXXXXX")"
root_b="$(mktemp -d "$SCRATCH_BASE/XXXXXX")"
trail_dir="$(mktemp -d "$SCRATCH_BASE/XXXXXX")"
rel_base="$(mktemp -d "$SCRATCH_BASE/XXXXXX")"
trap 'cd "$ORIG_DIR"; rm -rf "$repo_dir" "$plain_dir" \
  "$root_a" "$root_b" "$trail_dir" "$rel_base"' \
  EXIT INT TERM

(cd "$repo_dir" && git init -q) >/dev/null 2>&1
mkdir -p "$repo_dir/sub" "$rel_base/relroot"

# 1 & 2: unset, cwd inside a git repo -> the repo root is the default root;
# a sibling outside it is not.
unset PLAYBOOK_SAFE_ROOTS
cd "$repo_dir" || exit 1
run allow "rm -rf $repo_dir/sub/file"
run block "rm -rf $OUTSIDE/file"
cd "$ORIG_DIR" || exit 1

# 3: unset, cwd NOT inside a git repo -> cwd itself is the default root.
unset PLAYBOOK_SAFE_ROOTS
cd "$plain_dir" || exit 1
run allow "rm -rf $plain_dir/file"
run block "rm -rf $OUTSIDE/file"
cd "$ORIG_DIR" || exit 1

# 4: two colon-separated roots; a target in the second is allowed.
export PLAYBOOK_SAFE_ROOTS="$root_a:$root_b"
run allow "rm -rf $root_b/file"
unset PLAYBOOK_SAFE_ROOTS

# 5: PLAYBOOK_SAFE_ROOTS explicitly empty behaves like unset: still blocks a
# target outside both the default root and ~/.claude.
export PLAYBOOK_SAFE_ROOTS=""
run block "rm -rf /etc/passwd"
unset PLAYBOOK_SAFE_ROOTS

# 6: a root with a trailing slash must not create a double-slash bug.
export PLAYBOOK_SAFE_ROOTS="$trail_dir/"
run allow "rm -rf $trail_dir/file"
unset PLAYBOOK_SAFE_ROOTS

# 7: a relative root is canon-ed against the guard's own cwd; it must resolve
# to the intended directory and must not produce a false allow elsewhere.
cd "$rel_base" || exit 1
export PLAYBOOK_SAFE_ROOTS="relroot"
run allow "rm -rf $rel_base/relroot/file"
run block "rm -rf /etc/passwd"
unset PLAYBOOK_SAFE_ROOTS
cd "$ORIG_DIR" || exit 1

# 8: a nonexistent directory in PLAYBOOK_SAFE_ROOTS must not error or grant a
# blanket allow.
export PLAYBOOK_SAFE_ROOTS="/nonexistent/definitely/not/here"
run block "rm -rf /etc/passwd"
unset PLAYBOOK_SAFE_ROOTS

# 9: traversal escaping a configured root is still closed.
export PLAYBOOK_SAFE_ROOTS="$TRAV_ROOT"
run block "rm -rf $TRAV_ROOT/../.ssh"
unset PLAYBOOK_SAFE_ROOTS

# 10: the deny message names the roots actually in effect, not a hardcoded
# literal.
export PLAYBOOK_SAFE_ROOTS="$root_a:$root_b"
reason="$(printf '{"tool_input":{"command":%s}}' "$(json_str "rm -rf /etc/passwd")" \
  | bash "$GUARD" 2>/dev/null | jq -r '.hookSpecificOutput.permissionDecisionReason // ""')"
if [[ "$reason" == *"$root_a"* && "$reason" == *"$root_b"* ]]; then
  pass=$((pass + 1))
else
  fail=$((fail + 1))
  printf 'FAIL: deny reason does not name configured roots: %s\n' "$reason" >&2
fi
unset PLAYBOOK_SAFE_ROOTS

printf '\nrm-workspace-guard: %d passed, %d failed\n' "$pass" "$fail"
[[ "$fail" -eq 0 ]]
