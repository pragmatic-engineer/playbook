// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Replaces the message of HEAD with git plumbing, so no hook runs and the
//! index and the working tree are never touched: a new commit object is built
//! on the old commit's tree and parents, and HEAD moves to it only if it still
//! points at the old commit.

use crate::common::attribution::has_sign_off;
use crate::common::proc::{run_with_input, run_with_timeout};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// A signed commit can wait on an agent or a hardware key.
const SIGN_TIMEOUT: Duration = Duration::from_secs(30);
const GIT_TIMEOUT: Duration = Duration::from_secs(5);
/// The reflog line the move of HEAD leaves, which the backstop reads to tell
/// its own rewrite from a commit.
const REFLOG_MESSAGE: &str = "playbook: clean commit message";

/// The commit being replaced.
pub struct Old<'a> {
    pub sha: &'a str,
    pub author: (&'a str, &'a str),
    /// The author is an AI, so the configured identity takes its place. A
    /// person's authorship and the date are always kept.
    pub author_is_ai: bool,
}

/// Replaces `old`, which is HEAD, with a commit that has `message`, plus the
/// configured identity's `Signed-off-by` line when `sign_off` is set and the
/// message has none. `None` when any step fails or HEAD has moved since.
pub fn rewrite_head(dir: &Path, old: &Old, message: &str, sign_off: bool) -> Option<()> {
    let tree = read(dir, &["rev-parse", &format!("{}^{{tree}}", old.sha)])?;
    let parents = read(dir, &["log", "-1", "--format=%P", old.sha])?;
    let date = read(dir, &["log", "-1", "--format=%ad", "--date=raw", old.sha])?;
    let author = if old.author_is_ai {
        identity(dir)?
    } else {
        (old.author.0.to_string(), old.author.1.to_string())
    };
    let message = match sign_off && !has_sign_off(message) {
        true => signed_off(dir, message)?,
        false => message.to_string(),
    };
    let mut commit = git(dir);
    commit.args(["commit-tree", tree.trim()]);
    for parent in parents.split_whitespace() {
        commit.args(["-p", parent]);
    }
    if signs(dir) {
        commit.arg("-S");
    }
    commit
        .args(["-F", "-"])
        .env("GIT_AUTHOR_NAME", &author.0)
        .env("GIT_AUTHOR_EMAIL", &author.1)
        .env("GIT_AUTHOR_DATE", date.trim());
    let out = run_with_input(&mut commit, message.as_bytes(), SIGN_TIMEOUT)
        .filter(|out| out.status.success())?;
    let new = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let mut update = git(dir);
    update.args(["update-ref", "-m", REFLOG_MESSAGE, "HEAD", &new, old.sha]);
    run_with_timeout(&mut update, GIT_TIMEOUT)
        .filter(|out| out.status.success())
        .map(|_| ())
}

/// `git -C dir`.
fn git(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}

fn read(dir: &Path, args: &[&str]) -> Option<String> {
    let mut command = git(dir);
    command.args(args);
    let out = run_with_timeout(&mut command, GIT_TIMEOUT).filter(|out| out.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Whether the repository signs commits: `commit.gpgSign`, as configured.
fn signs(dir: &Path) -> bool {
    read(dir, &["config", "--type=bool", "--get", "commit.gpgSign"])
        .is_some_and(|value| value.trim() == "true")
}

/// The configured committer as `(name, email)`.
fn identity(dir: &Path) -> Option<(String, String)> {
    let ident = read(dir, &["var", "GIT_COMMITTER_IDENT"])?;
    let (name, rest) = ident.split_once(" <")?;
    let (email, _) = rest.split_once('>')?;
    Some((name.trim().to_string(), email.to_string()))
}

/// `message` with the configured identity's `Signed-off-by` trailer, placed
/// the way `git interpret-trailers` places it.
fn signed_off(dir: &Path, message: &str) -> Option<String> {
    let (name, email) = identity(dir)?;
    let trailer = format!("Signed-off-by: {name} <{email}>");
    let mut command = git(dir);
    command.args(["interpret-trailers", "--no-divider", "--trailer", &trailer]);
    let placed = run_with_input(&mut command, message.as_bytes(), GIT_TIMEOUT)
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned());
    Some(placed.unwrap_or_else(|| format!("{}\n\n{trailer}\n", message.trim_end())))
}
