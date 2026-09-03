# SPDX-FileCopyrightText: 2026 Igor Santos
# SPDX-License-Identifier: MIT
#
# Transitional shim. shell/cc.zsh moved to shell/zsh/cc.zsh as part of the
# bash/zsh/shared layout reorganisation. This file exists only so an already
# installed ~/.zshrc that still sources the old path keeps working.
#
# New installs source shell/zsh/cc.zsh directly (setup-local.sh writes that
# path). Re-run /setup or shell/setup-local.sh --aliases to update an existing
# rc file to the new path; this shim can then be removed by uninstall.sh.
[[ -f "$HOME/.config/playbook/shell/zsh/cc.zsh" ]] && source "$HOME/.config/playbook/shell/zsh/cc.zsh"
