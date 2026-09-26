// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Classifies a worktree by matching its path against playbook's known
//! creation conventions (`playbook wu`, the Agent tool, `cc worktree`, and
//! review worktrees), then decides whether an already-classified worktree is
//! safe to reap.

use crate::common::paths::playbook_root_from;
use crate::common::repo_slug;
use crate::common::run_with_timeout;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Which convention created a worktree, matched purely by its path shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Convention {
    Wu,
    AgentTool,
    CcLauncher,
    Review,
    Unmanaged,
}

// Wider than a plain 5s: this crate's fully parallel tests flake at 5s under
// load, the same reason `src/common/paths.rs` widened its own git timeout.
const GIT_TIMEOUT: Duration = Duration::from_secs(15);

/// Classifies `worktree_path` against each known creation convention's path
/// shape, resolved relative to `home`.
pub fn classify(worktree_path: &Path, home: &Path) -> Convention {
    if is_wu_convention(worktree_path, home) {
        Convention::Wu
    } else if is_agent_tool_convention(worktree_path) {
        Convention::AgentTool
    } else if is_cc_launcher_convention(worktree_path) {
        Convention::CcLauncher
    } else if is_review_convention(worktree_path) {
        Convention::Review
    } else {
        Convention::Unmanaged
    }
}

/// `playbook_root_from(home)/repos/<owner>/<repo>/*/worktrees/*/*`, where the
/// owner/repo prefix comes from `repo_slug()`, never from `worktree_id()`:
/// the worktree-id segment differs between two worktrees of the same repo, so
/// a literal compare would miss every worktree but the one that created it.
fn is_wu_convention(path: &Path, home: &Path) -> bool {
    let slug = repo_slug();
    let Some((owner, repo)) = slug.split_once('/') else {
        return false;
    };
    let prefix = playbook_root_from(home)
        .join("repos")
        .join(owner)
        .join(repo);
    let Ok(rest) = path.strip_prefix(&prefix) else {
        return false;
    };
    let segments: Vec<_> = rest.components().collect();
    segments.len() == 4 && segments[1].as_os_str() == "worktrees"
}

/// `.claude/worktrees/agent-*` anywhere in the path, matching the Agent
/// tool's launch convention regardless of which repo root it sits under.
fn is_agent_tool_convention(path: &Path) -> bool {
    let segments: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    segments
        .windows(3)
        .any(|w| w[0] == ".claude" && w[1] == "worktrees" && w[2].starts_with("agent-"))
}

/// `<main-worktree-parent>/.worktrees/<repo>/<branch>`, the default
/// `_wt_resolve_base` shape. Derived from the MAIN worktree entry of `git
/// worktree list --porcelain`, never the calling process's own toplevel: a
/// sweep invoked from inside the just-created worktree would otherwise
/// resolve its own parent instead of the launcher's base directory.
fn is_cc_launcher_convention(path: &Path) -> bool {
    let Some(porcelain) = git_stdout(&["worktree", "list", "--porcelain"]) else {
        return false;
    };
    let Some(main) = crate::cc::worktree::main_worktree(&porcelain) else {
        return false;
    };
    let main_root = PathBuf::from(main);
    let Some(parent) = main_root.parent() else {
        return false;
    };
    let base = crate::cc::worktree::resolve_base(&main_root, parent, None);
    is_direct_child(path, &base)
}

/// `review-worktrees/<pr>-<sha>` under the git common dir, per
/// `shell/review-worktree.sh`'s `cmd_setup`.
fn is_review_convention(path: &Path) -> bool {
    let Some(common_dir) = git_stdout(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
    else {
        return false;
    };
    let base = PathBuf::from(common_dir).join("review-worktrees");
    is_direct_child(path, &base)
}

/// True if `path` is exactly one path component below `base`.
fn is_direct_child(path: &Path, base: &Path) -> bool {
    matches!(path.strip_prefix(base), Ok(rest) if rest.components().count() == 1)
}

/// Compares `worktree_head_commit` against `origin/<default_branch>` to
/// decide whether the WU worktree's work already reached the default branch.
pub fn wu_worktree_landed(
    worktree_head_commit: &str,
    default_branch: &str,
    repo_root: &Path,
) -> bool {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo_root).args([
        "merge-base",
        "--is-ancestor",
        worktree_head_commit,
        &format!("origin/{default_branch}"),
    ]);
    matches!(run_with_timeout(&mut command, GIT_TIMEOUT), Some(out) if out.status.success())
}

