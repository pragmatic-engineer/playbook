// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `shell/review-worktree.sh`'s `cmd_teardown`, which
//! now delegates to `playbook worktree remove` (falling back to the old
//! unconditional `git worktree remove -f -f` when that refuses), matching
//! `tests/cc_core.rs`'s convention of shelling out to the real script rather
//! than re-implementing its logic in Rust.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-review-teardown-{tag}-{}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir");
    fs::canonicalize(&dir).expect("canonicalize scratch dir")
}

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

fn git_ok(dir: &Path, args: &[&str]) {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_stdout(dir: &Path, args: &[&str]) -> String {
    String::from_utf8_lossy(&git(dir, args).stdout)
        .trim()
        .to_string()
}

fn repo() -> PathBuf {
    let dir = scratch("repo");
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "T"],
    ] {
        git_ok(&dir, &args);
    }
    fs::write(dir.join("README.md"), "seed\n").expect("write seed file");
    git_ok(&dir, &["add", "."]);
    git_ok(&dir, &["commit", "-q", "-m", "seed"]);
    dir
}

/// Runs `shell/review-worktree.sh teardown <path>` with the compiled
/// `playbook` binary on `PATH`, matching how the script calls it in
/// production (a bare `playbook`, resolved via the shell's own PATH).
fn run_teardown(repo_root: &Path, worktree_path: &Path) -> Output {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shell/review-worktree.sh");
    let bin_dir = PathBuf::from(env!("CARGO_BIN_EXE_playbook"))
        .parent()
        .expect("compiled binary has a parent dir")
        .to_path_buf();
    let path_var = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new("bash")
        .arg(&script)
        .arg("teardown")
        .arg(worktree_path)
        .current_dir(repo_root)
        .env("PATH", path_var)
        .env("HOME", repo_root) // isolate from the developer's real config
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("bash should run review-worktree.sh")
}

fn bash_available() -> bool {
    Command::new("bash").arg("--version").output().is_ok()
}

#[test]
fn teardown_removes_a_normally_unlocked_review_worktree() {
    if !bash_available() {
        eprintln!("SKIP: bash not available");
        return;
    }

    // Arrange: a review-convention worktree, unlocked (the normal shape
    // `cmd_teardown` runs `git worktree unlock` against first).
    let repo_root = repo();
    let common_dir = git_stdout(
        &repo_root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let worktree_path = PathBuf::from(&common_dir)
        .join("review-worktrees")
        .join("1-abcdef1");
    git_ok(
        &repo_root,
        &[
            "worktree",
            "add",
            "--detach",
            worktree_path.to_str().expect("utf8 path"),
            "main",
        ],
    );

    // Act
    let out = run_teardown(&repo_root, &worktree_path);

    // Assert
    assert!(
        out.status.success(),
        "teardown should exit 0, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        fs::symlink_metadata(&worktree_path).is_err(),
        "the review worktree should have been removed: {}",
        worktree_path.display()
    );
    // Pins the delegation itself, not just the end state the fallback
    // `git worktree remove -f -f` would also produce: `playbook worktree
    // remove` prints `worktree <path>: removed` on its own success path,
    // and the fallback prints nothing.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("{}: removed", worktree_path.display())),
        "expected the playbook worktree remove delegation to report removal, got: {stdout}"
    );

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn teardown_removes_a_dirty_worktree_left_by_an_aborted_review() {
    if !bash_available() {
        eprintln!("SKIP: bash not available");
        return;
    }

    // Arrange: a review-convention worktree with an untracked file left
    // behind, standing in for build artifacts a check suite run during
    // review can leave uncommitted (deep-review always runs the full check
    // suite in the worktree). `playbook worktree remove` alone would refuse
    // this (its uncommitted-work guard), so this pins the unconditional
    // fallback `cmd_teardown` needs: commands/quick-review.md and
    // commands/deep-review.md both depend on teardown removing the worktree
    // even after an aborted or dirty review, never leaving it behind.
    let repo_root = repo();
    let common_dir = git_stdout(
        &repo_root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let worktree_path = PathBuf::from(&common_dir)
        .join("review-worktrees")
        .join("2-abcdef2");
    git_ok(
        &repo_root,
        &[
            "worktree",
            "add",
            "--detach",
            worktree_path.to_str().expect("utf8 path"),
            "main",
        ],
    );
    fs::write(
        worktree_path.join("build-artifact.log"),
        "leftover output\n",
    )
    .expect("leave an untracked file behind, simulating a check-suite run");

    // Act
    let out = run_teardown(&repo_root, &worktree_path);

    // Assert: still removed, unconditionally, exit 0.
    assert!(
        out.status.success(),
        "teardown should exit 0 even for a dirty worktree, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        fs::symlink_metadata(&worktree_path).is_err(),
        "a dirty review worktree left by an aborted review must still be removed: {}",
        worktree_path.display()
    );
    // Pins that the FALLBACK specifically fired, not the primary delegation:
    // `playbook worktree remove` refuses a dirty worktree and prints nothing
    // on that path, so its success line must be absent here.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains(&format!("{}: removed", worktree_path.display())),
        "playbook worktree remove should have refused the dirty worktree, not reported removing it: {stdout}"
    );

    let _ = fs::remove_dir_all(&repo_root);
}
