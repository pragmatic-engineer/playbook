// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Tests for `worktree::classify`'s path-convention matching, using real
//! scratch git repos and real `git worktree add` calls rather than mocked
//! git plumbing, matching `tests/cc_worktree.rs`'s convention.

use playbook::worktree::{
    classify, named_branch_landed, parse_lock_pid, pid_is_alive, review_worktree_landed,
    wu_worktree_landed, Convention, PrState,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// `classify` calls `repo_slug()`, which shells out against the process's
/// current directory, so any test that relies on it must serialize on cwd
/// like `src/common/paths.rs`'s own tests do.
static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn lock_cwd() -> std::sync::MutexGuard<'static, ()> {
    CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("playbook-wtsweep-{tag}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir");
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

fn git_stdout(repo_path: &Path, args: &[&str]) -> String {
    let output = git(repo_path, args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Initializes `dir` as a repo with one commit on `main` and an `origin`
/// remote, so `repo_slug()` resolves to a stable `owner/repo`.
fn init_repo(dir: &Path, remote_url: &str) {
    fs::create_dir_all(dir).expect("create repo dir");
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "T"],
        vec!["remote", "add", "origin", remote_url],
    ] {
        git_ok(dir, &args);
    }
    fs::write(dir.join("README.md"), "seed\n").expect("write seed file");
    git_ok(dir, &["add", "."]);
    git_ok(dir, &["commit", "-q", "-m", "seed"]);
}

/// Adds a real linked worktree on a new named branch, returning its
/// canonical path.
fn add_worktree(repo_root: &Path, dest: &Path, branch: &str) -> PathBuf {
    fs::create_dir_all(dest.parent().expect("dest has a parent")).expect("mkdir dest parent");
    git_ok(
        repo_root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            branch,
            dest.to_str().expect("utf8 path"),
            "main",
        ],
    );
    dest.canonicalize().expect("worktree should resolve")
}

/// Adds a real linked worktree with a detached HEAD, returning its
/// canonical path.
fn add_detached_worktree(repo_root: &Path, dest: &Path) -> PathBuf {
    fs::create_dir_all(dest.parent().expect("dest has a parent")).expect("mkdir dest parent");
    git_ok(
        repo_root,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            dest.to_str().expect("utf8 path"),
            "main",
        ],
    );
    dest.canonicalize().expect("worktree should resolve")
}

