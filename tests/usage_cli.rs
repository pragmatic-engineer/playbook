// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Binary-level tests for `playbook usage` and `playbook usage ingest`, run
//! against a scratch `$HOME` seeded from the committed fixtures. The real
//! `~/.claude` is never read.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/usage")
}

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("playbook-usage-{}-{tag}-{n}", std::process::id()));
        let project = dir.join(".claude/projects/proj-one");
        fs::create_dir_all(&project).unwrap();
        fs::copy(
            fixtures().join("usage/proj-one/s1.jsonl"),
            project.join("s1.jsonl"),
        )
        .unwrap();
        fs::copy(
            fixtures().join("account/claude.json"),
            dir.join(".claude.json"),
        )
        .unwrap();
        Home(dir.canonicalize().unwrap())
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(args)
            .env("HOME", &self.0)
            .output()
            .unwrap()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(Path::new(&self.0));
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn bare_usage_ingests_and_prints_a_summary_with_fixture_values() {
    let home = Home::new("summary");

    let out = home.run(&["usage"]);

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = stdout(&out);
    assert!(
        text.contains("3 messages, $0.0973 estimated cost"),
        "{text}"
    );
    let row = text
        .lines()
        .find(|l| l.contains("claude-sonnet-5"))
        .unwrap();
    assert!(row.contains("770") && row.contains("0.0863"), "{row}");
    assert!(text.contains("dev@example.com"), "{text}");
}

#[test]
fn usage_ingest_reports_counts_and_a_second_run_adds_nothing() {
    let home = Home::new("ingest");

    let first = home.run(&["usage", "ingest"]);
    let second = home.run(&["usage", "ingest"]);

    assert!(first.status.success() && second.status.success());
    assert_eq!(
        stdout(&first).trim(),
        "usage ingest: added 3 usage events and 0 tool events (0 already stored)"
    );
    // The watermark is inclusive, so the newest event is re-read and skipped.
    assert_eq!(
        stdout(&second).trim(),
        "usage ingest: added 0 usage events and 0 tool events (1 already stored)"
    );
}

#[test]
fn a_home_with_no_claude_data_prints_the_empty_message_and_exits_zero() {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("playbook-usage-{}-empty-{n}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .arg("usage")
        .env("HOME", &dir)
        .output()
        .unwrap();

    assert!(out.status.success());
    assert!(stdout(&out).contains("No usage recorded yet"));
    let _ = fs::remove_dir_all(dir);
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[test]
fn usage_ingest_reads_codex_rollouts_when_the_sessions_dir_exists() {
    let home = Home::new("codex");
    copy_dir(&fixtures().join("codex"), &home.0.join(".codex/sessions"));

    let first = home.run(&["usage", "ingest"]);
    let second = home.run(&["usage", "ingest"]);

    assert!(first.status.success() && second.status.success());
    assert_eq!(
        stdout(&first).trim(),
        "usage ingest: added 7 usage events and 0 tool events (0 already stored)"
    );
    assert!(
        stdout(&second).starts_with("usage ingest: added 0 usage events and 0 tool events"),
        "{}",
        stdout(&second)
    );
}
