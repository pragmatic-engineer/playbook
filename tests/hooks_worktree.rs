// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Tests for the `worktree-create` and `worktree-remove` hooks, spawning the
//! real binary against real scratch git repos. Env is set per child process,
//! never on the test process, so no env lock is needed.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A scratch `<root>/proj/<repo>` checkout with a bare `origin`, plus the root.
struct Fixture {
    root: PathBuf,
    repo: PathBuf,
}

fn git(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .expect("git should spawn")
}

fn git_ok(dir: &Path, args: &[&str]) -> String {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn fixture(tag: &str) -> Fixture {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("playbook-wt-hook-{}-{tag}-{n}", std::process::id()));
    let repo = root.join("proj").join("app");
    let origin = root.join("origin.git");
    fs::create_dir_all(&repo).unwrap();
    git_ok(
        &root,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            origin.to_str().unwrap(),
        ],
    );
    git_ok(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join("README.md"), "seed\n").unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "seed"]);
    git_ok(
        &repo,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git_ok(&repo, &["push", "-q", "-u", "origin", "main"]);
    Fixture { root, repo }
}

fn run_hook(name: &str, payload: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["hook", name])
        .env_remove("CI")
        .env_remove("PLAYBOOK_HEADLESS")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("playbook should spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn create(repo: &Path, name: &str) -> Output {
    let payload = format!(
        r#"{{"hook_event_name":"WorktreeCreate","cwd":"{}","worktree_name":"{name}"}}"#,
        repo.display()
    );
    run_hook("worktree-create", &payload)
}

fn remove(cwd: &Path, path: &Path) -> Output {
    let payload = format!(
        r#"{{"hook_event_name":"WorktreeRemove","cwd":"{}","worktree_path":"{}"}}"#,
        cwd.display(),
        path.display()
    );
    run_hook("worktree-remove", &payload)
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn created_path(out: &Output) -> PathBuf {
    assert!(out.status.success(), "create failed: {}", stderr(out));
    PathBuf::from(stdout(out).trim_end_matches('\n'))
}

#[test]
fn create_uses_cc_launcher_location_and_prints_only_the_path() {
    let f = fixture("create");
    let out = create(&f.repo, "probe");
    let expected = f.root.join("proj/.worktrees/app/probe");
    assert_eq!(stdout(&out), format!("{}\n", expected.display()));
    assert!(expected.join("README.md").exists());
    assert_eq!(
        git_ok(&expected, &["symbolic-ref", "--short", "HEAD"]).trim(),
        "worktree-probe"
    );
    assert!(git_ok(&f.repo, &["worktree", "list"]).contains("probe"));
}

#[test]
fn create_twice_reuses_the_registered_worktree() {
    let f = fixture("reuse");
    let first = created_path(&create(&f.repo, "probe"));
    fs::write(first.join("scratch.txt"), "keep me\n").unwrap();
    let second = created_path(&create(&f.repo, "probe"));
    assert_eq!(first, second);
    assert!(second.join("scratch.txt").exists());
}

#[test]
fn create_rejects_traversal_and_separator_names() {
    let f = fixture("sanitize");
    for bad in ["..", "../escape", "a/b", ""] {
        let out = create(&f.repo, bad);
        assert!(!out.status.success(), "{bad:?} should fail");
        assert!(
            stdout(&out).is_empty(),
            "stdout must stay empty for {bad:?}"
        );
    }
    assert!(!f.root.join("proj/escape").exists());
    assert!(!f.repo.join(".claude").exists());
}

#[test]
fn create_falls_back_to_claude_worktrees_when_base_dir_is_a_file() {
    let f = fixture("fallback");
    fs::write(f.root.join("proj/.worktrees"), "not a dir").unwrap();
    let out = create(&f.repo, "probe");
    let path = created_path(&out);
    assert_eq!(path, f.repo.join(".claude/worktrees/probe"));
    assert!(path.join("README.md").exists());
    assert!(stderr(&out).contains("falling back"));
    assert_eq!(stdout(&out), format!("{}\n", path.display()));
}

#[test]
fn create_uses_payload_base_commit_when_given() {
    let f = fixture("basecommit");
    let seed = git_ok(&f.repo, &["rev-parse", "HEAD"]).trim().to_string();
    fs::write(f.repo.join("later.txt"), "x\n").unwrap();
    git_ok(&f.repo, &["add", "."]);
    git_ok(&f.repo, &["commit", "-q", "-m", "later"]);
    let payload = format!(
        r#"{{"cwd":"{}","worktree_name":"old","base_commit":"{seed}"}}"#,
        f.repo.display()
    );
    let path = created_path(&run_hook("worktree-create", &payload));
    assert!(!path.join("later.txt").exists());
}

#[test]
fn remove_deletes_a_clean_pushed_worktree_and_its_branch() {
    let f = fixture("rm-clean");
    let path = created_path(&create(&f.repo, "done"));
    let out = remove(&f.repo, &path);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!path.exists());
    assert!(git_ok(&f.repo, &["branch", "--list", "worktree-done"])
        .trim()
        .is_empty());
}

#[test]
fn remove_keeps_a_worktree_with_uncommitted_changes() {
    let f = fixture("rm-dirty");
    let path = created_path(&create(&f.repo, "dirty"));
    fs::write(path.join("untracked.txt"), "wip\n").unwrap();
    let out = remove(&f.repo, &path);
    assert!(out.status.success());
    assert!(path.exists());
    assert!(stderr(&out).contains("uncommitted"));
}

#[test]
fn remove_keeps_a_worktree_with_unpushed_commits() {
    let f = fixture("rm-unpushed");
    let path = created_path(&create(&f.repo, "ahead"));
    fs::write(path.join("new.txt"), "work\n").unwrap();
    git_ok(&path, &["add", "."]);
    git_ok(&path, &["commit", "-q", "-m", "local only"]);
    let out = remove(&f.repo, &path);
    assert!(out.status.success());
    assert!(path.exists());
    assert!(stderr(&out).contains("commits"));
    assert!(!git_ok(&f.repo, &["branch", "--list", "worktree-ahead"])
        .trim()
        .is_empty());
}

#[test]
fn remove_deletes_after_the_branch_is_pushed() {
    let f = fixture("rm-pushed");
    let path = created_path(&create(&f.repo, "shipped"));
    fs::write(path.join("new.txt"), "work\n").unwrap();
    git_ok(&path, &["add", "."]);
    git_ok(&path, &["commit", "-q", "-m", "work"]);
    git_ok(&path, &["push", "-q", "origin", "worktree-shipped"]);
    let out = remove(&f.repo, &path);
    assert!(out.status.success());
    assert!(!path.exists());
}

#[test]
fn remove_refuses_a_non_worktree_path_and_the_main_worktree() {
    let f = fixture("rm-refuse");
    let stranger = f.root.join("stranger");
    fs::create_dir_all(&stranger).unwrap();
    fs::write(stranger.join("f.txt"), "x").unwrap();

    let out = remove(&f.repo, &stranger);
    assert!(out.status.success());
    assert!(stranger.join("f.txt").exists());
    assert!(stderr(&out).contains("not a worktree"));

    let out = remove(&f.repo, &f.repo);
    assert!(out.status.success());
    assert!(f.repo.join("README.md").exists());
    assert!(stderr(&out).contains("main worktree"));
}
