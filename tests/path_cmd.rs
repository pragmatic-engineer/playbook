// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Binary-spawn tests for `playbook path <kind>`, resolving the absolute
//! path to a worktree-scoped storage directory (`plans`/`designs`/`implement`/`worktrees`).

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
            std::process::id()
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
            std::process::id()
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

    fn expected_dir(&self, dir_name: &str) -> PathBuf {
        self.home
            .join(".config")
            .join("playbook")
            .join("repos")
            .join("test-owner")
            .join("test-repo")
            .join(worktree_id(&self.repo))
            .join(dir_name)
    }
}

/// Mirrors `paths::worktree_id`'s slugify rule independently, not by
/// calling the production function, so this stays a real check.
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
fn every_kind_prints_its_worktree_scoped_subdirectory() {
    for (kind, dir_name) in [
        ("plans", "plans"),
        ("designs", "designs"),
        ("implement", "implement"),
        ("worktrees", "worktrees"),
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
