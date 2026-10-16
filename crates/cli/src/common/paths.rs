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
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(not(unix))]
pub fn is_writable(dir: &std::path::Path) -> bool {
    std::fs::metadata(dir).is_ok_and(|m| !m.permissions().readonly())
}