#[test]
fn classify_returns_expected_convention_for_each_of_the_four_path_conventions() {
    // Arrange: one repo with an origin remote, so `repo_slug()` resolves to
    // a stable `owner/repo` for the Wu-convention match, plus one linked
    // worktree per convention under a scratch `$HOME` and the repo's
    // container directory.
    let _guard = lock_cwd();
    let container = scratch("classify")
        .canonicalize()
        .expect("container should resolve");
    let repo_root = container.join("repo");
    init_repo(&repo_root, "https://github.com/acme/widgets.git");
    let repo_root = repo_root.canonicalize().expect("repo root should resolve");
    let home = container.join("home");
    fs::create_dir_all(&home).expect("create home dir");
    let home = home.canonicalize().expect("home should resolve");

    let wu_path = add_detached_worktree(
        &repo_root,
        &home
            .join(".config")
            .join("playbook")
            .join("repos")
            .join("acme")
            .join("widgets")
            .join("wt-abc123")
            .join("worktrees")
            .join("plan-slug")
            .join("wu-1"),
    );

    let agent_tool_path = add_worktree(
        &repo_root,
        &repo_root.join(".claude").join("worktrees").join("agent-99"),
        "agent-branch",
    );

    let cc_launcher_path = add_worktree(
        &repo_root,
        &container
            .join(".worktrees")
            .join("repo")
            .join("feature-branch"),
        "feature-branch",
    );

    let common_dir = git_stdout(
        &repo_root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let review_path = add_detached_worktree(
        &repo_root,
        &Path::new(&common_dir)
            .join("review-worktrees")
            .join("42-abc1234"),
    );

    // A hand-made directory under the same container, and a wu-shaped path
    // with an extra nested segment: neither should ever classify as managed.
    let plain_path = container.join("just-a-directory");
    fs::create_dir_all(&plain_path).expect("create plain dir");

    let extra_nested_wu_path = home
        .join(".config")
        .join("playbook")
        .join("repos")
        .join("acme")
        .join("widgets")
        .join("wt-abc123")
        .join("worktrees")
        .join("plan-slug")
        .join("wu-1")
        .join("too-deep");
    fs::create_dir_all(&extra_nested_wu_path).expect("create extra nested dir");

    let previous_cwd = std::env::current_dir().expect("read current dir");
    std::env::set_current_dir(&repo_root).expect("cd into repo root");

    // Act
    let cases = [
        (wu_path, Convention::Wu),
        (agent_tool_path, Convention::AgentTool),
        (cc_launcher_path, Convention::CcLauncher),
        (review_path, Convention::Review),
        (plain_path, Convention::Unmanaged),
        (extra_nested_wu_path, Convention::Unmanaged),
    ];
    let got: Vec<Convention> = cases
        .iter()
        .map(|(path, _)| classify(path, &home))
        .collect();

    std::env::set_current_dir(&previous_cwd).expect("restore cwd");

    // Assert
    for ((path, expected), actual) in cases.iter().zip(got) {
        assert_eq!(
            actual, *expected,
            "path {path:?} classified as {actual:?}, expected {expected:?}"
        );
    }

    let _ = fs::remove_dir_all(&container);
}

#[test]
fn classify_returns_wu_when_called_from_a_different_worktree_of_the_same_repo() {
    // Arrange: a Wu-convention path built with one worktree-id segment
    // ("wt-abc123"), plus a second linked worktree of the same repo standing
    // in for a different simulated toplevel that shares the same origin
    // remote.
    let _guard = lock_cwd();
    let container = scratch("classify-cross-wt")
        .canonicalize()
        .expect("container should resolve");
    let repo_root = container.join("repo");
    init_repo(&repo_root, "https://github.com/acme/widgets.git");
    let repo_root = repo_root.canonicalize().expect("repo root should resolve");
    let home = container.join("home");
    fs::create_dir_all(&home).expect("create home dir");
    let home = home.canonicalize().expect("home should resolve");

    let wu_path = add_detached_worktree(
        &repo_root,
        &home
            .join(".config")
            .join("playbook")
            .join("repos")
            .join("acme")
            .join("widgets")
            .join("wt-abc123")
            .join("worktrees")
            .join("plan-slug")
            .join("wu-1"),
    );

    // A second Wu-convention path with a DIFFERENT worktree-id segment
    // ("wt-def456" vs "wt-abc123"): if the wildcard were ever narrowed to a
    // literal match, only one of these two would still classify as Wu.
    let other_wu_path = add_detached_worktree(
        &repo_root,
        &home
            .join(".config")
            .join("playbook")
            .join("repos")
            .join("acme")
            .join("widgets")
            .join("wt-def456")
            .join("worktrees")
            .join("plan-slug")
            .join("wu-2"),
    );

    let other_toplevel = add_worktree(
        &repo_root,
        &container.join("other-toplevel"),
        "other-branch",
    );

    let previous_cwd = std::env::current_dir().expect("read current dir");
    std::env::set_current_dir(&other_toplevel).expect("cd into the other worktree's toplevel");

    // Act
    let got = classify(&wu_path, &home);
    let other_got = classify(&other_wu_path, &home);

    std::env::set_current_dir(&previous_cwd).expect("restore cwd");

    // Assert
    assert_eq!(
        got,
        Convention::Wu,
        "expected Wu when classifying from a different toplevel of the same repo, got {got:?}"
    );
    assert_eq!(
        other_got,
        Convention::Wu,
        "expected Wu for a different worktree-id segment too, got {other_got:?}"
    );

    let _ = fs::remove_dir_all(&container);
}

#[test]
fn classify_returns_cc_launcher_when_called_from_a_different_linked_worktree_of_the_same_repo() {
    // Arrange: a cc-launcher-convention worktree (`.worktrees/<repo>/<branch>`
    // relative to the main worktree's parent, same shape as scenario 1's
    // fixture), plus a second linked worktree of the same repo standing in
    // for the eager trigger's cwd (inside the just-created worktree, not the
    // main checkout).
    let _guard = lock_cwd();
    let container = scratch("classify-cross-wt-cc")
        .canonicalize()
        .expect("container should resolve");
    let repo_root = container.join("repo");
    init_repo(&repo_root, "https://github.com/acme/widgets.git");
    let repo_root = repo_root.canonicalize().expect("repo root should resolve");
    let home = container.join("home");
    fs::create_dir_all(&home).expect("create home dir");
    let home = home.canonicalize().expect("home should resolve");

    let cc_launcher_path = add_worktree(
        &repo_root,
        &container
            .join(".worktrees")
            .join("repo")
            .join("feature-branch"),
        "feature-branch",
    );

    let other_toplevel = add_worktree(
        &repo_root,
        &container.join("other-toplevel"),
        "other-branch",
    );

    let previous_cwd = std::env::current_dir().expect("read current dir");
    std::env::set_current_dir(&other_toplevel).expect("cd into the other worktree's toplevel");

    // Act
    let got = classify(&cc_launcher_path, &home);

    std::env::set_current_dir(&previous_cwd).expect("restore cwd");

    // Assert
    assert_eq!(
        got,
        Convention::CcLauncher,
        "expected CcLauncher when classifying from a different linked worktree of the same repo, got {got:?}"
    );

    let _ = fs::remove_dir_all(&container);
}

#[test]
fn wu_worktree_landed_true_when_commit_reachable_from_default_branch_false_when_not() {
    // Arrange: a scratch repo with a seed commit, a hand-built `origin/main`
    // ref advanced past it (the ancestor case), and a divergent commit on an
    // unmerged feature branch that `origin/main` never reaches.
    let repo_root = scratch("wu-landed")
        .canonicalize()
        .expect("repo root should resolve");
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "T"],
    ] {
        git_ok(&repo_root, &args);
    }
    fs::write(repo_root.join("README.md"), "seed\n").expect("write seed file");
    git_ok(&repo_root, &["add", "."]);
    git_ok(&repo_root, &["commit", "-q", "-m", "seed"]);
    let landed_commit = git_stdout(&repo_root, &["rev-parse", "HEAD"]);

    fs::write(repo_root.join("README.md"), "advance\n").expect("write advance file");
    git_ok(&repo_root, &["add", "."]);
    git_ok(&repo_root, &["commit", "-q", "-m", "advance main"]);
    let origin_main_sha = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
    git_ok(
        &repo_root,
        &["update-ref", "refs/remotes/origin/main", &origin_main_sha],
    );

    git_ok(
        &repo_root,
        &["checkout", "-q", "-b", "feature", &landed_commit],
    );
    fs::write(repo_root.join("feature.txt"), "wip\n").expect("write feature file");
    git_ok(&repo_root, &["add", "."]);
    git_ok(&repo_root, &["commit", "-q", "-m", "unmerged feature work"]);
    let unmerged_commit = git_stdout(&repo_root, &["rev-parse", "HEAD"]);

    // Act
    let cases = [
        (landed_commit.as_str(), true),
        (unmerged_commit.as_str(), false),
    ];
    let got: Vec<bool> = cases
        .iter()
        .map(|(commit, _)| wu_worktree_landed(commit, "main", &repo_root))
        .collect();

    // Assert
    for ((commit, expected), actual) in cases.iter().zip(got) {
        assert_eq!(
            actual, *expected,
            "commit {commit} landed={actual}, expected {expected}"
        );
    }

    let _ = fs::remove_dir_all(&repo_root);
}

