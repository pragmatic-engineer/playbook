// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook gate` subcommand family, backed by a local SQLite database at
//! `.claude/state.db`. This module holds the schema and connection layer
//! (`db`), the `gate record` CLI entry point (`record`), and the
//! `gate check` CLI entry point (`check`).

pub mod check;
pub mod db;
pub mod record;
