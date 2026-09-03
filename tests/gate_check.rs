// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Binary-spawn tests for `playbook gate check`. `gate check` needs
//! pre-existing rows to query, so each fixture seeds them directly through
//! `playbook::gate::db` (the same in-process seeding precedent
//! `tests/gate_db.rs` uses) before spawning the binary for the invocation
//! under test.

use playbook::gate::db;
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
    /// A scratch git repo with an `origin` remote, plus its own scratch
    /// `$HOME`, so every fixture is isolated from the real machine and every other fixture.
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "playbook-gate-check-{tag}-{}-{n}",
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
            "playbook-gate-check-home-{tag}-{}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&home).expect("scratch home should be creatable");

        Self { repo, home }
    }

    /// A repo fixture with no `origin` remote, so worktree scoping cannot resolve: covers the silent-fallback error path.
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

    fn db_path(&self) -> PathBuf {
        self.home
            .join(".config")
            .join("playbook")
            .join("repos")
            .join("test-owner")
            .join("test-repo")
            .join(worktree_id(&self.repo))
            .join("state.db")
    }

    /// Seed a phase row directly through the library, bypassing `gate
    /// record`'s CLI: `gate check` needs pre-existing rows to query.
    fn seed(&self, plan_slug: &str, phase: &str, verdict: &str) {
        let conn = db::open_db(&self.db_path()).expect("open db for seed");
        db::upsert_phase(
            &conn,
            plan_slug,
            phase,
            verdict,
            "evidence",
            "seed-cmd",
            "2026-01-01T00:00:00Z",
        )
        .expect("seed upsert");
    }

    fn run(&self, plan_slug: &str, command: &str, phases: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(["gate", "check", plan_slug, command])
            .args(phases)
            .current_dir(&self.repo)
            .env("HOME", &self.home)
            .output()
            .expect("playbook binary should spawn")
    }
}

/// Mirrors `paths::worktree_id`'s slugify rule independently (not by
/// calling the production function), so this stays a real check, not a tautology.
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
fn all_named_phases_pass_exits_zero() {
    // Arrange
    let f = Fixture::new("all-pass");
    f.seed("plan-a", "spec", "PASS");
    f.seed("plan-a", "impl", "PASS");

    // Act
    let out = f.run("plan-a", "gate-run", &["spec", "impl"]);

    // Assert
    assert_eq!(
        out.status.code(),
        Some(0),
        "expected exit 0, got {:?}: {}",
        out.status.code(),
        stderr_of(&out)
    );
}

#[test]
fn warn_and_pass_mix_exits_zero_and_warn_line_is_distinguishable_from_pass() {
    // Arrange
    let f = Fixture::new("warn-pass");
    f.seed("plan-a", "spec", "PASS");
    f.seed("plan-a", "impl", "WARN");

    // Act
    let out = f.run("plan-a", "gate-run", &["spec", "impl"]);

    // Assert
    assert_eq!(
        out.status.code(),
        Some(0),
        "expected exit 0: {}",
        stderr_of(&out)
    );
    let stdout = stdout_of(&out);
    let warn_line = stdout
        .lines()
        .find(|line| line.starts_with("impl:"))
        .unwrap_or_else(|| panic!("no line for impl phase in: {stdout}"));
    assert!(
        warn_line.contains("WARN"),
        "warn phase's line must contain WARN: {warn_line}"
    );
    assert!(
        !warn_line.contains("PASS"),
        "warn phase's line must NOT contain PASS: {warn_line}"
    );
}

/// Table-driven, one case per state, matching the plan's exit-code table:
/// `[(Missing, 1), (Fail, 1), (Warn, 0), (Inconclusive, 1)]`. A single named
/// phase is checked per case, seeded to that state (or left unseeded for
/// Missing).
#[test]
fn each_single_phase_state_maps_to_its_own_exit_code() {
    // Arrange
    let cases: [(&str, Option<&str>, Option<i32>); 4] = [
        ("missing", None, Some(1)),
        ("fail", Some("FAIL"), Some(1)),
        ("warn", Some("WARN"), Some(0)),
        ("inconclusive", Some("INCONCLUSIVE"), Some(1)),
    ];

    for (label, verdict, expected_code) in cases {
        let f = Fixture::new(&format!("state-{label}"));
        if let Some(v) = verdict {
            f.seed("plan-a", "phase-x", v);
        }

        // Act
        let out = f.run("plan-a", "gate-run", &["phase-x"]);

        // Assert
        assert_eq!(
            out.status.code(),
            expected_code,
            "state {label}: expected exit {expected_code:?}, got {:?}: stdout={} stderr={}",
            out.status.code(),
            stdout_of(&out),
            stderr_of(&out)
        );
    }
}

#[test]
fn fail_and_missing_phases_in_one_invocation_are_both_named_distinctly() {
    // Arrange
    let f = Fixture::new("fail-and-missing");
    f.seed("plan-a", "broken", "FAIL");
    // "unrecorded" is deliberately never seeded.

    // Act
    let out = f.run("plan-a", "gate-run", &["broken", "unrecorded"]);

    // Assert
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1: {}",
        stdout_of(&out)
    );
    let stderr = stderr_of(&out);
    assert!(
        stderr.contains("broken: FAIL"),
        "the FAIL phase must be individually named: {stderr}"
    );
    assert!(
        stderr.contains("unrecorded: MISSING"),
        "the MISSING phase must be individually named: {stderr}"
    );
}

#[test]
fn zero_phase_arguments_exits_one_with_a_pinned_message() {
    // Arrange
    let f = Fixture::new("zero-phases");

    // Act
    let out = f.run("plan-a", "gate-run", &[]);

    // Assert
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1, got {:?}: {}",
        out.status.code(),
        stdout_of(&out)
    );
    assert!(
        stderr_of(&out).contains("no phases specified"),
        "got: {}",
        stderr_of(&out)
    );
}

#[test]
fn gate_check_errors_when_worktree_scoping_cannot_resolve() {
    // Arrange: no `origin` remote configured, `gate check`'s own
    // independent path-construction call site.
    let f = Fixture::new_without_origin("no-origin");

    // Act
    let out = f.run("plan-a", "gate-run", &["spec"]);

    // Assert
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected a hard error, not a silent repo-local fallback: {}",
        stderr_of(&out)
    );
    assert!(
        !f.repo.join(".claude").join("state.db").exists(),
        "must never fall back to reading/writing a repo-local state.db"
    );
}
