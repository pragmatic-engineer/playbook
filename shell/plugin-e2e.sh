#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# plugin-e2e.sh: end-to-end verification of the playbook plugin and
# installer. Proves the "add the plugin and it just works" path in a clean-slate
# config (an isolated CLAUDE_CONFIG_DIR) and exercises install.sh into a throwaway
# HOME. Not a CI unit (needs the claude CLI and is slow); run it by hand:
#
#   bash shell/plugin-e2e.sh [REPO_DIR]
#
# Exit 0 if every check passes, non-zero otherwise (failures listed at the end).
set -u

REPO="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
PASS=0; FAIL=0; WARN=0
FAILED_NAMES=()
ok()   { printf '  \033[32mPASS\033[0m %s\n' "$1"; PASS=$((PASS+1)); }
bad()  { printf '  \033[31mFAIL\033[0m %s\n' "$1"; FAIL=$((FAIL+1)); FAILED_NAMES+=("$1"); }
warn() { printf '  \033[33mWARN\033[0m %s\n' "$1"; WARN=$((WARN+1)); }
hdr()  { printf '\n\033[1m== %s ==\033[0m\n' "$1"; }

command -v claude >/dev/null 2>&1 || { echo "claude CLI not found on PATH" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "jq not found on PATH" >&2; exit 2; }

hdr "A. Manifest validation (claude plugin validate --strict)"
[ ! -f "$REPO/.claude-plugin/marketplace.json" ] && ok "marketplace.json not in plugin repo" || bad "marketplace.json still in plugin repo"
if claude plugin validate "$REPO/.claude-plugin/plugin.json" --strict >/dev/null 2>&1; then
  ok "plugin manifest and all bundled components validate (strict)"
else
  bad "plugin manifest/components validate --strict"
  claude plugin validate "$REPO/.claude-plugin/plugin.json" --strict 2>&1 \
    | grep -iE 'error|Validating (agent|skill|command)' | head -20 | sed 's/^/       /'
fi

hdr "B. JSON well-formedness and required plugin.json fields"
for f in .claude-plugin/plugin.json hooks/hooks.json; do
  if jq empty "$REPO/$f" 2>/dev/null; then ok "$f is valid JSON"; else bad "$f invalid JSON"; fi
done
pname="$(jq -r '.name // empty' "$REPO/.claude-plugin/plugin.json")"
pver="$(jq -r '.version // empty' "$REPO/.claude-plugin/plugin.json")"
[ -n "$pname" ] && ok "plugin.json name=$pname" || bad "plugin.json missing name"
[ -n "$pver" ] && ok "plugin.json version=$pver" || bad "plugin.json missing version"

hdr "C. Hook integrity (every hooks.json command resolves and parses)"
while IFS= read -r c; do
  path="${c//\"/}"; path="${path/\$\{CLAUDE_PLUGIN_ROOT\}/$REPO}"
  base="$(basename "$path")"
  if [ -f "$path" ]; then
    case "$path" in
      *.py) checker="python3 -m py_compile" ;;
      *)    checker="bash -n" ;;
    esac
    if $checker "$path" 2>/dev/null; then ok "hook resolves and parses: $base"; else bad "hook syntax error: $base"; fi
  else
    bad "hook missing: $path"
  fi
done < <(jq -r '.hooks | to_entries[] | .value[] | .hooks[] | .command' "$REPO/hooks/hooks.json")

hdr "D. Component frontmatter (each tracked skill/command/agent has a description)"
missing=0
for f in "$REPO"/agents/*.md "$REPO"/commands/*.md "$REPO"/skills/*/SKILL.md; do
  [ -f "$f" ] || continue
  awk 'NR==1&&/^---/{f=1;next} f&&/^---/{exit} f{print}' "$f" | grep -q '^description:' \
    || { bad "no description: ${f#"$REPO"/}"; missing=1; }
done
[ "$missing" -eq 0 ] && ok "all tracked skills/commands/agents declare a description"

hdr "E. Clean-slate plugin install (isolated CLAUDE_CONFIG_DIR, tracked surface)"
BASE="$(mktemp -d)"; L="$BASE/plugin"; mkdir -p "$L/.claude-plugin"
if ! ( cd "$REPO" && git archive --format=tar HEAD 2>/dev/null | tar -x -C "$L" ); then
  cp -R "$REPO"/commands "$REPO"/skills "$REPO"/agents "$REPO"/hooks "$REPO"/.claude-plugin "$L/" 2>/dev/null
