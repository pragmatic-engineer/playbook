// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The playbook paths live in `playbook-common`; this module re-exports them
//! and adds the one path that needs the usage crate.

pub use playbook_common::paths::*;

/// The data folder that holds `usage.db`, the durable, cross-project usage
/// store shared with playbook-telemetry, and the lock and receiver files next
/// to it. The crate owns the folder logic.
pub fn usage_db_dir() -> std::path::PathBuf {
    crate::usage::data::data_dir(&crate::common::session::home_dir())
}