/// Whether a branch's pull request is merged or open, the richer signal
/// [`named_branch_landed`] needs beyond a flat open-or-not list. `None`
/// stands for no PR found (or `gh` unavailable), which falls back to local
/// git history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Merged,
    Open,
}

/// Seconds in a day, for converting `stale_after_days` into the age
/// comparison [`named_branch_landed`] runs against a commit timestamp.
const SECS_PER_DAY: i64 = 86_400;

/// Whether a named-branch worktree's branch already landed: true for a
/// merged PR, false for an open one, and otherwise a local fallback: merged
/// into `default_branch` or stale past `stale_after_days`, with the
/// threshold itself counting as stale.
pub fn named_branch_landed(
    branch: &str,
    default_branch: &str,
    stale_after_days: i64,
    repo_root: &Path,
    now_epoch: i64,
    pr_state: Option<PrState>,
) -> bool {
    match pr_state {
        Some(PrState::Merged) => return true,
        Some(PrState::Open) => return false,
        None => {}
    }

    let merged = crate::cc::worktree::merged_branches(repo_root, default_branch);
    if crate::cc::worktree::contains_line(&merged, branch) {
        return true;
    }

    let Some(commit_epoch) = branch_commit_epoch(branch, repo_root) else {
        return false;
    };
    now_epoch - commit_epoch >= stale_after_days * SECS_PER_DAY
}

/// Unix seconds of `branch`'s last commit, or `None` if the branch or its
/// history cannot be read.
fn branch_commit_epoch(branch: &str, repo_root: &Path) -> Option<i64> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo_root)
        .args(["log", "-1", "--format=%ct", branch]);
    let out = run_with_timeout(&mut command, GIT_TIMEOUT)?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .and_then(|s| s.parse().ok())
}

/// Whether a review-convention worktree already landed, judged by its lock
/// state rather than git history: a review worktree carries no branch or PR
/// to query, only a lock file `git worktree add`/`lock` may or may not have
/// reached yet.
pub fn review_worktree_landed(
    is_locked: bool,
    lock_owner_pid: Option<u32>,
    lock_age_secs: i64,
    never_locked_grace_secs: i64,
) -> bool {
    if !is_locked {
        return lock_age_secs >= never_locked_grace_secs;
    }
    match lock_owner_pid {
        Some(pid) => !pid_is_alive(pid),
        None => false,
    }
}

/// Whether `pid` still refers to a running process, checked via `kill -0`
/// rather than a signal-handling crate dependency (none already in this
/// crate covers it). Any non-success exit, including permission denied,
/// reads as not alive. Runs directly rather than through
/// [`run_with_timeout`], unlike this module's git and `gh` calls: a local
/// syscall carries no hang risk to guard against.
pub fn pid_is_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .is_ok_and(|status| status.success())
}

/// Extracts a pid from a lock reason string, trying `review-worktree.sh`'s
/// flat `pid=<n>` shape first, then the Agent tool's `pid <n>` shape. `None`
/// if neither is found: the Agent-tool format is not a documented contract
/// anywhere in this repo, so it is never assumed to be stable.
pub fn parse_lock_pid(lock_reason: &str) -> Option<u32> {
    parse_pid_after(lock_reason, "pid=").or_else(|| parse_pid_after(lock_reason, "pid "))
}

/// Digits immediately following the first occurrence of `marker`, or `None`
/// if `marker` is absent or is not followed by at least one digit.
fn parse_pid_after(lock_reason: &str, marker: &str) -> Option<u32> {
    let after_marker = &lock_reason[lock_reason.find(marker)? + marker.len()..];
    after_marker
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

/// `git <args>` against the CALLING process's current directory, trimmed.
/// `None` on any failure, including a timeout. Mirrors `src/cc/worktree.rs`'s
/// `git_stdout`, but without a `-C <repo_root>`: `classify` has no repo root
/// of its own, only the caller's cwd.
fn git_stdout(args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.args(args);
    let out = run_with_timeout(&mut command, GIT_TIMEOUT)?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
