# shellcheck shell=bash
# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: Apache-2.0

# Optional short name, emitted by `playbook shell-init` when the config key
# launcher.shortName is true. Valid in bash and zsh.
#
# `pb` is defined only when nothing else uses the name: no command on PATH, no
# alias, no function, no builtin or keyword. The check runs here, every time the
# rc file is read, so a tool installed later still wins. The function body lives
# in a single-quoted string and is evaluated only after the check passes, because
# bash and zsh expand an alias of the same name while parsing a definition.
_playbook_pb_taken() {
  if [ -n "${BASH_VERSION-}" ]; then
    case "$(builtin type -t pb 2>/dev/null)" in
      "") return 1 ;;
      function) [ -n "${_PLAYBOOK_PB-}" ] || return 0; return 1 ;;
      *) return 0 ;;
    esac
  else
    case "$(whence -w pb 2>/dev/null)" in
      *": none"|"") return 1 ;;
      *": function") [ -n "${_PLAYBOOK_PB-}" ] || return 0; return 1 ;;
      *) return 0 ;;
    esac
  fi
}
_PLAYBOOK_PB_DEF='pb() { command playbook "$@"; }'
if ! _playbook_pb_taken; then
  eval "$_PLAYBOOK_PB_DEF"
  _PLAYBOOK_PB=1
fi
unset _PLAYBOOK_PB_DEF
