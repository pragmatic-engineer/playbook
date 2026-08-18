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