/// Checks out a fresh branch off `main` with one commit dated `epoch` (both
/// author and committer), so the age check under test reads a real,
/// controllable `%ct` rather than depending on wall-clock time.
fn commit_branch_at_epoch(repo_root: &Path, branch: &str, epoch: i64) {
    git_ok(repo_root, &["checkout", "-q", "main"]);
    git_ok(repo_root, &["checkout", "-q", "-b", branch]);
    let date = format!("{epoch} +0000");
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["commit", "-q", "--allow-empty", "-m", "aged commit"])
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .output()
        .expect("git commit should spawn");
    assert!(
        out.status.success(),
        "git commit --allow-empty failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn named_branch_landed_covers_pr_state_local_fallback_and_age_boundary() {
    // Arrange: one repo with a default branch, a throwaway branch for the
    // PR-state cases (their outcome must be decided before any local git
    // state is consulted), a branch actually merged back into main for the
    // no-PR fallback case, and three branches whose last commit sits under,
    // exactly at, and over the staleness threshold relative to a fixed,
    // injected `now_epoch`.
    const NOW_EPOCH: i64 = 1_700_000_000;
    const STALE_AFTER_DAYS: i64 = 30;
    const SECS_PER_DAY: i64 = 86_400;

    let repo_root = scratch("named-branch-landed")
        .canonicalize()
        .expect("repo root should resolve");
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "T"],
    ] {
        git_ok(&repo_root, &args);
    }
    fs::write(repo_root.join("README.md"), "seed\n").expect("write seed file");
    git_ok(&repo_root, &["add", "."]);
    git_ok(&repo_root, &["commit", "-q", "-m", "seed"]);

    git_ok(&repo_root, &["branch", "pr-branch"]);

    git_ok(&repo_root, &["checkout", "-q", "-b", "merged-branch"]);
    fs::write(repo_root.join("merged.txt"), "merged\n").expect("write merged file");
    git_ok(&repo_root, &["add", "."]);
    git_ok(&repo_root, &["commit", "-q", "-m", "merged branch work"]);
    git_ok(&repo_root, &["checkout", "-q", "main"]);
    git_ok(&repo_root, &["merge", "-q", "--no-ff", "merged-branch"]);

    commit_branch_at_epoch(&repo_root, "unmerged-fresh", NOW_EPOCH - SECS_PER_DAY);
    commit_branch_at_epoch(
        &repo_root,
        "unmerged-just-under",
        NOW_EPOCH - (STALE_AFTER_DAYS * SECS_PER_DAY - 1),
    );
    commit_branch_at_epoch(
        &repo_root,
        "unmerged-at-boundary",
        NOW_EPOCH - STALE_AFTER_DAYS * SECS_PER_DAY,
    );
    commit_branch_at_epoch(
        &repo_root,
        "unmerged-stale",
        NOW_EPOCH - (STALE_AFTER_DAYS + 1) * SECS_PER_DAY,
    );
    // Checked out on a throwaway branch, not "main": pins that the landed
    // check compares against the passed-in default branch, never whatever
    // HEAD happens to be pointed at.
    git_ok(&repo_root, &["checkout", "-q", "-b", "some-other-checkout"]);

    // Act
    let cases = [
        ("unmerged-fresh", Some(PrState::Merged), true),
        ("pr-branch", Some(PrState::Open), false),
        ("merged-branch", None, true),
        ("unmerged-fresh", None, false),
        ("unmerged-just-under", None, false),
        ("unmerged-stale", None, true),
        ("unmerged-at-boundary", None, true),
    ];
    let got: Vec<bool> = cases
        .iter()
        .map(|(branch, pr_state, _)| {
            named_branch_landed(
                branch,
                "main",
                STALE_AFTER_DAYS,
                &repo_root,
                NOW_EPOCH,
                *pr_state,
            )
        })
        .collect();

    // Assert
    for ((branch, pr_state, expected), actual) in cases.iter().zip(got) {
        assert_eq!(
            actual, *expected,
            "branch {branch} pr_state {pr_state:?} landed={actual}, expected {expected}"
        );
    }

    let _ = fs::remove_dir_all(&repo_root);
}

