// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Installer/repair modules for the local Claude Code configuration, backing
//! the `playbook init` subcommand:
//!
//! - `merge` is the three-way settings merge (the retired shell original).
//! - `wire` writes the ported hook entries into `settings.json`, backing it up
//!   first.
//! - `shim` installs the bash and zsh launcher shim idempotently.
//! - `statusline` moves a legacy `statusLine.command` to `playbook statusline`.
//! - `system_prompt` places the opt-in `prompts/SYSTEM_PROMPT.md`.
//! - `memory_migrate`, `migrate` and `self_root` handle upgrades and the plugin root.
//! - `run` composes all five above into `Command::Init`'s dispatch arm.
//!
//! Three of those five place a file, and each places one that some other
//! component names: `settings.json`'s `statusLine.command` names the
//! statusline, and `commands/doctor.md`'s Layer 4 names the system prompt.
//! That is the rule
//! this module enforces, **the component that names a path is the component
//! that puts the file there**, learned twice the hard way (the 2026-08-12
//! statusline outage, and the 2026-08-11 hook-rename incident that produced
//! roughly 110 silent errors over 28 hours).
//!
//! `hooks/hooks.json` now carries only the migration check and the two
//! worktree hooks, and `settings.shared.json` is already in binary-invoked
//! form. `wire` changes what a user's own `settings.json` looks like after
//! they choose to run `playbook init`.

pub mod memory_migrate;
pub mod merge;
pub mod migrate;
pub mod run;
pub mod self_root;
pub mod shell_init;
pub mod shim;
pub mod statusline;
pub mod system_prompt;
pub mod wire;
