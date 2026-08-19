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
//! - `run` composes the four above into `Command::Init`'s dispatch arm.
//!
//! `hooks/hooks.json` is deliberately NOT deleted and `settings.shared.json`
//! is deliberately NOT regenerated into binary-invoked form here: those two
//! moves, together with this one, are one atomic switchover that also needs
//! a published release to hand a binary to `install.sh`, which does not
//! exist yet. Landing this wiring alone is safe because `hooks/hooks.json`
//! still delivers the 11 functional hooks exactly as before; `wire` only
//! changes what a user's own `settings.json` looks like after they choose to
//! run `playbook init`.

pub mod merge;
pub mod run;
pub mod shim;
pub mod statusline;
pub mod wire;
