// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook gate` subcommand family, backed by a local SQLite database at
//! `.claude/state.db`. This module holds the schema and connection layer
//! (`db`); `record` and `check` land in later Work Units.

pub mod db;
