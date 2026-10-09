// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `pr_state_dir` reads the process cwd and `$HOME`, so these tests serialise
//! on them the way `tests/gate_record.rs`'s `ENV_LOCK` does.

use playbook::pr::shared::pr_state_dir;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_repo_and_home<T>(repo: &Path, home: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prev_cwd = std::env::current_dir().expect("read current dir");
    let prev_home = std::env::var_os("HOME");
    std::env::set_current_dir(repo).expect("cd into repo");
    std::env::set_var("HOME", home);
    let out = f();
    std::env::set_current_dir(&prev_cwd).expect("restore cwd");
    match prev_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    out
}

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-pr-state-{tag}-{}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir");
    dir.canonicalize().expect("canonicalize scratch dir")
}

fn repo_with_origin(tag: &str, with_origin: bool) -> (PathBuf, PathBuf) {
    let repo = scratch(&format!("{tag}-repo"));
    let home = scratch(&format!("{tag}-home"));
    let status = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(status.success());
    if with_origin {
        let status = Command::new("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/test-owner/test-repo.git",
            ])
            .current_dir(&repo)
            .status()
            .unwrap();
        assert!(status.success());
    }
    (repo, home)
}

#[test]
fn same_branch_resolves_the_same_path_twice() {
    // Arrange
    let (repo, home) = repo_with_origin("same", true);

    // Act
    let (first, second) = with_repo_and_home(&repo, &home, || {
        (pr_state_dir("feat/x"), pr_state_dir("feat/x"))
    });

    // Assert
    assert_eq!(first.expect("first call"), second.expect("second call"));
}

#[test]
fn different_branches_resolve_different_paths() {
    // Arrange
    let (repo, home) = repo_with_origin("diff", true);

    // Act
    let (a, b) = with_repo_and_home(&repo, &home, || {
        (pr_state_dir("feat/a"), pr_state_dir("feat/b"))
    });

    // Assert
    assert_ne!(a.expect("a"), b.expect("b"));
}

#[test]
fn path_sits_under_the_worktree_scoped_pr_dir_with_a_slugified_branch() {
    // Arrange
    let (repo, home) = repo_with_origin("shape", true);

    // Act
    let got = with_repo_and_home(&repo, &home, || pr_state_dir("feat/x_1")).expect("resolves");

    // Assert
    assert!(got.starts_with(home.join(".config/playbook/repos/test-owner/test-repo")));
    assert!(got.ends_with("pr/feat-x-1"), "got {got:?}");
}

#[test]
fn errors_clearly_outside_a_git_repository() {
    // Arrange
    let dir = scratch("nogit");
    let home = scratch("nogit-home");

    // Act
    let got = with_repo_and_home(&dir, &home, || pr_state_dir("feat/x"));

    // Assert
    let err = got.expect_err("not a repo must be an error");
    assert!(
        err.contains("could not resolve a worktree-scoped storage location"),
        "got {err}"
    );
}

#[test]
fn errors_clearly_when_there_is_no_origin_remote() {
    // Arrange
    let (repo, home) = repo_with_origin("noorigin", false);

    // Act
    let got = with_repo_and_home(&repo, &home, || pr_state_dir("feat/x"));

    // Assert
    assert!(got.is_err());
}
