// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook gate` subcommand family, backed by a local SQLite database at
//! `.claude/state.db`. This module holds the schema and connection layer
//! (`db`) and the `gate record` CLI entry point (`record`); `check` lands in
//! a later Work Unit.

pub mod db;
pub mod record;
