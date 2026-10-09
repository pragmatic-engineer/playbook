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

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn usage_json_prints_totals_and_the_live_view_without_charts() {
    let home = Home::new("json");

    let out = home.run(&["usage", "--json", "--range", "all"]);

    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let doc: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(doc["totals"]["messages"], 3);
    assert!((doc["totals"]["cost_usd"].as_f64().unwrap() - 0.0973221).abs() < 1e-9);
    assert!(doc["live"]["active"].is_array());
    assert!(doc.get("charts").is_none());
    assert!(doc["live"].get("burn_chart").is_none());
}

#[test]
fn usage_summary_prints_the_text_report() {
    let home = Home::new("summary-flag");

    let out = home.run(&["usage", "--summary"]);

    assert!(out.status.success());
    assert!(stdout(&out).contains("3 messages, $0.0973 estimated cost"));
}

#[test]
fn an_unknown_range_is_refused_with_the_valid_values() {
    let home = Home::new("bad-range");

    let out = home.run(&["usage", "--json", "--range", "7d"]);

    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("unknown range `7d`"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn conflicting_flags_are_refused() {
    let home = Home::new("conflict");

    for args in [
        ["usage", "--json", "--summary"],
        ["usage", "--web", "--json"],
        ["usage", "--summary", "--web"],
    ] {
        assert!(!home.run(&args).status.success(), "{args:?}");
    }
}

#[test]
fn usage_ingest_still_works_and_warns_that_it_is_deprecated() {
    let home = Home::new("ingest-deprecated");

    let out = home.run(&["usage", "ingest"]);

    assert!(out.status.success());
    assert!(stdout(&out).contains("usage ingest: added 3 usage events"));
    let err = stderr(&out);
    assert!(
        err.contains("deprecated") && err.contains("v0.22.0"),
        "{err}"
    );
    assert!(err.contains("playbook usage --summary"), "{err}");
}