fi
[ -f "$L/.claude-plugin/plugin.json" ] || bad "archive missing .claude-plugin/plugin.json"
jq -n '{name:"e2e-local",owner:{name:"e2e",email:"e2e@localhost"},plugins:[{name:"playbook",source:"./"}]}' > "$L/.claude-plugin/marketplace.json"
export CLAUDE_CONFIG_DIR="$BASE/cfg"
claude plugin marketplace add "$L" </dev/null >/dev/null 2>&1 && ok "marketplace add (clean config)" || bad "marketplace add"
claude plugin install "playbook@e2e-local" </dev/null >/dev/null 2>&1 && ok "plugin install" || bad "plugin install"
det="$(claude plugin details playbook 2>/dev/null)"
claude plugin list 2>/dev/null | grep -q 'enabled' && ok "plugin shows enabled" || bad "plugin not enabled"
printf '%s' "$det" | grep -q 'playbook' && ok "details contain playbook" || bad "details missing playbook"
ag="$(echo "$det" | awk -F'[()]' '/Agents \(/{print $2}')"
hk="$(echo "$det" | awk -F'[()]' '/Hooks \(/{print $2}')"
ag_expected=0
for f in "$REPO"/agents/*.md; do
  [ "$(basename "$f")" = "_TEMPLATE.md" ] || ag_expected=$((ag_expected+1))
done
hk_expected="$(jq '.hooks | keys | length' "$REPO/hooks/hooks.json")"
[ "${ag:-0}" = "$ag_expected" ] && ok "inventory Agents=$ag_expected" || bad "inventory Agents=$ag (expected $ag_expected)"
[ "${hk:-0}" = "$hk_expected" ] && ok "inventory Hooks=$hk_expected event types" || bad "inventory Hooks=$hk (expected $hk_expected)"
for a in reviewer auditor git; do
  echo "$det" | grep -qw "$a" && ok "agent present: $a" || bad "agent missing: $a"
done
unset CLAUDE_CONFIG_DIR
rm -rf "$BASE"

hdr "F. install.sh into a throwaway HOME (files, settings seed, guard wiring)"
TH="$(mktemp -d)"; CH="$TH/.claude"
HOME="$TH" CLAUDE_HOME="$CH" PLAYBOOK_SRC="$REPO" \
  bash "$REPO/install.sh" --no-setup --yes >"$TH/install.log" 2>&1
rc=$?
[ "$rc" -eq 0 ] && ok "install.sh --no-setup --yes exits 0" || bad "install.sh exit=$rc"
[ -f "$CH/settings.json" ] && ok "settings.json seeded" || bad "settings.json not seeded"
[ -f "$CH/.settings.base.json" ] && ok ".settings.base.json baseline written" || bad "baseline missing"
if [ -f "$CH/settings.json" ]; then
  guards="$(jq -r '[.hooks.PreToolUse[]?.hooks[]?.command] | map(select(test("rm-workspace-guard|bg-await-guard|no-dash-guard"))) | length' "$CH/settings.json" 2>/dev/null)"
  [ "${guards:-0}" -ge 3 ] && ok "3 safety guards wired in settings.json" || bad "safety guards not wired (found ${guards:-0})"
  func="$(jq -r '[.hooks[]?[]?.hooks[]?.command] | map(select(test("session-init|search-counter|post-edit-track"))) | length' "$CH/settings.json" 2>/dev/null)"
  [ "${func:-0}" = "0" ] && ok "functional hooks NOT in settings (plugin-owned, no double-fire)" || warn "functional hooks in settings ($func)"
fi
for g in rm-workspace-guard bg-await-guard no-dash-guard; do
  [ -f "$CH/hooks/$g.sh" ] && ok "guard installed: $g.sh" || bad "guard not installed: $g.sh"
done
[ ! -e "$CH/commands" ] && ok "commands/ not copied (plugin-owned)" || warn "commands/ copied directly"
rm -rf "$TH"

hdr "G. Safety guards behave (deny/allow)"
emdash="$(printf '\xe2\x80\x94')"
out="$(printf '{"tool_input":{"command":"git commit -m \\"x %s y\\""}}' "$emdash" | bash "$REPO/hooks/no-dash-guard.sh" 2>/dev/null)"
[ -n "$out" ] && ok "no-dash-guard blocks em dash in git commit" || bad "no-dash-guard did not block"
out="$(printf '{"tool_input":{"command":"git commit -m \\"clean message\\""}}' | bash "$REPO/hooks/no-dash-guard.sh" 2>/dev/null)"
[ -z "$out" ] && ok "no-dash-guard allows a clean commit" || bad "no-dash-guard false positive"
out="$(printf '{"tool_input":{"command":"rm -rf /etc/hosts"}}' | bash "$REPO/hooks/rm-workspace-guard.sh" 2>/dev/null)"
[ -n "$out" ] && ok "rm-workspace-guard blocks rm outside the allowlist" || bad "rm-workspace-guard did not block"

hdr "H. Repo validators and behavioral suites"
( cd "$REPO" && bash shell/check-manifest.sh >/dev/null 2>&1 ) && ok "check-manifest" || bad "check-manifest"
( cd "$REPO" && python3 shell/check-shared-settings.py settings.shared.json permissions.shared.json . >/dev/null 2>&1 ) \
  && ok "check-shared-settings" || bad "check-shared-settings"
tp=0; tf=0
while IFS= read -r t; do
  if ( cd "$REPO" && bash "$t" >/dev/null 2>&1 ); then tp=$((tp+1)); else tf=$((tf+1)); bad "suite: $t"; fi
done < <(cd "$REPO" && git ls-files '*.test.sh')
[ "$tf" -eq 0 ] && ok "all $tp behavioral test suites pass" || bad "$tf test suite(s) failed"

hdr "RESULT"
printf 'PASS=%d  FAIL=%d  WARN=%d\n' "$PASS" "$FAIL" "$WARN"
if [ "$FAIL" -gt 0 ]; then
  printf 'FAILURES:\n'
  for n in "${FAILED_NAMES[@]}"; do printf '  - %s\n' "$n"; done
fi
[ "$FAIL" -eq 0 ]
