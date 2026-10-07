// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook worktree review setup|teardown`: the locked, detached PR review
//! worktrees `/playbook:quick-review` and `/playbook:deep-review` run in.
//! Ported from the retired shell original.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// Default age past which a review worktree's lock counts as stale.
const DEFAULT_TTL_SECS: i64 = 86_400;

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `git <args>` in the current directory, trimmed stdout on success.
fn git_out(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Like [`git_ok`] but lets git's stderr reach the caller's.
fn git_loud(args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn git_ok(args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Removes every unlocked, or lock-expired, worktree under
/// `<root>/review-worktrees/`, then prunes orphaned admin entries.
fn sweep_stale(root: &Path, now: i64, ttl: i64) {
    let Ok(entries) = fs::read_dir(root.join("worktrees")) else {
        return;
    };
    let review_base = root.join("review-worktrees");
    for entry in entries.flatten() {
        let meta = entry.path();
        let Ok(gitdir) = fs::read_to_string(meta.join("gitdir")) else {
            continue;
        };
        let Some(wt_path) = Path::new(gitdir.trim()).parent() else {
            continue;
        };
        if !wt_path.starts_with(&review_base) {
            continue;
        }
        let stale = match fs::read_to_string(meta.join("locked")) {
            Err(_) => true,
            Ok(reason) => match lock_ts(&reason) {
                None => true,
                Some(ts) => now - ts > ttl,
            },
        };
        if stale {
            let path = wt_path.to_string_lossy();
            git_ok(&["worktree", "remove", "-f", "-f", &path]);
        }
    }
    git_ok(&["worktree", "prune"]);
}

/// The `ts=<epoch>` value inside a lock reason, if any.
fn lock_ts(reason: &str) -> Option<i64> {
    let rest = &reason[reason.find("ts=")? + 3..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Creates a locked, detached worktree for PR `pr` at `head_sha` and returns
/// its absolute path. Warnings go to stderr so stdout stays the path alone.
pub fn setup(pr: &str, head_sha: &str) -> Result<String, String> {
    let short: String = head_sha.chars().take(7).collect();
    let root = git_out(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .ok_or("not in a git repo")?;
    let dir = PathBuf::from(&root)
        .join("review-worktrees")
        .join(format!("{pr}-{short}"));

    let ttl = std::env::var("REVIEW_WT_TTL_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_TTL_SECS);
    sweep_stale(Path::new(&root), now_secs(), ttl);

    let fetch_url = match std::env::var("GH_FETCH_URL") {
        Ok(url) if !url.is_empty() => url,
        _ => {
            let out = Command::new("gh")
                .args(["repo", "view", "--json", "url", "-q", ".url"])
                .stderr(Stdio::inherit())
                .output();
            match out {
                Ok(o) if o.status.success() => {
                    String::from_utf8_lossy(&o.stdout).trim().to_string()
                }
                _ => {
                    return Err(
                        "failed to resolve repo URL via gh (not authenticated or not a \
                                GitHub repo)"
                            .to_string(),
                    )
                }
            }
        }
    };
    let pull_ref = format!("refs/pull/{pr}/head");
    let fetched = Command::new("git")
        .args(["fetch", &fetch_url, &pull_ref])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !fetched {
        return Err(format!("failed to fetch {pull_ref}"));
    }

    let origin_sha =
        git_out(&["rev-parse", "FETCH_HEAD"]).ok_or("failed to resolve FETCH_HEAD after fetch")?;
    if origin_sha != head_sha {
        let now_short: String = origin_sha.chars().take(7).collect();
        eprintln!(
            "warning: PR {pr} has moved since head_sha was resolved (requested {short}, origin \
             now at {now_short}); re-run to review the latest commits"
        );
    }

    if !git_loud(&["cat-file", "-e", &format!("{head_sha}^{{commit}}")]) {
        return Err(format!(
            "head {head_sha} not found after fetch (force-push?); re-run"
        ));
    }

    let dir_str = dir.to_string_lossy().to_string();
    if dir.is_dir() && is_locked(&dir_str) {
        return Err(format!("review already in progress for PR {pr}"));
    }

    if let Some(parent) = dir.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let added = Command::new("git")
        .args(["worktree", "add", "--detach", &dir_str, head_sha])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !added {
        return Err(format!("failed to add worktree at {dir_str}"));
    }

    let reason = format!(
        "review pr={pr} pid={} ts={}",
        std::process::id(),
        now_secs()
    );
    if !git_loud(&["worktree", "lock", "--reason", &reason, &dir_str]) {
        return Err(format!("failed to lock worktree at {dir_str}"));
    }
    Ok(dir_str)
}

/// Whether `git worktree list --porcelain` shows `dir` as locked.
fn is_locked(dir: &str) -> bool {
    let Some(list) = git_out(&["worktree", "list", "--porcelain"]) else {
        return false;
    };
    let header = format!("worktree {dir}");
    let mut in_block = false;
    for line in list.lines() {
        if line.is_empty() {
            in_block = false;
        } else if line == header {
            in_block = true;
        } else if in_block && line.starts_with("locked") {
            return true;
        }
    }
    false
}

/// Removes the review worktree at `path`, always, even after an aborted or
/// dirty review. Never fails: a missing path is already torn down. A path
/// outside `review-worktrees/` is left alone, so a wrong argument cannot wipe
/// another worktree.
pub fn teardown(path: &Path) {
    if !super::is_review_convention(path) {
        return;
    }
    let path_str = path.to_string_lossy();
    git_ok(&["worktree", "unlock", &path_str]);
    let home = crate::common::home_dir();
    let slug = crate::common::repo_slug();
    let slug = (!slug.is_empty()).then_some(slug);
    let repo_root = std::env::current_dir().unwrap_or_default();
    // A disabled cleanup policy exits Ok without removing, so check the path.
    if let Ok(line) = super::remove(&repo_root, &home, slug.as_deref(), path, now_secs()) {
        println!("{line}");
    }
    if path.exists() {
        git_ok(&["worktree", "remove", "-f", "-f", &path_str]);
    }
    git_ok(&["worktree", "prune"]);
}
