# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
# shellcheck shell=bash
# (sourced, so it has no shebang; the directive tells shellcheck the dialect)
#
# Transitional shim. shell/cc.sh moved to shell/bash/cc.sh as part of the
# bash/zsh/shared layout reorganisation. This file exists only so an already
# installed ~/.bashrc that still sources the old path keeps working.
#
# New installs source shell/bash/cc.sh directly (setup-local.sh writes that
# path). Re-run /playbook:setup or shell/setup-local.sh --aliases to update an existing
# rc file to the new path; this shim can then be removed by uninstall.sh.
[[ -f "$HOME/.claude/shell/bash/cc.sh" ]] && source "$HOME/.claude/shell/bash/cc.sh"
