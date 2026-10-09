// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook commit`: the git work of `/playbook:commit-and-push`. `prepare`
//! stages, formats and prints what the message drafter needs. `run` commits
//! with a sign-off and signing decision, rebases when behind, and pushes
//! without ever escalating a rejected push to a force.

pub mod prepare;
pub mod run;

use std::path::Path;
use std::process::{Command, Output};

/// Runs `git` in `dir`.
pub(crate) fn git_in(dir: &Path, args: &[&str]) -> Option<Output> {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()
}

/// Trimmed stdout of a git call that succeeded, else `None`.
pub(crate) fn git_text(dir: &Path, args: &[&str]) -> Option<String> {
    let out = git_in(dir, args)?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub(crate) fn git_success(dir: &Path, args: &[&str]) -> bool {
    git_in(dir, args).is_some_and(|o| o.status.success())
}

/// The current branch name, or `HEAD` when detached.
pub(crate) fn current_branch(dir: &Path) -> String {
    if let Some(name) = crate::common::gitfacts::abbrev_head(dir) {
        return name;
    }
    git_text(dir, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default()
}
