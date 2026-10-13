// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Binary-spawn tests for `playbook path <kind>`, resolving the absolute
//! path to a repo storage directory (`plans`/`implement`/`worktrees`/`handoff`)
//! under `~/.config/playbook/repos/<owner>/<repo>/`, the same from every
//! worktree.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    repo: PathBuf,
    home: PathBuf,
}

impl Fixture {
    /// A scratch git repo with an `origin` remote, plus its own scratch `$HOME`.
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "playbook-path-cmd-{tag}-{}-{n}",
            playbook::testing::run_id()
        ));
        fs::create_dir_all(&dir).expect("scratch repo should be creatable");
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&dir)
            .status()
            .expect("git init should run");
        assert!(init.success(), "git init should succeed");
        let remote = Command::new("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/test-owner/test-repo.git",
            ])
            .current_dir(&dir)
            .status()
            .expect("git remote add should run");
        assert!(remote.success(), "git remote add should succeed");
        let repo = dir.canonicalize().expect("scratch repo should resolve");

        let home = std::env::temp_dir().join(format!(
            "playbook-path-cmd-home-{tag}-{}-{n}",
            playbook::testing::run_id()
        ));
        fs::create_dir_all(&home).expect("scratch home should be creatable");

        Self { repo, home }
    }

    /// No `origin` remote, so worktree scoping cannot resolve.
    fn new_without_origin(tag: &str) -> Self {
        let f = Self::new(tag);
        let remove = Command::new("git")
            .args(["remote", "remove", "origin"])
            .current_dir(&f.repo)
            .status()
            .expect("git remote remove should run");
        assert!(remove.success(), "git remote remove should succeed");
        f
    }

    fn run(&self, kind: &str) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(["path", kind])
            .current_dir(&self.repo)
            .env("HOME", &self.home)
            .output()
            .expect("playbook binary should spawn")
    }

    fn repo_base(&self) -> PathBuf {
        self.home
            .join(".config")
            .join("playbook")
            .join("repos")
            .join("test-owner")
            .join("test-repo")
    }

    fn expected_dir(&self, dir_name: &str) -> PathBuf {
        self.repo_base().join(dir_name)
    }
}

/// Mirrors `paths::worktree_id`'s slugify rule independently, not by
/// calling the production function, so this stays a real check. The old
/// worktree-scoped folders were named with it.
fn worktree_id(repo: &std::path::Path) -> String {
    repo.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn stdout_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn every_kind_prints_its_repo_subdirectory() {
    for (kind, dir_name) in [
        ("plans", "plans"),
        ("implement", "implement"),
        ("worktrees", "worktrees"),
        ("handoff", "handoff"),
    ] {
        // Arrange
        let f = Fixture::new(&format!("kind-{kind}"));

        // Act
        let out = f.run(kind);

        // Assert
        assert_eq!(
            out.status.code(),
            Some(0),
            "expected exit 0 for {kind}: {}",
            stderr_of(&out)
        );
        let printed = stdout_of(&out);
        let lines: Vec<&str> = printed.lines().collect();
        assert_eq!(
            lines.len(),
            1,
            "stdout should be exactly one line for {kind}: {printed:?}"
        );
        assert_eq!(
            PathBuf::from(lines[0]),
            f.expected_dir(dir_name),
            "unexpected path for kind {kind}"
        );
    }
}

#[test]
fn errors_when_worktree_scoping_cannot_resolve() {
    // Arrange: no `origin` remote configured.
    let f = Fixture::new_without_origin("no-origin");

    // Act
    let out = f.run("plans");

    // Assert
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected a hard error, not a silent repo-local fallback: {}",
        stderr_of(&out)
    );
    assert!(
        stdout_of(&out).is_empty(),
        "no path should be printed on a resolution error"
    );
    assert!(
        stderr_of(&out).contains("playbook path:"),
        "got: {}",
        stderr_of(&out)
    );
}

#[test]
fn rejects_designs_as_an_unknown_kind() {
    // Arrange
    let f = Fixture::new("designs-removed");

    // Act
    let out = f.run("designs");

    // Assert
    assert_ne!(
        out.status.code(),
        Some(0),
        "designs should no longer be a valid path kind: {}",
        stderr_of(&out)
    );
}

#[test]
fn data_kind_prints_the_data_folder_without_an_origin_remote() {
    // Arrange
    let f = Fixture::new_without_origin("data-kind");

    // Act
    let out = f.run("data");

    // Assert
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert_eq!(
        stdout_of(&out).trim(),
        f.home
            .join(".config")
            .join("playbook")
            .join("data")
            .display()
            .to_string()
    );
}

fn write(path: &std::path::Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn a_linked_worktree_prints_the_same_folder_as_the_main_checkout() {
    // Arrange
    let f = Fixture::new("linked");
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&f.repo)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", stderr_of(&out));
    };
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=T",
        "commit",
        "--allow-empty",
        "-m",
        "x",
    ]);
    let linked = f.repo.with_file_name(format!(
        "{}-linked",
        f.repo.file_name().unwrap().to_string_lossy()
    ));
    git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "side",
        linked.to_str().unwrap(),
    ]);

    // Act
    let main_out = f.run("plans");
    let linked_out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["path", "plans"])
        .current_dir(&linked)
        .env("HOME", &f.home)
        .output()
        .unwrap();

    // Assert
    assert_eq!(stdout_of(&main_out), stdout_of(&linked_out));
    assert_eq!(
        PathBuf::from(stdout_of(&main_out).trim()),
        f.expected_dir("plans")
    );
}

#[test]
fn plans_from_an_old_worktree_folder_are_copied_once_and_the_originals_stay() {
    // Arrange
    let f = Fixture::new("migrate-plans");
    let old = f.repo_base().join(worktree_id(&f.repo)).join("plans");
    write(&old.join("topic.md"), "the plan");
    write(&old.join("topic.checkpoint.md"), "notes");

    // Act
    let first = f.run("plans");
    fs::remove_file(f.expected_dir("plans").join("topic.checkpoint.md")).unwrap();
    let second = f.run("plans");

    // Assert
    assert_eq!(first.status.code(), Some(0), "{}", stderr_of(&first));
    assert_eq!(
        fs::read_to_string(f.expected_dir("plans").join("topic.md")).unwrap(),
        "the plan"
    );
    assert!(
        stderr_of(&first).contains("copied 2 file(s)"),
        "{}",
        stderr_of(&first)
    );
    assert!(old.join("topic.md").is_file(), "the original must stay");
    assert!(old.join("MOVED.txt").is_file());
    assert!(
        !f.expected_dir("plans").join("topic.checkpoint.md").exists(),
        "a second run must not bring the file back"
    );
    assert!(stderr_of(&second).is_empty(), "{}", stderr_of(&second));
}

#[test]
fn live_worktree_checkouts_are_not_copied() {
    // Arrange
    let f = Fixture::new("migrate-worktrees");
    let old = f.repo_base().join(worktree_id(&f.repo)).join("worktrees");
    write(&old.join("branch/file.rs"), "code");

    // Act
    let out = f.run("worktrees");

    // Assert
    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert!(!f.expected_dir("worktrees").join("branch").exists());
    assert!(old.join("branch/file.rs").is_file());
}

#[test]
fn handoff_kind_prints_the_old_shared_folder_without_an_origin_remote() {
    let f = Fixture::new_without_origin("handoff-no-origin");

    let out = f.run("handoff");

    assert_eq!(out.status.code(), Some(0), "{}", stderr_of(&out));
    assert_eq!(
        PathBuf::from(stdout_of(&out).trim()),
        f.home.join(".config/playbook/runtime/handoff")
    );
}
