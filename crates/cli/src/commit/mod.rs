// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook commit`: the git work of `/playbook:commit-and-push`. `prepare`
//! stages, formats and prints what the message drafter needs. `run` commits
//! with a sign-off and signing decision, rebases when behind, and pushes
//! without ever escalating a rejected push to a force.

pub mod prepare;
pub mod run;
pub mod typecheck;

use crate::common::proc::run_with_timeout;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

/// Longest a local read or index write (`add -A` on a big tree) may take.
const GIT_TIMEOUT: Duration = Duration::from_secs(120);
/// Longest a commit (with its hooks and signing), fetch, rebase or push may
/// take. Bounded so a stuck network or credential prompt cannot hang forever.
const GIT_LONG_TIMEOUT: Duration = Duration::from_secs(600);

fn git_with(dir: &Path, args: &[&str], timeout: Duration) -> Option<Output> {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0");
    run_with_timeout(&mut command, timeout)
}

/// The marker that opens the abort message of [`guard_branch`]; the skill
/// docs and the tests match on it.
pub const WRONG_DIRECTORY: &str = "WRONG_DIRECTORY";

/// Stops a run that landed on the wrong branch. A forked command can start in
/// the session checkout instead of the worktree its caller works in, and then
/// it commits or opens a PR for the wrong branch. `expected` is the branch the
/// caller reported; `None` skips the check.
pub fn guard_branch(dir: &Path, expected: Option<&str>) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = current_branch(dir);
    if actual == expected {
        return Ok(());
    }
    Err(format!(
        "{WRONG_DIRECTORY}: the caller reported branch '{expected}' but {} is on '{actual}'. Nothing was changed. Pass the worktree root with --dir (git rev-parse --show-toplevel in the caller) and run again.",
        dir.display()
    ))
}

/// Runs `git` in `dir`, giving up after [`GIT_TIMEOUT`].
pub(crate) fn git_in(dir: &Path, args: &[&str]) -> Option<Output> {
    git_with(dir, args, GIT_TIMEOUT)
}

/// Like [`git_in`] for commit, fetch, rebase and push, which can run hooks or
/// wait on the network.
pub(crate) fn git_in_long(dir: &Path, args: &[&str]) -> Option<Output> {
    git_with(dir, args, GIT_LONG_TIMEOUT)
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
