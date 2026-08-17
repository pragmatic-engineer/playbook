// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Installer/repair modules for the local Claude Code configuration, backing
//! the `playbook init` subcommand. `merge` ports shell/merge-settings.py's
//! three-way settings merge; `wire`, `shim` and `statusline` are filled in by
//! later Work Units.

pub mod merge;
pub mod shim;
pub mod statusline;
pub mod wire;
