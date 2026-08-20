// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Installer/repair modules for the local Claude Code configuration, backing
//! the `playbook init` subcommand:
//!
//! - `merge` ports `shell/merge-settings.py`'s three-way settings merge.
//! - `guards` copies the four bash safety guards to the `~/.claude/hooks/`
//!   paths `wire` names, and reports which ones landed.
//! - `wire` writes the ported hook entries into `settings.json`, backing it up
//!   first, and leaves the guards `guards` placed on their `.sh` paths until
//!   WU-13.
//! - `shim` installs the bash and zsh launcher shim idempotently.
//! - `statusline` places `statusline.sh` at the path `settings.json` names.
//! - `run` composes all five above into `Command::Init`'s dispatch arm.
//!
//! Three of those five place a file, and each places one that some other
//! component names: `settings.json`'s `statusLine.command` names the
//! statusline, and `wire`'s guard commands name the guard scripts. That is
//! the rule this module enforces, **the component that names a path is the
//! component that puts the file there**, learned twice the hard way (the
//! 2026-08-12 statusline outage, and the 2026-08-11 hook-rename incident that
//! produced roughly 110 silent errors over 28 hours).
//!
//! `hooks/hooks.json` is deliberately NOT deleted and `settings.shared.json`
//! is deliberately NOT regenerated into binary-invoked form here: those two
//! moves are one atomic switchover, and they land together in WU-11's second
//! PR now that v0.10.0 has published a binary for `install.sh` to fetch.
//! Landing this wiring alone is safe because `hooks/hooks.json` still
//! delivers the 11 functional hooks exactly as before; `wire` only changes
//! what a user's own `settings.json` looks like after they choose to run
//! `playbook init`.

pub mod guards;
pub mod merge;
pub mod run;
pub mod shim;
pub mod statusline;
pub mod wire;
