// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `playbook worktree sweep`/`remove`, spawning the
//! real compiled binary against real scratch git repos and real `git
//! worktree add` calls, matching `tests/config_cli.rs`'s convention for
//! CLI-layer subcommands and `tests/worktree_sweep.rs`'s convention for
//! building convention-specific worktree fixtures.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-worktree-cli-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir should be creatable");
    dir
}

fn git(repo: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git command should spawn")
}

fn git_ok(repo: &Path, args: &[&str]) {
    let out = git(repo, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    String::from_utf8_lossy(&git(repo, args).stdout)
        .trim()
        .to_string()
}

/// A repo with one seed commit, dated `commit_epoch`, and an `origin` remote
/// so `repo_slug()` resolves to `<owner>/<repo>`.
fn init_repo_at_epoch(dir: &Path, remote_url: &str, commit_epoch: i64) {
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
    let date = format!("{commit_epoch} +0000");
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-q", "-m", "seed"])
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .output()
        .expect("git commit should spawn");
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

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

/// Run the real compiled binary with `cwd` and `$HOME` pinned to scratch
/// locations, so it never touches the developer's real config.
fn run_playbook(cwd: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .output()
        .expect("playbook binary should spawn")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn still_registered(repo_root: &Path, worktree_path: &Path) -> bool {
    let porcelain = git_stdout(repo_root, &["worktree", "list", "--porcelain"]);
    porcelain.contains(&worktree_path.to_string_lossy().into_owned())
}

/// Asserts `worktree_path` was both removed from disk, deregistered from
/// `git worktree list`, and named in the sweep's own report.
fn assert_removed_and_reported(repo_root: &Path, worktree_path: &Path, out: &Output) {
    assert!(out.status.success(), "stderr: {}", stderr_of(out));
    assert!(
        fs::symlink_metadata(worktree_path).is_err(),
        "worktree directory should have been removed: {}",
        worktree_path.display()
    );
    assert!(
        !still_registered(repo_root, worktree_path),
        "worktree should no longer be a registered git worktree"
    );
    let stdout = stdout_of(out);
    assert!(
        stdout.contains(&worktree_path.to_string_lossy().into_owned()),
        "sweep should report the removed path, got: {stdout}"
    );
    assert!(
        stdout.to_lowercase().contains("removed"),
        "sweep should report the worktree as removed, got: {stdout}"
    );
}

#[test]
fn sweep_removes_a_landed_unlocked_wu_convention_worktree() {
    // Arrange: a Wu-convention worktree checked out at the repo's only
    // commit, with `origin/main` pointing at that same commit, so
    // `wu_worktree_landed` sees it as already reachable from the default
    // branch.
    let container = scratch("wu").canonicalize().expect("container resolves");
    let repo_root = container.join("repo");
    init_repo_at_epoch(
        &repo_root,
        "https://github.com/acme/widgets.git",
        1_700_000_000,
    );
    let repo_root = repo_root.canonicalize().expect("repo root resolves");
    let head = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
    git_ok(
        &repo_root,
        &["update-ref", "refs/remotes/origin/main", &head],
    );
    let home = container.join("home");
    fs::create_dir_all(&home).expect("create home dir");
    let home = home.canonicalize().expect("home resolves");

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

    // Act
    let out = run_playbook(&repo_root, &home, &["worktree", "sweep"]);

    // Assert
    assert_removed_and_reported(&repo_root, &wu_path, &out);

    let _ = fs::remove_dir_all(&container);
}

#[test]
fn sweep_dry_run_reports_without_removing_a_landed_unlocked_wu_convention_worktree() {
    // Arrange: same landed, unlocked Wu-convention fixture as
    // `sweep_removes_a_landed_unlocked_wu_convention_worktree`.
    let container = scratch("wu-dry-run")
        .canonicalize()
        .expect("container resolves");
    let repo_root = container.join("repo");
    init_repo_at_epoch(
        &repo_root,
        "https://github.com/acme/widgets.git",
        1_700_000_000,
    );
    let repo_root = repo_root.canonicalize().expect("repo root resolves");
    let head = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
    git_ok(
        &repo_root,
        &["update-ref", "refs/remotes/origin/main", &head],
    );
    let home = container.join("home");
    fs::create_dir_all(&home).expect("create home dir");
    let home = home.canonicalize().expect("home resolves");

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

    // Act
    let out = run_playbook(&repo_root, &home, &["worktree", "sweep", "--dry-run"]);

    // Assert: nothing removed, but reported as a dry-run candidate.
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    assert!(
        fs::symlink_metadata(&wu_path).is_ok(),
        "worktree directory should still exist under dry-run: {}",
        wu_path.display()
    );
    assert!(
        still_registered(&repo_root, &wu_path),
        "worktree should still be a registered git worktree under dry-run"
    );
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("would remove (dry run)"),
        "sweep --dry-run should report the dry-run wording, got: {stdout}"
    );
    assert!(
        stdout.contains(&wu_path.to_string_lossy().into_owned()),
        "sweep --dry-run should report the candidate path, got: {stdout}"
    );

    let _ = fs::remove_dir_all(&container);
}

#[test]
fn sweep_leaves_an_unlanded_wu_convention_worktree_alone() {
    // Arrange: a Wu-convention worktree on its own branch with a commit that
    // is never merged into `origin/main`, so `wu_worktree_landed` sees it as
    // unreachable from the default branch.
    let container = scratch("wu-unlanded")
        .canonicalize()
        .expect("container resolves");
    let repo_root = container.join("repo");
    init_repo_at_epoch(
        &repo_root,
        "https://github.com/acme/widgets.git",
        1_700_000_000,
    );
    let repo_root = repo_root.canonicalize().expect("repo root resolves");
    let head = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
    git_ok(
        &repo_root,
        &["update-ref", "refs/remotes/origin/main", &head],
    );
    let home = container.join("home");
    fs::create_dir_all(&home).expect("create home dir");
    let home = home.canonicalize().expect("home resolves");

    let wu_path = add_worktree(
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
        "wu-2-unlanded",
    );
    fs::write(wu_path.join("unmerged.txt"), "work in progress\n")
        .expect("write unmerged file in worktree");
    git_ok(&wu_path, &["add", "."]);
    git_ok(&wu_path, &["commit", "-q", "-m", "unmerged work"]);

    // Act
    let out = run_playbook(&repo_root, &home, &["worktree", "sweep"]);

    // Assert: left alone, on disk, still registered, reported as not landed.
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    assert!(
        fs::symlink_metadata(&wu_path).is_ok(),
        "unlanded worktree directory should still exist: {}",
        wu_path.display()
    );
    assert!(
        still_registered(&repo_root, &wu_path),
        "unlanded worktree should still be a registered git worktree"
    );
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("not landed, skipping"),
        "sweep should report the unlanded worktree as not landed, got: {stdout}"
    );

    let _ = fs::remove_dir_all(&container);
}

#[test]
fn sweep_leaves_a_worktree_locked_by_a_live_pid_alone() {
    // Arrange: the same landed, unlocked Wu-convention fixture as
    // `sweep_removes_a_landed_unlocked_wu_convention_worktree`, then locked
    // with a reason naming a real running child process's pid, standing in
    // for a worktree another process still holds open.
    let container = scratch("wu-live-lock")
        .canonicalize()
        .expect("container resolves");
    let repo_root = container.join("repo");
    init_repo_at_epoch(
        &repo_root,
        "https://github.com/acme/widgets.git",
        1_700_000_000,
    );
    let repo_root = repo_root.canonicalize().expect("repo root resolves");
    let head = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
    git_ok(
        &repo_root,
        &["update-ref", "refs/remotes/origin/main", &head],
    );
    let home = container.join("home");
    fs::create_dir_all(&home).expect("create home dir");
    let home = home.canonicalize().expect("home resolves");

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

    let mut live_child = Command::new("sleep")
        .arg("10")
        .spawn()
        .expect("sleep child should spawn");
    let live_pid = live_child.id();
    git_ok(
        &repo_root,
        &[
            "worktree",
            "lock",
            wu_path.to_str().expect("utf8 path"),
            "--reason",
            &format!("pid={live_pid}"),
        ],
    );

    // Act
    let out = run_playbook(&repo_root, &home, &["worktree", "sweep"]);

    live_child.kill().expect("kill live child");
    live_child.wait().expect("wait for killed live child");

    // Assert: left alone, on disk, still registered and still locked.
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    assert!(
        fs::symlink_metadata(&wu_path).is_ok(),
        "worktree locked by a live pid should still exist: {}",
        wu_path.display()
    );
    assert!(
        still_registered(&repo_root, &wu_path),
        "worktree locked by a live pid should still be a registered git worktree"
    );
    let porcelain = git_stdout(&repo_root, &["worktree", "list", "--porcelain"]);
    assert!(
        porcelain.contains("locked"),
        "worktree should still be listed as locked, got: {porcelain}"
    );
    let stdout = stdout_of(&out);
    assert!(
        stdout.contains("locked by a live process, skipping"),
        "sweep should report the live-locked worktree as skipped, got: {stdout}"
    );

    let _ = fs::remove_dir_all(&container);
}

#[test]
fn sweep_removes_a_landed_unlocked_agent_tool_convention_worktree() {
    // Arrange: an AgentTool-convention worktree on a branch whose tip equals
    // `main`'s tip, so `git branch --merged <default>` already lists it as
    // merged. The seed commit is recent (well under staleAfterDays), so the
    // only way this can read as landed is through the merged-branch check
    // itself, not the age fallback.
    let container = scratch("agent-tool")
        .canonicalize()
        .expect("container resolves");
    let repo_root = container.join("repo");
    let recent_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should read after the unix epoch")
        .as_secs() as i64
        - 60;
    init_repo_at_epoch(
        &repo_root,
        "https://github.com/acme/widgets.git",
        recent_epoch,
    );
    let repo_root = repo_root.canonicalize().expect("repo root resolves");
    let head = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
    git_ok(
        &repo_root,
        &["update-ref", "refs/remotes/origin/main", &head],
    );
    let home = container.join("home");
    fs::create_dir_all(&home).expect("create home dir");
    let home = home.canonicalize().expect("home resolves");

    let agent_tool_path = add_worktree(
        &repo_root,
        &repo_root.join(".claude").join("worktrees").join("agent-99"),
        "agent-branch",
    );

    // Act
    let out = run_playbook(&repo_root, &home, &["worktree", "sweep"]);

    // Assert
    assert_removed_and_reported(&repo_root, &agent_tool_path, &out);

    let _ = fs::remove_dir_all(&container);
}

#[test]
fn sweep_removes_a_landed_unlocked_cc_launcher_convention_worktree() {
    // Arrange: a CcLauncher-convention worktree
    // (`<main-worktree-parent>/.worktrees/<repo>/<branch>`) on a branch
    // whose tip equals `main`'s tip, exercising the merged-branch check the
    // same way the AgentTool fixture does (recent seed commit, so the age
    // fallback can't be what makes this pass).
    let container = scratch("cc-launcher")
        .canonicalize()
        .expect("container resolves");
    let repo_root = container.join("repo");
    let recent_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should read after the unix epoch")
        .as_secs() as i64
        - 60;
    init_repo_at_epoch(
        &repo_root,
        "https://github.com/acme/widgets.git",
        recent_epoch,
    );
    let repo_root = repo_root.canonicalize().expect("repo root resolves");
    let head = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
    git_ok(
        &repo_root,
        &["update-ref", "refs/remotes/origin/main", &head],
    );
    let home = container.join("home");
    fs::create_dir_all(&home).expect("create home dir");
    let home = home.canonicalize().expect("home resolves");

    let cc_launcher_path = add_worktree(
        &repo_root,
        &container
            .join(".worktrees")
            .join("repo")
            .join("feature-branch"),
        "feature-branch",
    );

    // Act
    let out = run_playbook(&repo_root, &home, &["worktree", "sweep"]);

    // Assert
    assert_removed_and_reported(&repo_root, &cc_launcher_path, &out);

    let _ = fs::remove_dir_all(&container);
}

#[test]
fn sweep_removes_a_worktree_locked_by_a_dead_pid_via_double_force() {
    // Arrange: the same landed, unlocked Wu-convention fixture as
    // `sweep_removes_a_landed_unlocked_wu_convention_worktree`, then locked
    // with a reason naming a just-reaped child process's pid, standing in
    // for a lock left behind by a process that no longer exists. A locked
    // worktree normally refuses `git worktree remove` without at least one
    // `--force`, and refuses again with only one `--force`, so successful
    // removal here is itself evidence the double-force override fired.
    let container = scratch("wu-dead-lock")
        .canonicalize()
        .expect("container resolves");
    let repo_root = container.join("repo");
    init_repo_at_epoch(
        &repo_root,
        "https://github.com/acme/widgets.git",
        1_700_000_000,
    );
    let repo_root = repo_root.canonicalize().expect("repo root resolves");
    let head = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
    git_ok(
        &repo_root,
        &["update-ref", "refs/remotes/origin/main", &head],
    );
    let home = container.join("home");
    fs::create_dir_all(&home).expect("create home dir");
    let home = home.canonicalize().expect("home resolves");

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

    let mut dead_child = Command::new("true")
        .spawn()
        .expect("true child should spawn");
    let dead_pid = dead_child.id();
    dead_child
        .wait()
        .expect("wait for true child should reap it");
    git_ok(
        &repo_root,
        &[
            "worktree",
            "lock",
            wu_path.to_str().expect("utf8 path"),
            "--reason",
            &format!("pid={dead_pid}"),
        ],
    );

    // Act
    let out = run_playbook(&repo_root, &home, &["worktree", "sweep"]);

    // Assert
    assert_removed_and_reported(&repo_root, &wu_path, &out);

    let _ = fs::remove_dir_all(&container);
}

/// Formats `epoch_secs` as `YYYY-MM-DDTHH:MM:SSZ`, trying BSD `date -u -r`
/// then GNU `date -u -d @`, the same fallback order
/// `cc::sessions::local_timestamp` uses to render an arbitrary past epoch.
fn iso8601_utc(epoch_secs: i64) -> String {
    format_epoch(epoch_secs, "+%Y-%m-%dT%H:%M:%SZ")
}

/// Formats `epoch_secs` with `format_spec`, trying BSD `date -u -r` then GNU
/// `date -u -d`, the fallback order shared by every real-clock test helper
/// in this file.
fn format_epoch(epoch_secs: i64, format_spec: &str) -> String {
    for args in [
        vec!["-u".to_string(), "-r".to_string(), epoch_secs.to_string()],
        vec!["-u".to_string(), "-d".to_string(), format!("@{epoch_secs}")],
    ] {
        if let Ok(out) = Command::new("date").args(&args).arg(format_spec).output() {
            if out.status.success() {
                return String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
        }
    }
    panic!("date should format epoch {epoch_secs} on either BSD or GNU");
}

/// Backdates `path`'s mtime to `epoch_secs`, standing in for a worktree
/// created a while ago: `touch -t`'s `[[CC]YY]MMDDhhmm.SS` shape is accepted
/// by both BSD and GNU `touch`, so no OS-specific fallback is needed here.
fn touch_at_epoch(path: &Path, epoch_secs: i64) {
    let timestamp = format_epoch(epoch_secs, "+%Y%m%d%H%M.%S");
    let out = Command::new("touch")
        .arg("-t")
        .arg(&timestamp)
        .arg(path)
        .output()
        .expect("touch command should spawn");
    assert!(
        out.status.success(),
        "touch -t {timestamp} {path:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write_conflict_marker(worktree_path: &Path, epoch_secs: i64) {
    fs::write(
        worktree_path.join(".playbook-conflict-stop"),
        format!(
            "merge conflict during rebase\n{}\n",
            iso8601_utc(epoch_secs)
        ),
    )
    .expect("write conflict-STOP marker");
}

/// A Wu-convention worktree whose own commit is already reachable from
/// `origin/main`, the same landed fixture
/// `sweep_removes_a_landed_unlocked_wu_convention_worktree` exercises, so a
/// present conflict-STOP marker's grace period is the only thing left to
/// decide the outcome, not the underlying commit.
fn build_landed_wu_fixture(tag: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let container = scratch(tag).canonicalize().expect("container resolves");
    let repo_root = container.join("repo");
    init_repo_at_epoch(
        &repo_root,
        "https://github.com/acme/widgets.git",
        1_700_000_000,
    );
    let repo_root = repo_root.canonicalize().expect("repo root resolves");
    let head = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
    git_ok(
        &repo_root,
        &["update-ref", "refs/remotes/origin/main", &head],
    );
    let home = container.join("home");
    fs::create_dir_all(&home).expect("create home dir");
    let home = home.canonicalize().expect("home resolves");

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

    (container, repo_root, home, wu_path)
}

enum ConflictMarkerExpectation {
    LeftAlone,
    Removed,
}

#[test]
fn sweep_applies_conflict_stop_marker_grace_period_boundary() {
    // Arrange: the real current time, since the compiled binary reads its
    // own `now_epoch` from the real clock with no injection point, so each
    // marker's timestamp is computed relative to whenever this test runs
    // rather than a fixed constant.
    const GRACE_PERIOD_DAYS: i64 = 90;
    const SECS_PER_DAY: i64 = 86_400;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should read after the unix epoch")
        .as_secs() as i64;

    let cases = [
        ("fresh", now, ConflictMarkerExpectation::LeftAlone),
        (
            "at-grace-boundary",
            now - GRACE_PERIOD_DAYS * SECS_PER_DAY,
            ConflictMarkerExpectation::Removed,
        ),
        (
            "past-grace-period",
            now - (GRACE_PERIOD_DAYS + 1) * SECS_PER_DAY,
            ConflictMarkerExpectation::Removed,
        ),
    ];

    for (tag, marker_epoch, expected) in cases {
        let (container, repo_root, home, wu_path) =
            build_landed_wu_fixture(&format!("conflict-marker-{tag}"));
        write_conflict_marker(&wu_path, marker_epoch);

        // Act
        let out = run_playbook(&repo_root, &home, &["worktree", "sweep"]);

        // Assert
        match expected {
            ConflictMarkerExpectation::LeftAlone => {
                assert!(
                    out.status.success(),
                    "case {tag}, stderr: {}",
                    stderr_of(&out)
                );
                assert!(
                    fs::symlink_metadata(&wu_path).is_ok(),
                    "case {tag}: worktree within the conflict-STOP grace period should still exist: {}",
                    wu_path.display()
                );
                assert!(
                    still_registered(&repo_root, &wu_path),
                    "case {tag}: worktree within the conflict-STOP grace period should still be registered"
                );
                let stdout = stdout_of(&out);
                assert!(
                    stdout.contains("not landed, skipping"),
                    "case {tag}: sweep should report the marker as still within grace, got: {stdout}"
                );
            }
            ConflictMarkerExpectation::Removed => {
                assert_removed_and_reported(&repo_root, &wu_path, &out);
            }
        }

        let _ = fs::remove_dir_all(&container);
    }
}

#[test]
fn sweep_removes_a_landed_unlocked_review_convention_worktree() {
    // Arrange: a Review-convention worktree that was never locked, whose own
    // `.git` file (the linked-worktree pointer `git worktree add` writes
    // once) is backdated well past the never-locked grace window, standing
    // in for a review session whose worktree has sat idle that long. The
    // checked-out commit's own date is irrelevant here: the landed check
    // reads the worktree's own creation time, not its commit's.
    let container = scratch("review")
        .canonicalize()
        .expect("container resolves");
    let repo_root = container.join("repo");
    init_repo_at_epoch(
        &repo_root,
        "https://github.com/acme/widgets.git",
        1_700_000_000,
    );
    let repo_root = repo_root.canonicalize().expect("repo root resolves");
    let home = container.join("home");
    fs::create_dir_all(&home).expect("create home dir");
    let home = home.canonicalize().expect("home resolves");

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
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should read after the unix epoch")
        .as_secs() as i64;
    touch_at_epoch(&review_path.join(".git"), now - 400 * 86_400);

    // Act
    let out = run_playbook(&repo_root, &home, &["worktree", "sweep"]);

    // Assert
    assert_removed_and_reported(&repo_root, &review_path, &out);

    let _ = fs::remove_dir_all(&container);
}

#[test]
fn sweep_never_removes_the_worktree_it_is_invoked_from() {
    // Arrange: a landed, unlocked Wu-convention worktree, otherwise
    // removable by every other rule the sweep applies, but invoked with cwd
    // set to that same worktree, standing in for the eager trigger's own
    // cwd (inside the just-created worktree, not the main checkout).
    let container = scratch("caller-self")
        .canonicalize()
        .expect("container resolves");
    let repo_root = container.join("repo");
    init_repo_at_epoch(
        &repo_root,
        "https://github.com/acme/widgets.git",
        1_700_000_000,
    );
    let repo_root = repo_root.canonicalize().expect("repo root resolves");
    let head = git_stdout(&repo_root, &["rev-parse", "HEAD"]);
    git_ok(
        &repo_root,
        &["update-ref", "refs/remotes/origin/main", &head],
    );
    let home = container.join("home");
    fs::create_dir_all(&home).expect("create home dir");
    let home = home.canonicalize().expect("home resolves");

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

    // Act: invoked from inside the worktree itself, not the main checkout.
    let out = run_playbook(&wu_path, &home, &["worktree", "sweep"]);

    // Assert: left alone, on disk, still registered, never named in the
    // report.
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    assert!(
        fs::symlink_metadata(&wu_path).is_ok(),
        "the worktree the sweep runs from should never be removed: {}",
        wu_path.display()
    );
    assert!(
        still_registered(&repo_root, &wu_path),
        "the worktree the sweep runs from should still be a registered git worktree"
    );
    let stdout = stdout_of(&out);
    assert!(
        !stdout.contains(&wu_path.to_string_lossy().into_owned()),
        "the sweep should not report on the worktree it runs from at all, got: {stdout}"
    );

    let _ = fs::remove_dir_all(&container);
}
