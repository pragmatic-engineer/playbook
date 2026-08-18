// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports the decision and path logic of `shell/shared/worktree.sh`.
//!
//! First slice of WU-17. The git-driving half (worktree creation, upstream
//! setup, stale cleanup, rebase recovery) and the orchestration follow
//! separately, because 496 lines of shell across 86 git invocations does not
//! fit one reviewable change.

use crate::common::run_with_timeout;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Never collapse worktrees into the repo root, which `WORKTREE_BASE_DIR="."`
/// would otherwise do.
const DEFAULT_BASE_DIR: &str = ".worktrees";

#[derive(Debug, PartialEq)]
pub enum ConflictAction {
    Spawn,
    Abort,
}

/// What to do when a rebase conflicts and AI-resolve is available.
///
/// Silent mode spawns without asking. Otherwise only an interactive terminal
/// may consent, and a non-tty aborts rather than assuming yes: the alternative
/// is an unattended run rewriting someone's conflicts.
pub fn conflict_action(silent: bool, is_tty: bool, answer: &str) -> ConflictAction {
    if silent {
        return ConflictAction::Spawn;
    }
    if !is_tty {
        return ConflictAction::Abort;
    }
    // Empty means the user pressed return on a default-yes prompt.
    match answer.trim().to_ascii_lowercase().as_str() {
        "" | "y" | "yes" => ConflictAction::Spawn,
        _ => ConflictAction::Abort,
    }
}

/// The directory holding this repo's worktrees: `<base>/<repo>`.
///
/// A relative base sits under the repo's parent, an absolute one is used as is.
/// The `<repo>` leaf is what stops same-named branches in sibling repos from
/// colliding.
pub fn resolve_base(repo_root: &Path, repo_parent: &Path, configured: Option<&str>) -> PathBuf {
    let repo_name = repo_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let base = match configured {
        Some(b) if !b.is_empty() && b != "." => b,
        _ => DEFAULT_BASE_DIR,
    };

    if base.starts_with('/') {
        Path::new(base).join(repo_name)
    } else {
        repo_parent.join(base).join(repo_name)
    }
}

/// Where the `.env` lives, relative to the repo root, or `None`.
///
/// `Some(".")` means the repo root itself. Without an explicit hint it looks
/// one level down, matching the shell's `find -mindepth 2 -maxdepth 2`.
pub fn find_env_base(repo_root: &Path, hint: Option<&str>) -> Option<String> {
    if let Some(hint) = hint.filter(|h| !h.is_empty()) {
        if hint == "." && repo_root.join(".env").is_file() {
            return Some(".".to_string());
        }
        if repo_root.join(hint).join(".env").is_file() {
            return Some(hint.to_string());
        }
        return None;
    }

    if repo_root.join(".env").is_file() {
        return Some(".".to_string());
    }

    // One level down only, and sorted so the answer does not depend on
    // readdir order the way the shell's `head -n1` did.
    let mut candidates: Vec<String> = std::fs::read_dir(repo_root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join(".env").is_file())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

#[derive(Debug, PartialEq)]
pub enum EnvCopy {
    Copied(PathBuf),
    NoEnvConfigured,
    SourceMissing,
    /// The reason this guard exists. Reported, never silent.
    RefusedNotGitignored(String),
}

/// Copies the `.env` into a new worktree, but ONLY when it is gitignored in the
/// source repo.
///
/// This is a secret-leak control, not a convenience. A tracked `.env` copied
/// into a fresh worktree is a file `git add` will happily stage, so an
/// unignored one is refused and the refusal is reported. No-clobber: an
/// existing destination file is left alone.
pub fn copy_env(repo_root: &Path, dest: &Path, env_base: Option<&str>) -> EnvCopy {
    let Some(env_base) = env_base.filter(|b| !b.is_empty()) else {
        return EnvCopy::NoEnvConfigured;
    };
    let rel = if env_base == "." {
        ".env".to_string()
    } else {
        format!("{env_base}/.env")
    };
    let src = repo_root.join(&rel);
    if !src.is_file() {
        return EnvCopy::SourceMissing;
    }
    if !is_gitignored(repo_root, &rel) {
        return EnvCopy::RefusedNotGitignored(rel);
    }

    let target = dest.join(&rel);
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if target.exists() {
        // cp -n: an existing file is never overwritten.
        return EnvCopy::Copied(target);
    }
    match std::fs::copy(&src, &target) {
        Ok(_) => EnvCopy::Copied(target),
        Err(_) => EnvCopy::SourceMissing,
    }
}

/// `git check-ignore -q`, where exit 0 means the path is ignored.
///
/// A git failure or timeout reads as NOT ignored, which refuses the copy. That
/// direction is deliberate: the failure mode of guessing wrong here is staging
/// a secret.
fn is_gitignored(repo_root: &Path, rel: &str) -> bool {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo_root)
        .args(["check-ignore", "-q", rel]);
    matches!(run_with_timeout(&mut command, GIT_TIMEOUT), Some(o) if o.status.success())
}

