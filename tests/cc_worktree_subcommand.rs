// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! End-to-end tests for `playbook cc worktree <branch> [env-base]` (WU-18).
//!
//! Spawns the real compiled binary rather than calling `cc::worktree_run::run`
//! in-process: the whole point of this subcommand is its OS-pipe-level
//! output contract (stdout carries only the path, so a shell can safely
//! `cd "$(playbook cc worktree foo)"`), and only a real subprocess proves
//! that.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("playbook-wtcli-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir");
    // macOS's /tmp is a symlink into /private/tmp; canonicalize now so every
    // path built from this compares equal to what git and the child process
    // themselves report.
    fs::canonicalize(&dir).expect("canonicalize scratch dir")
}

/// A git invocation isolated from the real machine's global/system config, so
/// the default branch name and every other config-dependent behaviour stays
/// deterministic regardless of what is installed on the host.
fn git(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git")
}

/// A real repo with one commit on `master`.
fn repo(tag: &str) -> PathBuf {
    let dir = scratch(tag);
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "T"],
    ] {
        git(&dir, &args);
    }
    fs::write(dir.join("README.md"), "hi").expect("write");
    git(&dir, &["add", "README.md"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

/// Runs the compiled binary's `cc worktree` subcommand with `cwd` as the
/// child process's own current directory (`Command::current_dir`, scoped to
/// the child only): unlike `std::env::set_current_dir`, this never touches
/// this test process's own cwd, so parallel tests cannot corrupt each other.
fn run_cli(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .arg("cc")
        .arg("worktree")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env_remove("WORKTREE_BASE_DIR")
        .env_remove("WORKTREE_NO_PUSH")
        .output()
        .expect("run playbook cc worktree")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// The default worktree root `resolve_base` computes for `repo_root`:
/// `<repo-parent>/.worktrees/<repo-name>`.
fn wt_root_of(repo_root: &Path) -> PathBuf {
    repo_root
        .parent()
        .expect("repo has a parent")
        .join(".worktrees")
        .join(repo_root.file_name().expect("repo has a name"))
}

#[test]
fn successful_run_prints_only_the_worktree_path_on_stdout() {
    // Arrange: "develop" is a protected branch (never auto-rebased), which
    // keeps this test hermetic without configuring an `origin` remote at all.
    let repo = repo("success");
    git(&repo, &["branch", "develop"]);

    // Act
    let out = run_cli(&repo, &["develop"]);

    // Assert
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stdout_of(&out));
    let printed = stdout_of(&out);
    let lines: Vec<&str> = printed.lines().collect();
    assert_eq!(
        lines.len(),
        1,
        "stdout should be exactly one line: {printed:?}"
    );

    let path = PathBuf::from(lines[0]);
    assert!(
        path.is_dir(),
        "printed path {path:?} should be a real directory"
    );
    assert_eq!(path, wt_root_of(&repo).join("develop"));

    let head = git(&path, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "develop");
}

#[test]
fn invalid_branch_name_exits_2_and_prints_nothing_to_stdout() {
    // Arrange
    let repo = repo("invalid-branch");

    // Act: ".." is never a valid git ref component.
    let out = run_cli(&repo, &["bad..branch"]);

    // Assert
    assert_eq!(out.status.code(), Some(2));
    assert!(stdout_of(&out).is_empty());
}

#[test]
fn non_git_directory_exits_10() {
    // Arrange
    let dir = scratch("non-git");

    // Act
    let out = run_cli(&dir, &["anything"]);

    // Assert
    assert_eq!(out.status.code(), Some(10));
    assert!(stdout_of(&out).is_empty());
}

#[test]
fn stash_is_restored_when_the_run_fails_after_stashing() {
    // Arrange: a worktree already occupies the target folder on a branch
    // that still exists on `origin`, so `prepare_worktree` refuses (exit 5)
    // rather than recycling it. That refusal lands AFTER the auto-stash, so
    // this proves restore_stash runs even on a failure path.
    let repo = repo("stash-restore");
    let origin = scratch("stash-restore-origin");
    git(&origin, &["init", "-q", "--bare"]);
    git(
        &repo,
        &["remote", "add", "origin", &origin.to_string_lossy()],
    );
    git(&repo, &["branch", "old-work"]);

    let wt_root = wt_root_of(&repo);
    fs::create_dir_all(&wt_root).expect("wt root");
    let occupied = wt_root.join("shared");
    git(
        &repo,
        &["worktree", "add", &occupied.to_string_lossy(), "old-work"],
    );
    let push = git(&repo, &["push", "origin", "old-work"]);
    assert!(push.status.success(), "push should seed the remote branch");

    fs::write(repo.join("README.md"), "dirty-edit").expect("dirty the tree");

    // Act
    let out = run_cli(&repo, &["shared"]);

    // Assert
    assert_eq!(out.status.code(), Some(5));
    assert!(stdout_of(&out).is_empty());

    let stash_list = git(&repo, &["stash", "list"]);
    assert!(
        String::from_utf8_lossy(&stash_list.stdout)
            .trim()
            .is_empty(),
        "the auto-stash should have been popped back, not left behind"
    );
    let restored = fs::read_to_string(repo.join("README.md")).expect("read README.md");
    assert_eq!(
        restored, "dirty-edit",
        "the stashed edit should be restored to the main worktree"
    );
}
