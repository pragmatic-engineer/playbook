// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Serialises the tests that run the code under test in a chosen cwd.

use std::path::Path;

static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs `f` with the process cwd set to `dir`: the code under test runs git
/// in the cwd, so tests serialise on it like `tests/gate_record.rs`.
pub fn in_dir<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = CWD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prev = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(dir).expect("cd");
    let out = f();
    std::env::set_current_dir(prev).expect("restore cwd");
    out
}
