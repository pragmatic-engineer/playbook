// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Every scenario in `shell/check-manifest.test.sh`, ported, plus the two
//! cases the brief called out explicitly: a brand new disallowed top-level
//! directory being rejected, and an allowlisted one being accepted. That is
//! the exact failure this validator exists to catch.
//!
//! `src/manifest/check.rs` already covers the allowlist decision itself with
//! plain-list unit tests; these drive the real CLI against real git repos so
//! the `git ls-files` wiring is proven too.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// GIT_CONFIG_GLOBAL/GIT_CONFIG_SYSTEM are process-wide, and cargo runs the
/// tests in this binary on parallel threads, so mutating them needs one
/// shared lock (convention matches `tests/cc_worktree.rs`).
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Disables the machine's global/system git config for the duration of `f`,
/// so a contributor's own gpg-signing or hook config cannot break `git
/// commit` inside the scratch repos below.
fn with_isolated_git_env<T>(f: impl FnOnce() -> T) -> T {
    let _guard = lock_env();
    let prev_global = std::env::var_os("GIT_CONFIG_GLOBAL");
    let prev_system = std::env::var_os("GIT_CONFIG_SYSTEM");
    std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
    std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");
    let out = f();
    match prev_global {
        Some(v) => std::env::set_var("GIT_CONFIG_GLOBAL", v),
        None => std::env::remove_var("GIT_CONFIG_GLOBAL"),
    }
    match prev_system {
        Some(v) => std::env::set_var("GIT_CONFIG_SYSTEM", v),
        None => std::env::remove_var("GIT_CONFIG_SYSTEM"),
    }
    out
}

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-manifest-check-{tag}-{}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir should be creatable");
    dir
}

fn git(repo_path: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .output()
        .expect("git command should spawn")
}

fn git_ok(repo_path: &Path, args: &[&str]) {
    let out = git(repo_path, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo(dir: &Path) {
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "audit@example.test"],
        vec!["config", "user.name", "Audit Test"],
        vec!["config", "commit.gpgsign", "false"],
    ] {
        git_ok(dir, &args);
    }
}

/// Create an empty tracked fixture at `rel`, mirroring the shell suite's
/// `add_file`.
fn add_file(dir: &Path, rel: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().expect("rel should have a parent")).expect("mkdir -p");
    fs::write(path, "").expect("fixture file");
}

fn commit_repo(dir: &Path) {
    git_ok(dir, &["add", "-A"]);
    git_ok(dir, &["commit", "-q", "-m", "fixture"]);
}

/// An allowlisted skeleton: top-level files plus contents under allowlisted
/// directories, matching the shell suite's `seed_skeleton`.
fn seed_skeleton(dir: &Path) {
    add_file(dir, ".gitignore");
    add_file(dir, "README.md");
    add_file(dir, "LICENSE");
    add_file(dir, "settings.shared.json");
    add_file(dir, "permissions.shared.json");
    add_file(dir, "statusline.sh");
    add_file(dir, "shell/worktree.zsh");
    add_file(dir, "hooks/session-init.sh");
    add_file(dir, "docs/index.md");
    add_file(dir, ".github/workflows/ci.yml");
}

fn run(repo: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["manifest", "check"])
        .arg(repo)
        .output()
        .expect("playbook binary should spawn")
}

fn stderr_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn clean_skeleton_passes() {
    with_isolated_git_env(|| {
        let repo = scratch("clean");
        init_repo(&repo);
        seed_skeleton(&repo);
        commit_repo(&repo);

        let out = run(&repo);
        assert!(
            out.status.success(),
            "expected exit 0, got {:?}: {}",
            out.status.code(),
            stderr_of(&out)
        );
        assert!(String::from_utf8_lossy(&out.stdout).contains("check-manifest: OK"));
    });
}

#[test]
fn out_of_allowlist_file_fails() {
    with_isolated_git_env(|| {
        let repo = scratch("leak");
        init_repo(&repo);
        seed_skeleton(&repo);
        add_file(&repo, "sessions/leaked.json");
        commit_repo(&repo);

        let out = run(&repo);
        assert_eq!(out.status.code(), Some(1));
        let err = stderr_of(&out);
        assert!(err.contains("outside the allowlist"), "got: {err}");
        assert!(err.contains("sessions/leaked.json"), "got: {err}");
    });
}

#[test]
fn tracked_settings_json_fails() {
    with_isolated_git_env(|| {
        let repo = scratch("settings");
        init_repo(&repo);
        seed_skeleton(&repo);
        add_file(&repo, "settings.json");
        commit_repo(&repo);

        let out = run(&repo);
        assert_eq!(out.status.code(), Some(1));
        let err = stderr_of(&out);
        assert!(
            err.contains("personal settings.json must not be tracked"),
            "got: {err}"
        );
    });
}

/// The exact failure this validator exists to catch: a brand new top-level
/// directory nobody allowlisted.
#[test]
fn new_disallowed_top_level_directory_fails() {
    with_isolated_git_env(|| {
        let repo = scratch("new-dir");
        init_repo(&repo);
        seed_skeleton(&repo);
        add_file(&repo, "sneaky-new-dir/payload.txt");
        commit_repo(&repo);

        let out = run(&repo);
        assert_eq!(out.status.code(), Some(1));
        assert!(
            stderr_of(&out).contains("sneaky-new-dir/payload.txt"),
            "got: {}",
            stderr_of(&out)
        );
    });
}

/// The other half of the same check: an allowlisted top-level directory
/// (`skills/`, not exercised by `seed_skeleton`) must still pass.
#[test]
fn allowlisted_top_level_directory_passes() {
    with_isolated_git_env(|| {
        let repo = scratch("allowed-dir");
        init_repo(&repo);
        seed_skeleton(&repo);
        add_file(&repo, "skills/example/SKILL.md");
        commit_repo(&repo);

        let out = run(&repo);
        assert!(
            out.status.success(),
            "expected exit 0, got {:?}: {}",
            out.status.code(),
            stderr_of(&out)
        );
    });
}

#[test]
fn non_directory_repo_root_is_rejected() {
    let repo = scratch("not-a-dir").join("missing");
    let out = run(&repo);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_of(&out).contains("repo root is not a directory"),
        "got: {}",
        stderr_of(&out)
    );
}

#[test]
fn non_git_repo_root_is_rejected() {
    let dir = scratch("not-a-repo");
    let out = run(&dir);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr_of(&out).contains("not a git repository"),
        "got: {}",
        stderr_of(&out)
    );
}
