// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Shared access to `hooks/lib/config-hash.sh`.
//!
//! That script stays shell and is one of the few files WU-14 keeps, because
//! both the hooks and the launcher need the same hash and it is the single
//! source of truth for how it is computed. Reimplementing it here would mean
//! two definitions of "did the config change", plus a sha256 dependency.
//!
//! The two callers pass different roots: hooks resolve it under
//! `CLAUDE_PLUGIN_ROOT`, the launcher under `~/.config/playbook`.

use crate::common::run_with_timeout;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Sources the script and returns `config_hash`'s trimmed stdout, or an empty
/// string on any failure: missing root, missing bash, timeout, non-zero exit.
///
/// An empty string is the "unknown" value both callers already handle, so this
/// never panics and never blocks a session on a hashing problem.
pub fn config_hash(root: &Path, timeout: Duration) -> String {
    if root.as_os_str().is_empty() {
        return String::new();
    }
    let script = root.join("hooks").join("lib").join("config-hash.sh");
    let mut command = Command::new("bash");
    command
        .arg("-c")
        .arg(". \"$1\"; config_hash")
        .arg("_")
        .arg(&script);
    match run_with_timeout(&mut command, timeout) {
        Some(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    }
}
