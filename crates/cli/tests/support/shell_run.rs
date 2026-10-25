// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The repository root and the text of a finished `install.sh` run.

use std::path::PathBuf;
use std::process::Output;

/// The repository root.
pub fn repo_root() -> PathBuf {
    // The config value ends in "/.", so normalise it for path comparisons.
    let root = PathBuf::from(env!("PLAYBOOK_REPO_ROOT"));
    root.canonicalize().unwrap_or(root)
}

pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Stdout then stderr, as one string.
// The tests that read stdout and stderr apart never join them.
#[allow(dead_code)]
pub fn both(out: &Output) -> String {
    format!("{}{}", stdout(out), stderr(out))
}
