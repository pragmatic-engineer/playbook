// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Two defaults of Claude Code's own worktree creation that the
//! `WorktreeCreate` hook replaces, so the hook has to provide them itself:
//! copying the gitignored files named by `.worktreeinclude`, and keeping
//! `origin/HEAD` at most a day old. Neither may fail the hook: a problem is
//! reported on stderr and the worktree is still created.

use crate::common::{run_with_input, run_with_timeout};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

/// How long a refresh of `origin/HEAD` stays good.
pub(crate) const FETCH_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// The most a refresh may wait for the network.
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const GIT_TIMEOUT: Duration = Duration::from_secs(45);
/// Written inside the git directory after every refresh attempt, so a repo
/// that is offline is not retried on every worktree.
const STAMP_FILE: &str = "playbook-origin-fetch";
/// A `.worktreeinclude` that matches more than this is almost certainly a
/// pattern like `*` and would copy a whole dependency tree.
const MAX_FILES: usize = 10_000;

fn git_command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(dir);
    command
}

/// Copies the files that match a pattern in `<main_root>/.worktreeinclude`
/// and are also ignored by git into the same relative path under `target`.
/// Tracked files are never copied, an existing destination is never
/// overwritten, and symlinks are skipped. Returns the number copied.
pub(crate) fn copy_worktreeinclude(main_root: &Path, target: &Path) -> usize {
    let include = main_root.join(".worktreeinclude");
    if !include.is_file() {
        return 0;
    }
    let mut list = git_command(main_root);
    list.args(["ls-files", "-z", "--others", "--ignored"])
        .arg(format!("--exclude-from={}", include.display()));
    let Some(out) = run_with_timeout(&mut list, GIT_TIMEOUT).filter(|o| o.status.success()) else {
        eprintln!("playbook worktree-create: could not read .worktreeinclude matches; skipped");
        return 0;
    };
    let candidates = out.stdout;
    if candidates.is_empty() {
        return 0;
    }
    // `.worktreeinclude` matches only count when git also ignores the file.
    let mut check = git_command(main_root);
    check.args(["check-ignore", "-z", "--stdin"]);
    let Some(out) = run_with_input(&mut check, &candidates, GIT_TIMEOUT) else {
        eprintln!("playbook worktree-create: could not check ignored files; skipped");
        return 0;
    };
    // check-ignore exits 1 when nothing is ignored, which is not an error here.
    let Ok(canonical_target) = target.canonicalize() else {
        return 0;
    };
    let mut copied = 0;
    for raw in out.stdout.split(|&b| b == 0).filter(|p| !p.is_empty()) {
        if copied >= MAX_FILES {
            eprintln!(
                "playbook worktree-create: .worktreeinclude matches more than {MAX_FILES} files; stopped"
            );
            break;
        }
        let Ok(rel) = std::str::from_utf8(raw) else {
            continue;
        };
        if copy_one(main_root, &canonical_target, rel) {
            copied += 1;
        }
    }
    copied
}

/// A path git printed relative to the repo: only plain components.
fn safe_relative(rel: &str) -> Option<PathBuf> {
    let path = PathBuf::from(rel);
    let plain = path.components().all(|c| matches!(c, Component::Normal(_)))
        && path.components().next().is_some();
    plain.then_some(path)
}

fn copy_one(main_root: &Path, canonical_target: &Path, rel: &str) -> bool {
    let Some(rel_path) = safe_relative(rel) else {
        return false;
    };
    let source = main_root.join(&rel_path);
    match fs::symlink_metadata(&source) {
        Ok(meta) if meta.is_file() => {}
        _ => return false,
    }
    let dest = canonical_target.join(&rel_path);
    if fs::symlink_metadata(&dest).is_ok() {
        return false;
    }
    let Some(parent) = dest.parent() else {
        return false;
    };
    if fs::create_dir_all(parent).is_err() {
        return false;
    }
    // A tracked symlinked directory in the worktree must not redirect the copy
    // outside it.
    match parent.canonicalize() {
        Ok(real) if real.starts_with(canonical_target) => {}
        _ => return false,
    }
    fs::copy(&source, &dest).is_ok()
}

/// Whether the git directory shows a fetch or an attempt in the last day.
fn fetched_recently(git_dir: &Path) -> bool {
    [STAMP_FILE, "FETCH_HEAD"].iter().any(|name| {
        fs::metadata(git_dir.join(name))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < FETCH_INTERVAL)
    })
}

fn git_output(dir: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut command = git_command(dir);
    command
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "true")
        .env("SSH_ASKPASS", "true");
    let out = run_with_timeout(&mut command, timeout)?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Fetches the default branch of `origin` when the repo has not been
/// fetched in the last 24 hours, waiting at most five seconds and never for
/// input. A failure keeps the cached ref. Returns whether a fetch succeeded.
pub(crate) fn refresh_origin_head(main_root: &Path) -> bool {
    let Some(common) = git_output(
        main_root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        GIT_TIMEOUT,
    ) else {
        return false;
    };
    let git_dir = PathBuf::from(common);
    if fetched_recently(&git_dir) {
        return false;
    }
    if git_output(main_root, &["remote", "get-url", "origin"], GIT_TIMEOUT).is_none() {
        return false;
    }
    let touch = || {
        let _ = fs::write(git_dir.join(STAMP_FILE), b"");
    };
    let head_ref = ["symbolic-ref", "--short", "refs/remotes/origin/HEAD"];
    let mut default = git_output(main_root, &head_ref, GIT_TIMEOUT);
    if default.is_none()
        && git_output(
            main_root,
            &["remote", "set-head", "origin", "--auto"],
            FETCH_TIMEOUT,
        )
        .is_some()
    {
        default = git_output(main_root, &head_ref, GIT_TIMEOUT);
    }
    let Some(branch) = default.as_deref().and_then(|d| d.strip_prefix("origin/")) else {
        touch();
        return false;
    };
    let fetched = git_output(
        main_root,
        &["fetch", "--quiet", "--no-tags", "origin", branch],
        FETCH_TIMEOUT,
    )
    .is_some();
    touch();
    if !fetched {
        eprintln!(
            "playbook worktree-create: could not refresh origin/{branch}; using the cached ref"
        );
    }
    fetched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_relative_paths_are_safe() {
        assert!(safe_relative(".env").is_some());
        assert!(safe_relative("config/secrets.json").is_some());
        assert!(safe_relative("../x").is_none());
        assert!(safe_relative("/etc/passwd").is_none());
        assert!(safe_relative("a/../../b").is_none());
        assert!(safe_relative("").is_none());
    }
}
