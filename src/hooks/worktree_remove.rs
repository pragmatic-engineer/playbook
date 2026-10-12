// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! WorktreeRemove hook: removes a worktree only when nothing would be lost.
//! Dirty trees and unpublished commits keep the worktree; it never forces.

use super::worktree_create::{git, registered_paths, same_path, BRANCH_PREFIX};
use crate::cc::worktree::main_worktree;
use crate::common::Payload;
use std::path::{Path, PathBuf};

/// Entry point: always exits 0, since a kept worktree is not an error.
pub fn run(payload: &Payload) {
    match remove(payload) {
        Ok(msg) => eprintln!("playbook worktree-remove: {msg}"),
        Err(why) => eprintln!("playbook worktree-remove: kept worktree: {why}"),
    }
}

fn remove(payload: &Payload) -> Result<String, String> {
    let path = PathBuf::from(payload.field(".worktree_path"));
    if !path.is_absolute() {
        return Err("worktree_path is missing or not absolute".to_string());
    }
    let cwd = PathBuf::from(payload.field(".cwd"));
    let probe = if cwd.is_dir() {
        cwd.as_path()
    } else {
        path.as_path()
    };
    let porcelain = git(probe, &["worktree", "list", "--porcelain"])?;
    let main_root = PathBuf::from(main_worktree(&porcelain).ok_or("no main worktree found")?);

    if same_path(&path, &main_root) {
        return Err("refusing to remove the main worktree".to_string());
    }
    if !registered_paths(&porcelain)
        .iter()
        .any(|p| same_path(p, &path))
    {
        return Err(format!("{} is not a worktree of this repo", path.display()));
    }

    let status = git(&path, &["status", "--porcelain", "--untracked-files=all"])?;
    if !status.trim().is_empty() {
        return Err("it has uncommitted or untracked changes".to_string());
    }
    let default = default_branch(&main_root);
    if !fully_published(&path, "HEAD", &default)? {
        return Err(format!(
            "it has commits not on any remote or the main branch; to discard it yourself run `git worktree remove --force '{}'`",
            path.display()
        ));
    }
    let branch = git(&path, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let path_s = path.to_string_lossy().to_string();
    git(&main_root, &["worktree", "remove", &path_s])?;

    let owned = branch.starts_with(BRANCH_PREFIX) && branch != default;
    if owned {
        let qualified = format!("refs/heads/{branch}");
        match fully_published(&main_root, &qualified, &default) {
            Ok(true) => {
                let _ = git(&main_root, &["branch", "-D", "--", &branch]);
            }
            _ => eprintln!("playbook worktree-remove: kept branch {branch}"),
        }
    }
    Ok(format!("removed {}", path.display()))
}

/// True when every commit of `rev` is reachable from a remote-tracking ref or
/// the local default branch.
fn fully_published(dir: &Path, rev: &str, default: &str) -> Result<bool, String> {
    let local_default = format!("refs/heads/{default}");
    let has_default = git(dir, &["show-ref", "--verify", "--quiet", &local_default]).is_ok();
    let mut args = vec!["rev-list", "--count", rev, "--not", "--remotes"];
    if has_default {
        args.push(&local_default);
    }
    let count = git(dir, &args)?;
    if count.trim() == "0" {
        return Ok(true);
    }
    Ok(squash_merged(dir, rev, default))
}

/// True when merging `rev` into the default branch changes nothing while `rev`
/// does carry a net change, which is how a squash-merged branch looks once its
/// remote branch is gone. A branch with no net change (a commit and its revert)
/// is not counted, since its commits would be lost.
fn squash_merged(dir: &Path, rev: &str, default: &str) -> bool {
    let targets = [
        format!("refs/remotes/origin/{default}"),
        format!("refs/heads/{default}"),
    ];
    targets.iter().any(|target| {
        if git(dir, &["show-ref", "--verify", "--quiet", target]).is_err() {
            return false;
        }
        let Ok(base) = git(dir, &["merge-base", target, rev]) else {
            return false;
        };
        if git(dir, &["diff", "--quiet", base.trim(), rev]).is_ok() {
            return false;
        }
        let Ok(merged) = git(dir, &["merge-tree", "--write-tree", target, rev]) else {
            return false;
        };
        let Ok(tree) = git(dir, &["rev-parse", &format!("{target}^{{tree}}")]) else {
            return false;
        };
        merged.lines().next().map(str::trim) == Some(tree.trim())
    })
}

/// The published default branch, else the main worktree's current branch.
fn default_branch(main_root: &Path) -> String {
    let published = git(
        main_root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .map(|s| s.trim().trim_start_matches("origin/").to_string())
    .unwrap_or_default();
    if !published.is_empty() {
        return published;
    }
    git(main_root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}