/// Branches tried when the remote publishes no `origin/HEAD`, in order.
const BASE_BRANCH_CANDIDATES: [&str; 4] = ["main", "master", "trunk", "develop"];

/// Last resort when the remote is unreachable and no candidate exists locally.
const BASE_BRANCH_FALLBACK: &str = "master";

/// A worktree is stale once its last commit is older than this.
const STALE_AFTER_DAYS: i64 = 30;

const SECS_PER_DAY: i64 = 86_400;

/// Cleanup runs at most this often per repo. Kept as its own name so callers
/// reference the rate limit rather than SECS_PER_DAY directly: retuning this
/// must not move the staleness cutoff, which is a separate decision that
/// happens to share the same number today.
const CLEANUP_INTERVAL_SECS: i64 = SECS_PER_DAY;

/// The base branch as a remote-tracking ref, for example `origin/main`.
///
/// Prefers what the remote itself publishes as `origin/HEAD`, then the common
/// names, so a repo whose default is `trunk` is not silently rebased onto a
/// `main` that does not exist.
pub fn base_branch(repo_root: &Path) -> String {
    let published = git_stdout(
        repo_root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .map(|s| s.trim().trim_start_matches("origin/").to_string())
    .filter(|s| !s.is_empty());

    if let Some(name) = published {
        return format!("origin/{name}");
    }

    for candidate in BASE_BRANCH_CANDIDATES {
        let reference = format!("refs/remotes/origin/{candidate}");
        if git_ok(repo_root, &["show-ref", "--verify", "--quiet", &reference]) {
            return format!("origin/{candidate}");
        }
    }
    format!("origin/{BASE_BRANCH_FALLBACK}")
}

/// Everything the staleness decision needs, gathered by the caller so the
/// decision itself stays pure and exhaustively testable.
pub struct WorktreeStatus<'a> {
    pub path: &'a Path,
    pub branch: &'a str,
    /// The worktree this run is creating, which must never be reaped.
    pub is_target: bool,
    /// Something is cwd'd into it.
    pub in_use: bool,
    pub has_open_pr: bool,
    pub merged_into_base: bool,
    /// Unix seconds of the last commit.
    pub last_commit_epoch: i64,
}

/// Whether a worktree may be removed.
///
/// The four skips come first and are absolute: the target of this run, anything
/// in use, a branch with an open pull request, and a detached or unreadable
/// HEAD. Only then does merged-or-old decide. Ordering matters, since an old
/// branch with an open PR is still wanted.
pub fn is_stale(status: &WorktreeStatus, now_epoch: i64) -> bool {
    if status.is_target || status.in_use || status.has_open_pr || status.branch.is_empty() {
        return false;
    }
    if status.merged_into_base {
        return true;
    }
    let cutoff = now_epoch - (STALE_AFTER_DAYS * SECS_PER_DAY);
    status.last_commit_epoch < cutoff
}

/// Whether the daily cleanup is due, given the marker's mtime.
///
/// `None` means the marker is absent, which counts as due: a repo that has
/// never been cleaned should be.
pub fn cleanup_due(marker_mtime_epoch: Option<i64>, now_epoch: i64) -> bool {
    match marker_mtime_epoch {
        None => true,
        Some(stamped) => now_epoch - stamped >= CLEANUP_INTERVAL_SECS,
    }
}

fn git_stdout(repo_root: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo_root).args(args);
    let out = run_with_timeout(&mut command, GIT_TIMEOUT)?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

fn git_ok(repo_root: &Path, args: &[&str]) -> bool {
    git_stdout(repo_root, args).is_some()
}

