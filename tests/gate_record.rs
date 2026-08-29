// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Binary-spawn tests for `playbook gate record`. These prove the CLI-level
//! half of the plan's claims that unit tests on `record::run` alone cannot:
//! a bad input must exit non-zero AND leave the database untouched, and
//! re-recording the same `(plan_slug, phase)` must overwrite, not
//! duplicate. Both need a real DB file and exit code to check against.

use playbook::gate::db;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    repo: PathBuf,
}

impl Fixture {
    /// A scratch git repo: `gate record` resolves the DB path via
    /// `git rev-parse --show-toplevel` from cwd, so every fixture needs to
    /// actually be a git repository, not just a plain directory.
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "playbook-gate-record-{tag}-{}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("scratch repo should be creatable");
        let init = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&dir)
            .status()
            .expect("git init should run");
        assert!(init.success(), "git init should succeed");

        Self { repo: dir }
    }

    fn write_input(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.repo.join(name);
        fs::write(&path, contents).expect("input fixture should write");
        path
    }

    fn run(
        &self,
        plan_slug: &str,
        command: &str,
        phase: &str,
        input: &PathBuf,
    ) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(["gate", "record", plan_slug, command, phase])
            .arg(input)
            .current_dir(&self.repo)
            .output()
            .expect("playbook binary should spawn")
    }

    fn db_path(&self) -> PathBuf {
        self.repo.join(".claude").join("state.db")
    }
}

fn stderr_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn no_verdict_line_exits_nonzero_and_writes_nothing() {
    // Arrange
    let f = Fixture::new("no-verdict");
    let input = f.write_input("report.md", "Nothing conclusive to report here.");

    // Act
    let out = f.run("plan-a", "spec-review", "spec", &input);

    // Assert
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1, got {:?}: {}",
        out.status.code(),
        stderr_of(&out)
    );
    assert!(
        stderr_of(&out).contains("gate record:"),
        "got: {}",
        stderr_of(&out)
    );
    let row = db::open_db(&f.db_path())
        .and_then(|conn| db::query_phase(&conn, "plan-a", "spec"))
        .expect("open/query should not error even though nothing was ever written");
    assert!(
        row.is_none(),
        "no row should have been written for an input with no VERDICT line"
    );
}

#[test]
fn invalid_verdict_value_exits_nonzero_and_writes_nothing() {
    // Arrange
    let f = Fixture::new("invalid-verdict");
    let input = f.write_input("report.md", "VERDICT: PASSING");

    // Act
    let out = f.run("plan-a", "spec-review", "spec", &input);

    // Assert
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1, got {:?}: {}",
        out.status.code(),
        stderr_of(&out)
    );
    let row = db::open_db(&f.db_path())
        .and_then(|conn| db::query_phase(&conn, "plan-a", "spec"))
        .expect("open/query should not error even though nothing was ever written");
    assert!(
        row.is_none(),
        "a value outside the 4 keywords must not be recorded"
    );
}

#[test]
fn re_recording_same_plan_and_phase_overwrites_not_duplicates() {
    // Arrange
    let f = Fixture::new("re-record");
    let first_input = f.write_input("first.md", "VERDICT: WARN");
    let second_input = f.write_input("second.md", "VERDICT: PASS");

    // Act
    let first = f.run("plan-a", "spec-review", "spec", &first_input);
    assert_eq!(
        first.status.code(),
        Some(0),
        "first record should succeed: {}",
        stderr_of(&first)
    );
    let second = f.run("plan-a", "spec-review-2", "spec", &second_input);

    // Assert
    assert_eq!(
        second.status.code(),
        Some(0),
        "second record should succeed: {}",
        stderr_of(&second)
    );
    let conn = db::open_db(&f.db_path()).expect("open db after two records");
    let row = db::query_phase(&conn, "plan-a", "spec")
        .expect("query should not error")
        .expect("row should exist after recording");
    assert_eq!(
        row.verdict, "PASS",
        "only the latest verdict should be present"
    );
}
