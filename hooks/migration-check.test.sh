#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# Behavioral tests for migration-check.sh, the one hook hooks.json still
# registers. Wired settings.json (already carries a `playbook hook
# session-init` command) stays silent; unwired settings.json warns; a
# missing or malformed settings.json never breaks the session.
#
# Run:  bash hooks/migration-check.test.sh
set -u

HERE="$(cd "$(dirname "$0")" && pwd)"
GUARD="$HERE/migration-check.sh"

PASS=0
FAIL=0
ok()  { echo "PASS: $1"; PASS=$((PASS+1)); }
bad() { echo "FAIL: $1"; FAIL=$((FAIL+1)); }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

run_guard() {  # <home> -> stdout on OUT, exit code on RC
  OUT="$(HOME="$1" bash "$GUARD" 2>"$WORK/stderr.txt")"
  RC=$?
}

# 1. Wired settings.json (carries a playbook hook session-init command) -> silent, exit 0.
home1="$WORK/wired"
mkdir -p "$home1/.claude"
cat > "$home1/.claude/settings.json" <<'EOF'
{
  "hooks": {
    "SessionStart": [
      { "hooks": [ { "type": "command", "command": "playbook hook session-init" } ] }
    ]
  }
}
EOF
run_guard "$home1"
[ "$RC" -eq 0 ] && ok "wired settings: exits 0" || bad "wired settings: exits 0 (rc=$RC)"
[ -z "$OUT" ] && ok "wired settings: silent" || bad "wired settings: silent (got: $OUT)"

# 2. Unwired settings.json (no playbook hook commands at all) -> warns, exit 0.
home2="$WORK/unwired"
mkdir -p "$home2/.claude"
cat > "$home2/.claude/settings.json" <<'EOF'
{
  "hooks": {
    "PreToolUse": [
      { "matcher": "Bash", "hooks": [ { "type": "command", "command": "~/.claude/hooks/rm-workspace-guard.sh" } ] }
    ]
  }
}
EOF
run_guard "$home2"
[ "$RC" -eq 0 ] && ok "unwired settings: exits 0" || bad "unwired settings: exits 0 (rc=$RC)"
case "$OUT" in
  *'"hookEventName":"SessionStart"'*'Re-run the installer'*) ok "unwired settings: emits the re-run warning" ;;
  *) bad "unwired settings: emits the re-run warning (got: $OUT)" ;;
esac

# 3. Missing settings.json -> exit 0, no traceback, and still warns (nothing wired).
home3="$WORK/missing"
mkdir -p "$home3"
run_guard "$home3"
[ "$RC" -eq 0 ] && ok "missing settings.json: exits 0" || bad "missing settings.json: exits 0 (rc=$RC)"
[ ! -s "$WORK/stderr.txt" ] && ok "missing settings.json: no stderr traceback" || bad "missing settings.json: no stderr traceback (got: $(cat "$WORK/stderr.txt"))"
[ -n "$OUT" ] && ok "missing settings.json: warns (nothing is wired)" || bad "missing settings.json: warns"

# 4. Malformed settings.json -> exit 0, no traceback.
home4="$WORK/malformed"
mkdir -p "$home4/.claude"
printf 'not even json {\n' > "$home4/.claude/settings.json"
run_guard "$home4"
[ "$RC" -eq 0 ] && ok "malformed settings.json: exits 0" || bad "malformed settings.json: exits 0 (rc=$RC)"
[ ! -s "$WORK/stderr.txt" ] && ok "malformed settings.json: no stderr traceback" || bad "malformed settings.json: no stderr traceback (got: $(cat "$WORK/stderr.txt"))"

TOTAL=$((PASS+FAIL))
echo ""
echo "${PASS}/${TOTAL} cases passed"
[ "$FAIL" -eq 0 ]
