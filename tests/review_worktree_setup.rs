// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `playbook worktree review setup` and the setup then
//! teardown round trip, ported from `shell/review-worktree.test.sh`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-review-setup-{tag}-{}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir");
    fs::canonicalize(&dir).expect("canonicalize")
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

fn git_ok(dir: &Path, args: &[&str]) -> String {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A working repo, a bare clone standing in for GitHub, and the seed sha.
struct Fixture {
    repo: PathBuf,
    bare: PathBuf,
    sha: String,
}

impl Fixture {
    fn new() -> Self {
        let repo = scratch("repo");
        git_ok(&repo, &["init", "-q", "-b", "main"]);
        git_ok(&repo, &["config", "user.email", "t@t"]);
        git_ok(&repo, &["config", "user.name", "T"]);
        fs::write(repo.join("README"), "init\n").unwrap();
        git_ok(&repo, &["add", "."]);
        git_ok(&repo, &["commit", "-q", "-m", "initial"]);
        let sha = git_ok(&repo, &["rev-parse", "HEAD"]);
        let bare = scratch("bare");
        git_ok(
            &repo,
            &["clone", "--bare", "-q", ".", bare.to_str().unwrap()],
        );
        let fixture = Self { repo, bare, sha };
        fixture.set_pr_ref(7, &fixture.sha.clone());
        fixture
    }

    fn set_pr_ref(&self, pr: u32, sha: &str) {
        git_ok(
            &self.bare,
            &["update-ref", &format!("refs/pull/{pr}/head"), sha],
        );
    }

    /// Commit a change in the working repo, push the object to the bare, and
    /// return the new sha.
    fn new_commit(&self, line: &str) -> String {
        fs::write(self.repo.join("README"), format!("{line}\n")).unwrap();
        git_ok(&self.repo, &["commit", "-q", "-am", line]);
        let sha = git_ok(&self.repo, &["rev-parse", "HEAD"]);
        git_ok(
            &self.repo,
            &[
                "push",
                "-q",
                self.bare.to_str().unwrap(),
                "HEAD:refs/heads/side",
            ],
        );
        sha
    }

    fn playbook(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_playbook"));
        cmd.args(["worktree", "review"])
            .args(args)
            .current_dir(&self.repo)
            .env("HOME", &self.repo)
            .env("GH_FETCH_URL", format!("file://{}", self.bare.display()))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        cmd.output().expect("playbook")
    }

    fn setup(&self, pr: &str, sha: &str) -> Output {
        self.playbook(&["setup", pr, sha], &[])
    }

    fn listed(&self, path: &str) -> bool {
        git_ok(&self.repo, &["worktree", "list", "--porcelain"])
            .lines()
            .any(|l| l == format!("worktree {path}"))
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn setup_creates_a_locked_detached_worktree_at_the_requested_head() {
    let f = Fixture::new();

    let out = f.setup("7", &f.sha);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let wt = stdout(&out);
    assert!(
        Path::new(&wt).is_absolute() && Path::new(&wt).is_dir(),
        "{wt}"
    );
    assert!(f.listed(&wt));
    let porcelain = git_ok(&f.repo, &["worktree", "list", "--porcelain"]);
    let block: Vec<&str> = porcelain
        .split("\n\n")
        .filter(|b| b.contains(&format!("worktree {wt}")))
        .collect();
    assert!(block[0].contains("\nlocked"), "not locked: {}", block[0]);
    assert_eq!(git_ok(Path::new(&wt), &["rev-parse", "HEAD"]), f.sha);
}

#[test]
fn setup_then_teardown_removes_the_worktree_and_teardown_is_idempotent() {
    let f = Fixture::new();
    let wt = stdout(&f.setup("7", &f.sha));

    let first = f.playbook(&["teardown", &wt], &[]);
    let second = f.playbook(&["teardown", &wt], &[]);

    assert!(first.status.success() && second.status.success());
    assert!(!Path::new(&wt).exists(), "directory still exists");
    assert!(!f.listed(&wt), "worktree still listed");
}

#[test]
fn teardown_of_a_path_that_never_existed_succeeds() {
    let f = Fixture::new();
    let ghost = f.repo.join("nonexistent-worktree-path");

    let out = f.playbook(&["teardown", ghost.to_str().unwrap()], &[]);

    assert!(out.status.success());
}

#[test]
fn a_second_setup_while_live_locked_is_refused() {
    let f = Fixture::new();
    assert!(f.setup("7", &f.sha).status.success());

    let out = f.setup("7", &f.sha);

    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("already in progress"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_stale_locked_review_worktree_is_swept_during_setup() {
    let f = Fixture::new();
    let new_sha = f.new_commit("v2");
    f.set_pr_ref(7, &new_sha);
    let root = git_ok(
        &f.repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let stale = Path::new(&root)
        .join("review-worktrees")
        .join(format!("7-{}", &f.sha[..7]));
    fs::create_dir_all(stale.parent().unwrap()).unwrap();
    git_ok(
        &f.repo,
        &[
            "worktree",
            "add",
            "--detach",
            "-q",
            stale.to_str().unwrap(),
            &f.sha,
        ],
    );
    let old_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - 15;
    git_ok(
        &f.repo,
        &[
            "worktree",
            "lock",
            "--reason",
            &format!("review pr=7 pid=999999 ts={old_ts}"),
            stale.to_str().unwrap(),
        ],
    );

    let out = f.playbook(
        &["setup", "7", &new_sha],
        &[("REVIEW_WT_TTL_SECONDS", "10")],
    );

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(Path::new(&stdout(&out)).is_absolute());
    assert!(
        !f.listed(stale.to_str().unwrap()),
        "stale worktree survived"
    );
}

#[test]
fn setup_never_sweeps_an_unlocked_worktree_outside_review_worktrees() {
    let f = Fixture::new();
    let other = scratch("other").join("wu");
    git_ok(
        &f.repo,
        &[
            "worktree",
            "add",
            "--detach",
            "-q",
            other.to_str().unwrap(),
            &f.sha,
        ],
    );

    let out = f.setup("7", &f.sha);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        f.listed(other.to_str().unwrap()),
        "unrelated worktree was swept"
    );
}

#[test]
fn an_unlocked_leftover_under_review_worktrees_is_swept() {
    let f = Fixture::new();
    let root = git_ok(
        &f.repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let leftover = Path::new(&root).join("review-worktrees").join("3-abcdef1");
    fs::create_dir_all(leftover.parent().unwrap()).unwrap();
    git_ok(
        &f.repo,
        &[
            "worktree",
            "add",
            "--detach",
            "-q",
            leftover.to_str().unwrap(),
            &f.sha,
        ],
    );

    let out = f.setup("7", &f.sha);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(
        !f.listed(leftover.to_str().unwrap()),
        "unlocked leftover survived"
    );
}

#[test]
fn teardown_leaves_a_worktree_outside_review_worktrees_alone() {
    let f = Fixture::new();
    let other = scratch("other2").join("wu");
    git_ok(
        &f.repo,
        &[
            "worktree",
            "add",
            "--detach",
            "-q",
            other.to_str().unwrap(),
            &f.sha,
        ],
    );
    fs::write(other.join("dirty.txt"), "work\n").unwrap();

    let out = f.playbook(&["teardown", other.to_str().unwrap()], &[]);

    assert!(out.status.success());
    assert!(
        other.join("dirty.txt").exists(),
        "unrelated worktree was removed"
    );
}

#[test]
fn a_fresh_locked_worktree_for_another_pr_is_not_swept() {
    let f = Fixture::new();
    let sha8 = f.new_commit("pr8");
    f.set_pr_ref(8, &sha8);
    let pr7 = stdout(&f.setup("7", &f.sha));

    let out = f.setup("8", &sha8);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(f.listed(&pr7), "fresh PR 7 worktree was swept");
}

#[test]
fn setup_with_a_bogus_sha_fails() {
    let f = Fixture::new();

    let out = f.setup("7", "deadbeef000000000000000000000000deadbeef");

    assert!(!out.status.success());
}

#[test]
fn setup_works_from_another_cwd_via_git_dir_and_prints_an_absolute_path() {
    let f = Fixture::new();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_playbook"));
    cmd.args(["worktree", "review", "setup", "7", &f.sha])
        .current_dir(std::env::temp_dir())
        .env("GIT_DIR", f.repo.join(".git"))
        .env("HOME", &f.repo)
        .env("GH_FETCH_URL", format!("file://{}", f.bare.display()))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null");

    let out = cmd.output().unwrap();

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let wt = stdout(&out);
    assert!(
        Path::new(&wt).is_absolute() && Path::new(&wt).is_dir(),
        "{wt}"
    );
}

#[test]
fn setup_warns_when_origin_moved_but_still_checks_out_the_requested_sha() {
    let f = Fixture::new();
    let new_sha = f.new_commit("v2");
    f.set_pr_ref(7, &new_sha);

    let out = f.setup("7", &f.sha);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("has moved"), "{}", stderr(&out));
    let wt = stdout(&out);
    assert_eq!(git_ok(Path::new(&wt), &["rev-parse", "HEAD"]), f.sha);
}