#[test]
fn review_worktree_landed_covers_lock_state_and_pid_liveness() {
    // Arrange: a live child process standing in for a lock still held by a
    // running review session, and a reaped child standing in for one whose
    // owner already exited without releasing the lock. A dead lock pid is
    // never sufficient on its own here: `review-worktree.sh` locks with its
    // own short-lived setup script's pid, so that pid is dead even for a
    // review still actively running; the lock's age must also clear a real
    // TTL (`REVIEW_LOCK_TTL_SECS`, 24 hours) before it reads as landed.
    const NEVER_LOCKED_GRACE_SECS: i64 = 30;
    const REVIEW_LOCK_TTL_SECS: i64 = 86_400;

    let mut live_child = Command::new("sleep")
        .arg("5")
        .spawn()
        .expect("sleep child should spawn");
    let live_pid = live_child.id();

    let mut dead_child = Command::new("true")
        .spawn()
        .expect("true child should spawn");
    let dead_pid = dead_child.id();
    dead_child.wait().expect("true child should exit");

    // Act
    let cases = [
        (false, None, NEVER_LOCKED_GRACE_SECS + 1, true),
        (false, None, NEVER_LOCKED_GRACE_SECS - 1, false),
        (false, None, NEVER_LOCKED_GRACE_SECS, true),
        (true, None, 0, false),
        (true, None, REVIEW_LOCK_TTL_SECS, true),
        (true, Some(live_pid), 0, false),
        (true, Some(live_pid), REVIEW_LOCK_TTL_SECS, false),
        (true, Some(dead_pid), 0, false),
        (true, Some(dead_pid), REVIEW_LOCK_TTL_SECS - 1, false),
        (true, Some(dead_pid), REVIEW_LOCK_TTL_SECS, true),
    ];
    let got: Vec<bool> = cases
        .iter()
        .map(|(is_locked, lock_owner_pid, lock_age_secs, _)| {
            review_worktree_landed(
                *is_locked,
                *lock_owner_pid,
                *lock_age_secs,
                NEVER_LOCKED_GRACE_SECS,
            )
        })
        .collect();

    live_child.kill().expect("kill live child");
    live_child.wait().expect("wait for killed live child");

    // Assert
    for ((is_locked, lock_owner_pid, lock_age_secs, expected), actual) in cases.iter().zip(got) {
        assert_eq!(
            actual, *expected,
            "is_locked={is_locked} lock_owner_pid={lock_owner_pid:?} lock_age_secs={lock_age_secs} \
             landed={actual}, expected {expected}"
        );
    }
}

