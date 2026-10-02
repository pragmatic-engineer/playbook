// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Pieces `pr prepare` and `pr create` share: the scratch directory both
//! read and write, and the `gh` seam both call through.

use crate::common::paths::{repo_scoped_dir, RepoScope};
use crate::common::proc::run_with_timeout;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// How long a `gh` call may take; PR creation talks to the network.
const GH_TIMEOUT: Duration = Duration::from_secs(60);

/// Every non-alphanumeric character becomes `-`, the same rule
/// `common::paths::worktree_id` uses for its own path segment.
fn branch_slug(branch: &str) -> String {
    branch
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The scratch directory for `branch`'s PR files, under this worktree's
/// repo-scoped storage. Both subcommands resolve it here so they cannot
/// disagree on where the diff and body live.
pub fn pr_state_dir(branch: &str) -> Result<PathBuf, String> {
    let base = repo_scoped_dir(RepoScope::Worktree).ok_or_else(|| {
        "could not resolve a worktree-scoped storage location; this repo needs a git \
         checkout with an `origin` remote"
            .to_string()
    })?;
    Ok(base.join("pr").join(branch_slug(branch)))
}

/// How long a `git` call that talks to the network may take.
const NET_TIMEOUT: Duration = Duration::from_secs(120);

/// Runs a local `git` command and returns its trimmed stdout. Local commands
/// are read with `output()` (not `run_with_timeout`) because that helper
/// never drains the pipes, so a large `git diff` would stall it.
pub(crate) fn git(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run git {}: {e}", args.join(" ")))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Runs a `git` command that talks to the network, bounded by `NET_TIMEOUT`.
pub(crate) fn git_net(args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("git");
    command.args(args);
    let output = run_with_timeout(&mut command, NET_TIMEOUT)
        .ok_or_else(|| format!("git {} did not finish in time", args.join(" ")))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Runs a local `git` command with stdout going straight to `path`, so a
/// diff of any size never passes through memory or a pipe.
pub(crate) fn git_to_file(args: &[&str], path: &Path) -> Result<(), String> {
    let file =
        fs::File::create(path).map_err(|e| format!("could not create {}: {e}", path.display()))?;
    let status = Command::new("git")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(file))
        .status()
        .map_err(|e| format!("could not run git {}: {e}", args.join(" ")))?;
    if !status.success() {
        return Err(format!("git {} failed", args.join(" ")));
    }
    Ok(())
}

/// Resolves the PR's base branch and says where it came from: the flag,
/// then the repo's default branch, then `origin/HEAD`, then `main`.
pub fn resolve_base(
    gh: &dyn GhClient,
    base_arg: Option<&str>,
) -> Result<(String, &'static str), String> {
    if let Some(base) = base_arg.filter(|b| !b.is_empty()) {
        return Ok((base.to_string(), "--base flag"));
    }
    if let Some(base) = gh.repo_default_branch()?.filter(|b| !b.is_empty()) {
        return Ok((base, "repo default"));
    }
    if let Ok(reference) = git(&["symbolic-ref", "refs/remotes/origin/HEAD"]) {
        if let Some(base) = reference.strip_prefix("refs/remotes/origin/") {
            if !base.is_empty() {
                return Ok((base.to_string(), "git symbolic-ref"));
            }
        }
    }
    Ok(("main".to_string(), "fallback"))
}

/// An open or closed PR already attached to a branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingPr {
    pub url: String,
    pub state: String,
}

/// The `gh` operations the PR subcommands need, one method per operation so
/// a test fake declares exactly which ones it touches.
pub trait GhClient {
    fn pr_view(&self, branch: &str) -> Result<Option<ExistingPr>, String>;
    fn repo_default_branch(&self) -> Result<Option<String>, String>;
    /// Returns the new PR's URL.
    fn pr_create(&self, title: &str, body_file: &str, base: &str) -> Result<String, String>;
    /// Returns the PR's `baseRefName`.
    fn pr_view_base(&self, branch: &str) -> Result<String, String>;
    fn pr_edit_base(&self, branch: &str, base: &str) -> Result<(), String>;
}

/// Shells out to the real `gh` binary.
pub struct RealGhClient;

/// Runs `gh` with `args`; `Ok` is trimmed stdout, `Err` the trimmed stderr.
fn gh(args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("gh");
    command.args(args);
    let output = run_with_timeout(&mut command, GH_TIMEOUT).ok_or_else(|| {
        format!(
            "gh {} did not finish (missing, or timed out)",
            args.join(" ")
        )
    })?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

impl GhClient for RealGhClient {
    fn pr_view(&self, branch: &str) -> Result<Option<ExistingPr>, String> {
        // A branch with no PR makes `gh pr view` exit non-zero, which is the
        // normal "nothing there yet" answer, not a failure.
        let Ok(out) = gh(&["pr", "view", branch, "--json", "url,state"]) else {
            return Ok(None);
        };
        let value: serde_json::Value =
            serde_json::from_str(&out).map_err(|e| format!("gh pr view returned bad JSON: {e}"))?;
        let field = |name: &str| value.get(name).and_then(|v| v.as_str()).map(str::to_string);
        match (field("url"), field("state")) {
            (Some(url), Some(state)) => Ok(Some(ExistingPr { url, state })),
            _ => Ok(None),
        }
    }

    fn repo_default_branch(&self) -> Result<Option<String>, String> {
        let out = gh(&[
            "repo",
            "view",
            "--json",
            "defaultBranchRef",
            "-q",
            ".defaultBranchRef.name",
        ]);
        Ok(out.ok().filter(|name| !name.is_empty()))
    }

    fn pr_create(&self, title: &str, body_file: &str, base: &str) -> Result<String, String> {
        gh(&[
            "pr",
            "create",
            "--title",
            title,
            "--body-file",
            body_file,
            "--base",
            base,
            "--draft",
        ])
        .map_err(|e| format!("gh pr create failed: {e}"))
    }

    fn pr_view_base(&self, branch: &str) -> Result<String, String> {
        gh(&[
            "pr",
            "view",
            branch,
            "--json",
            "baseRefName",
            "-q",
            ".baseRefName",
        ])
        .map_err(|e| format!("gh pr view failed: {e}"))
    }

    fn pr_edit_base(&self, branch: &str, base: &str) -> Result<(), String> {
        gh(&["pr", "edit", branch, "--base", base])
            .map(|_| ())
            .map_err(|e| format!("gh pr edit failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CannedGh;

    impl GhClient for CannedGh {
        fn pr_view(&self, _branch: &str) -> Result<Option<ExistingPr>, String> {
            Ok(Some(ExistingPr {
                url: "https://example.test/pr/7".to_string(),
                state: "OPEN".to_string(),
            }))
        }
        fn repo_default_branch(&self) -> Result<Option<String>, String> {
            Ok(Some("develop".to_string()))
        }
        fn pr_create(&self, _t: &str, _b: &str, _base: &str) -> Result<String, String> {
            Ok("https://example.test/pr/8".to_string())
        }
        fn pr_view_base(&self, _branch: &str) -> Result<String, String> {
            Ok("main".to_string())
        }
        fn pr_edit_base(&self, _branch: &str, _base: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn a_trait_object_returns_exactly_the_canned_pr_view_response() {
        // Arrange
        let gh: &dyn GhClient = &CannedGh;

        // Act
        let got = gh.pr_view("feat/x").expect("canned response");

        // Assert
        assert_eq!(
            got,
            Some(ExistingPr {
                url: "https://example.test/pr/7".to_string(),
                state: "OPEN".to_string(),
            })
        );
    }
}
