// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Installer/repair modules for the local Claude Code configuration, backing
//! the `playbook init` subcommand. All four are implemented:
//!
//! - `merge` ports `shell/merge-settings.py`'s three-way settings merge.
//! - `wire` writes the ported hook entries into `settings.json`, backing it up
//!   first, and leaves the four guards on their `.sh` paths until WU-13.
//! - `shim` installs the bash and zsh launcher shim idempotently.
//! - `statusline` places `statusline.sh` at the path `settings.json` names.
//!
//! Nothing here is reachable from the CLI yet: `src/main.rs`'s `Command::Init`
//! is still a stub, and WU-11 wires it together with retiring
//! `hooks/hooks.json` and regenerating the seed, as one atomic switchover.

pub mod merge;
pub mod shim;
pub mod statusline;
pub mod wire;
