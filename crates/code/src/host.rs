// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The bridge from the code index to the playbook program. What the index
//! needs from playbook (its name, where it keeps state, repo identity, file
//! privacy and, in later Segments, config values and process policy) goes
//! through this module. Shared helpers come from `playbook_common`.

use std::path::{Path, PathBuf};

/// The program name used in user facing messages.
pub const PROGRAM: &str = "playbook";

/// This tool's version, stored in the index metadata.
pub fn tool_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The state folder for the current worktree, where the index cache lives
/// (`code/index.sqlite3` goes inside it). Fails with an actionable message
/// when the repo has no way to name itself.
pub fn state_dir() -> Result<PathBuf, String> {
    playbook_common::paths::repo_scoped_dir(playbook_common::paths::RepoScope::Worktree).ok_or_else(
        || {
            "could not resolve where to keep the code index; this repo needs a git 'origin' \
             remote and a resolvable worktree root"
                .to_string()
        },
    )
}

/// A stable identity for the current worktree.
pub fn worktree_id() -> String {
    playbook_common::paths::worktree_id()
}

/// Creates `dir` and its parents, owner only on unix.
pub fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    playbook_common::atomic::ensure_private_dir(dir)
}

/// Creates the file owner only on unix before SQLite opens it, and tightens a
/// wider existing file.
pub fn make_file_private(path: &Path) -> Result<(), String> {
    playbook_common::atomic::make_db_file_private(path)
}

/// The command line a user runs for `args`, for error messages.
pub fn command(args: &str) -> String {
    format!("{PROGRAM} code {args}")
}