#[test]
fn parse_lock_pid_covers_both_lock_formats_and_malformed_input() {
    // Arrange: a flat `pid=<n>` reason as `review-worktree.sh` writes it, a
    // parenthesized `pid <n>` reason as the Agent tool writes it, and two
    // malformed shapes: no pid token at all, and a `pid=` key with no digits.
    let cases = [
        ("review pr=42 pid=12345 ts=1234567890", Some(12345)),
        (
            "claude agent abc123 (pid 12345 start 2026-09-01T00:00:00Z)",
            Some(12345),
        ),
        ("review pr=42 ts=1234567890", None),
        ("review pr=42 pid= ts=1234567890", None),
    ];

    // Act
    let got: Vec<Option<u32>> = cases
        .iter()
        .map(|(lock_reason, _)| parse_lock_pid(lock_reason))
        .collect();

    // Assert
    for ((lock_reason, expected), actual) in cases.iter().zip(got) {
        assert_eq!(
            actual, *expected,
            "lock_reason {lock_reason:?} parsed pid={actual:?}, expected {expected:?}"
        );
    }
}

#[test]
fn pid_is_alive_true_for_a_running_process_false_for_a_reaped_one() {
    // Arrange: a real long-running child standing in for a live pid, and a
    // real short-lived child that has already been waited on, so its pid is
    // reaped rather than a hardcoded constant.
    let mut running_child = Command::new("sleep")
        .arg("10")
        .spawn()
        .expect("sleep child should spawn");
    let running_pid = running_child.id();

    let mut reaped_child = Command::new("true")
        .spawn()
        .expect("true child should spawn");
    let reaped_pid = reaped_child.id();
    reaped_child.wait().expect("true child should exit");

    // Act
    let cases = [(running_pid, true), (reaped_pid, false)];
    let got: Vec<bool> = cases.iter().map(|(pid, _)| pid_is_alive(*pid)).collect();

    running_child.kill().expect("kill running child");
    running_child.wait().expect("wait for killed running child");

    // Assert
    for ((pid, expected), actual) in cases.iter().zip(got) {
        assert_eq!(
            actual, *expected,
            "pid {pid} alive={actual}, expected {expected}"
        );
    }
}