/// Whether the new worktree can reuse the main checkout's `node_modules`.
///
/// Requires a `package.json` in the worktree, and a lockfile plus installed
/// modules in the source, then that the two lockfiles are byte-identical.
///
/// The shell hashed both lockfiles with sha256sum or shasum and compared the
/// digests, which also meant skipping the reuse entirely when neither tool was
/// present. Comparing the bytes answers the same question, is strictly
/// stronger than a digest, and drops both the external dependency and that
/// silent skip.
pub fn node_modules_reusable(worktree: &Path, repo_root: &Path) -> bool {
    if !worktree.join("package.json").is_file() {
        return false;
    }
    let source_lock = repo_root.join("package-lock.json");
    if !source_lock.is_file() || !repo_root.join("node_modules").is_dir() {
        return false;
    }

    // A worktree without its own lockfile inherits the source's, which is the
    // copy the shell made before comparing, so that case matches by definition.
    let worktree_lock = worktree.join("package-lock.json");
    if !worktree_lock.is_file() {
        return true;
    }
    match (std::fs::read(&source_lock), std::fs::read(&worktree_lock)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The worktree path already checked out on `branch`, from
/// `git worktree list --porcelain`.
///
/// The recovery path when `git worktree add` fails: a branch can only live in
/// one worktree, so the usual cause is that it is already checked out
/// somewhere, and reporting that beats a bare failure.
pub fn worktree_for_branch(porcelain: &str, branch: &str) -> Option<String> {
    let wanted = format!("refs/heads/{branch}");
    let mut current: Option<&str> = None;
    for line in porcelain.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current = Some(path);
        } else if let Some(found) = line.strip_prefix("branch ") {
            if found == wanted {
                return current.map(|p| p.to_string());
            }
        }
    }
    None
}

/// What the launcher found at the target path, gathered by the caller so the
/// decision stays pure.
pub struct TargetState<'a> {
    pub target_exists: bool,
    /// Registered with `git worktree list`, as opposed to a stray directory.
    pub registered: bool,
    /// `None` means detached or unreadable HEAD.
    pub current_branch: Option<&'a str>,
    pub wanted_branch: &'a str,
    /// Whether the branch currently checked out there still exists on the remote.
    pub current_branch_on_remote: bool,
    /// A different worktree already holding the wanted branch, and whether its
    /// directory is still present.
    pub existing_for_wanted: Option<(&'a str, bool)>,
}

#[derive(Debug, PartialEq)]
pub enum WorktreePlan {
    /// Detached HEAD in a registered worktree: abort any rebase or merge and
    /// check the branch back out. The caller re-evaluates afterwards, since
    /// recovery can fail and fall through to a rebuild.
    RecoverDetached,
    /// A directory git does not know about. Prune, remove, recreate.
    CleanOrphanAndCreate,
    /// Already on the wanted branch: fetch and fast-forward in place.
    ReuseTarget,
    /// The occupying branch still exists on the remote, so it is unfinished
    /// work. Refuse rather than destroy it.
    RefuseOccupied(String),
    /// The occupying branch is gone from the remote, so it was merged or
    /// deleted and the worktree can be recycled.
    RecycleTarget(String),
    /// The wanted branch is checked out in another worktree that still exists.
    ReuseExisting(String),
    /// Git has a worktree registered for the branch but the directory is gone.
    PruneStaleAndCreate,
    Create,
}

/// Chooses the recovery path for the target worktree.
///
/// The ordering is the whole point and mirrors the shell. In particular
/// `RefuseOccupied` comes before `RecycleTarget`: recycling removes the
/// worktree and deletes its branch, so a branch that still exists on the remote
/// must stop the operation rather than be destroyed. Getting that pair the
/// wrong way round would silently discard unfinished work.
pub fn plan_for_target(state: &TargetState) -> WorktreePlan {
    if state.target_exists {
        return match (state.current_branch, state.registered) {
            (None, true) => WorktreePlan::RecoverDetached,
            (None, false) => WorktreePlan::CleanOrphanAndCreate,
            (Some(current), _) if current == state.wanted_branch => WorktreePlan::ReuseTarget,
            (Some(current), _) if state.current_branch_on_remote => {
                WorktreePlan::RefuseOccupied(current.to_string())
            }
            (Some(current), _) => WorktreePlan::RecycleTarget(current.to_string()),
        };
    }

    match state.existing_for_wanted {
        Some((path, true)) => WorktreePlan::ReuseExisting(path.to_string()),
        Some((_, false)) => WorktreePlan::PruneStaleAndCreate,
        None => WorktreePlan::Create,
    }
}
