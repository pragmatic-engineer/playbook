# shellcheck shell=bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0

# Optional claude override, emitted by `playbook shell-init` when the config
# key launcher.claudeOverride is true. Valid in bash and zsh.
#
# The function body lives in a single-quoted string and is evaluated only after
# the checks pass: bash and zsh expand an alias of the same name while parsing a
# function definition, so parsing it first would fail on a user's own alias.
_playbook_claude_taken() {
  if [ -n "${BASH_VERSION-}" ]; then
    case "$(builtin type -t claude 2>/dev/null)" in
      alias|builtin|keyword) return 0 ;;
      function) [ -n "${_PLAYBOOK_CLAUDE_OVERRIDE-}" ] || return 0 ;;
    esac
  else
    case "$(whence -w claude 2>/dev/null)" in
      *": alias"|*": builtin"|*": reserved") return 0 ;;
      *": function") [ -n "${_PLAYBOOK_CLAUDE_OVERRIDE-}" ] || return 0 ;;
    esac
  fi
  return 1
}
# Absolute path of the real claude executable, never a function or an alias.
_playbook_real_claude() {
  if [ -n "${BASH_VERSION-}" ]; then
    builtin type -P claude 2>/dev/null
  else
    whence -p claude 2>/dev/null
  fi
}
_playbook_is_tty() { [ -t 0 ] && [ -t 1 ]; }
# Returns 0 when the call must go straight to claude, untouched.
_playbook_claude_bypass() {
  [ "${PLAYBOOK_BYPASS-}" = "1" ] && return 0
  _playbook_is_tty || return 0
  command -v playbook >/dev/null 2>&1 || return 0
  local _pb_first _pb_arg
  _pb_first="${1-}"
  case "$_pb_first" in
    --dangerously-skip-permissions|--allow-dangerously-skip-permissions) _pb_first="${2-}" ;;
  esac
  case "$_pb_first" in
    agents|attach|auth|auto-mode|config|daemon|doctor|gateway|import|install|kill|logs|mcp|plugin|plugins|purge|remote-control|respawn|rm|self-hosted-runner|setup-token|stop|ultrareview|update) return 0 ;;
  esac
  for _pb_arg in "$@"; do
    case "$_pb_arg" in
      --) break ;;
      -p|--print|--print=*|-v|--version|-h|--help) return 0 ;;
      --output-format|--output-format=*|--input-format|--input-format=*|--json-schema|--json-schema=*) return 0 ;;
      --include-partial-messages|--include-hook-events|--replay-user-messages|--prompt-suggestions|--forward-subagent-text) return 0 ;;
      --max-turns|--max-turns=*|--max-budget-usd|--max-budget-usd=*|--no-session-persistence) return 0 ;;
      --permission-prompt-tool|--permission-prompt-tool=*|--permission-prompts|--permission-prompts=*) return 0 ;;
      --append-subagent-system-prompt|--append-subagent-system-prompt=*|--append-subagent-system-prompt-file|--append-subagent-system-prompt-file=*) return 0 ;;
      --bare|--safe-mode|--restricted|--init|--init-only|--maintenance|--bg|--background|--exec|--desktop|--cloud|--remote|--teleport) return 0 ;;
      --system-prompt|--system-prompt=*|--system-prompt-file|--system-prompt-file=*) return 0 ;;
      --append-system-prompt|--append-system-prompt=*|--append-system-prompt-file|--append-system-prompt-file=*) return 0 ;;
      --system-prompt-snapshot|--system-prompt-snapshot=*|--exclude-dynamic-system-prompt-sections) return 0 ;;
      --settings|--settings=*|--setting-sources|--setting-sources=*|--agent|--agent=*) return 0 ;;
    esac
  done
  return 1
}
_PLAYBOOK_CLAUDE_DEF='claude() {
  local _pb_real
  if _playbook_claude_bypass "$@"; then
    _pb_real="$(_playbook_real_claude)"
    if [ -z "$_pb_real" ]; then
      echo "claude: command not found" >&2
      return 127
    fi
    "$_pb_real" "$@"
    return $?
  fi
  _playbook_cc -- "$@"
}'
if ! _playbook_claude_taken && [ -n "$(_playbook_real_claude)" ]; then
  eval "$_PLAYBOOK_CLAUDE_DEF"
  _PLAYBOOK_CLAUDE_OVERRIDE=1
fi
unset _PLAYBOOK_CLAUDE_DEF
