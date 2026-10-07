// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Cheap, bounded git lookups. One probe decides whether `cwd` is a repo; the
//! remaining reads then run concurrently, each under its own timeout.

use crate::common::run_with_timeout;
use std::process::Command;
use std::time::Duration;

/// Upper bound for any single git call, so a wedged repo never stalls a render.
const GIT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct GitInfo {
    /// Current branch, `detached` when HEAD is not on one.
    pub branch: String,
    /// `origin`'s URL, empty when there is none.
    pub remote_url: String,
    pub dirty: bool,
}

fn git(cwd: &str, args: &[&str]) -> Option<std::process::Output> {
    let mut cmd = Command::new("git");
    cmd.args(["--no-optional-locks", "-C", cwd]).args(args);
    run_with_timeout(&mut cmd, GIT_TIMEOUT)
}

fn text(out: Option<std::process::Output>) -> String {
    out.filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim_end_matches('\n')
                .to_string()
        })
        .unwrap_or_default()
}

fn clean(out: Option<std::process::Output>) -> bool {
    out.is_some_and(|o| o.status.success())
}

/// Reads branch, origin URL and dirtiness, or `None` outside a git repo.
pub fn inspect(cwd: &str) -> Option<GitInfo> {
    if !clean(git(cwd, &["rev-parse", "--git-dir"])) {
        return None;
    }
    let (branch, remote_url, unstaged_clean, staged_clean) = std::thread::scope(|s| {
        let branch = s.spawn(|| text(git(cwd, &["branch", "--show-current"])));
        let remote = s.spawn(|| text(git(cwd, &["config", "--get", "remote.origin.url"])));
        let unstaged = s.spawn(|| clean(git(cwd, &["diff", "--quiet"])));
        let staged = s.spawn(|| clean(git(cwd, &["diff", "--cached", "--quiet"])));
        (
            branch.join().unwrap_or_default(),
            remote.join().unwrap_or_default(),
            unstaged.join().unwrap_or(false),
            staged.join().unwrap_or(false),
        )
    });
    Some(GitInfo {
        branch: if branch.is_empty() {
            "detached".to_string()
        } else {
            branch
        },
        remote_url,
        dirty: !(unstaged_clean && staged_clean),
    })
}
