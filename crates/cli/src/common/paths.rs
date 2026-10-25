// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The playbook paths live in `playbook-common`; this module re-exports them
//! and adds the one path that needs the usage crate.

pub use playbook_common::paths::*;

/// The data folder that holds `usage.db`, the durable, cross-project usage
/// store shared with pbtop, and the lock and receiver files next
/// to it. The crate owns the folder logic.
pub fn usage_db_dir() -> std::path::PathBuf {
    crate::usage::data::data_dir(&crate::common::session::home_dir())
}

/// `[[ -w dir ]]`: true when the effective user may write into `dir`.
#[cfg(unix)]
pub fn is_writable(dir: &std::path::Path) -> bool {
    rustix::fs::access(dir, rustix::fs::Access::WRITE_OK).is_ok()
}

#[cfg(not(unix))]
pub fn is_writable(dir: &std::path::Path) -> bool {
    std::fs::metadata(dir).is_ok_and(|m| !m.permissions().readonly())
}
