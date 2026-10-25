// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Runs a new executable once before the test that times it.

#[cfg(unix)]
use std::process::{Command, Stdio};

/// Run a freshly written executable once, outside any timeout under test.
/// Some machines scan a new executable on its first run, which can take
/// longer than the hook's own 5 second wait for `git`. Without this the hook
/// gives up, reads an empty slug, and the test fails for reasons unrelated to
/// what it checks. Later runs of the same file take milliseconds.
#[cfg(unix)]
pub fn warm_up(executable: &std::path::Path) {
    let _ = Command::new(executable)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
