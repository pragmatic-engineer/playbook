// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook gate snapshot`, and `gate record` / `gate check` finding their
//! source files by name when `--source` is left out. One scratch repo per
//! test, with its own `$HOME`, so nothing touches the real machine.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Fixture {
    repo: PathBuf,
    home: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "pb-gate-snap-cli-{}-{tag}",
            playbook::testing::run_id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        let home = dir.join("home");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&home).unwrap();
        for args in [
            vec!["init", "-q"],
            vec![
                "remote",
                "add",
                "origin",
                "https://github.com/test-owner/test-repo.git",
            ],
        ] {
            assert!(Command::new("git")
                .args(&args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        let repo = repo.canonicalize().unwrap();
        Fixture { repo, home }
    }

    fn run(&self, args: &[&str], stdin: Option<&str>) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(args)
            .current_dir(&self.repo)
            .env("HOME", &self.home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(text) = stdin {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        } else {
            drop(child.stdin.take());
        }
        child.wait_with_output().unwrap()
    }

    fn plans_dir(&self) -> PathBuf {
        let out = self.run(&["path", "plans", "--create"], None);
        PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
    }
}

fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn snapshot_copies_the_draft_for_each_phase_and_prints_the_paths() {
    let fx = Fixture::new("copy");
    let plans = fx.plans_dir();
    fs::write(plans.join("p.gate-source.md"), "draft v1").unwrap();

    let out = fx.run(
        &["gate", "snapshot", "p", "adversarial", "test-review"],
        None,
    );

    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let printed = text(&out);
    assert_eq!(printed.lines().count(), 2, "{printed}");
    for phase in ["adversarial", "test-review"] {
        let copy = plans.join(format!("p.gate-source.{phase}.md"));
        assert_eq!(fs::read_to_string(copy).unwrap(), "draft v1");
    }
}

#[test]
fn snapshot_without_a_draft_fails_and_copies_nothing() {
    let fx = Fixture::new("nodraft");
    let plans = fx.plans_dir();

    let out = fx.run(&["gate", "snapshot", "p", "fact-check"], None);

    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("write the current draft"));
    assert!(!Path::new(&plans.join("p.gate-source.fact-check.md")).exists());
}

#[test]
fn record_and_check_find_their_sources_by_name_and_a_changed_draft_is_stale() {
    let fx = Fixture::new("flow");
    let plans = fx.plans_dir();
    fs::write(plans.join("p.gate-source.md"), "draft v1").unwrap();
    assert!(fx
        .run(&["gate", "snapshot", "p", "fact-check"], None)
        .status
        .success());

    let rec = fx.run(
        &["gate", "record", "p", "plan", "fact-check", "-"],
        Some("All good.\nVERDICT: PASS\n"),
    );
    assert!(
        rec.status.success(),
        "{}",
        String::from_utf8_lossy(&rec.stderr)
    );

    let ok = fx.run(&["gate", "check", "p", "plan", "fact-check"], None);
    assert!(
        ok.status.success(),
        "{} {}",
        text(&ok),
        String::from_utf8_lossy(&ok.stderr)
    );

    fs::write(plans.join("p.gate-source.md"), "draft v2").unwrap();
    let stale = fx.run(&["gate", "check", "p", "plan", "fact-check"], None);
    assert_eq!(stale.status.code(), Some(1));
    let said = String::from_utf8_lossy(&stale.stderr).to_uppercase();
    assert!(said.contains("STALE"), "{said}");
}

#[test]
fn an_explicit_source_still_wins() {
    let fx = Fixture::new("explicit");
    let src = fx.repo.join("spec.md");
    fs::write(&src, "spec").unwrap();
    let s = src.to_str().unwrap();

    let rec = fx.run(
        &[
            "gate",
            "record",
            "q",
            "implement",
            "fact-check",
            "-",
            "--source",
            s,
        ],
        Some("VERDICT: PASS"),
    );
    assert!(rec.status.success());
    let ok = fx.run(
        &[
            "gate",
            "check",
            "q",
            "implement",
            "fact-check",
            "--source",
            s,
        ],
        None,
    );
    assert!(ok.status.success(), "{}", text(&ok));
}

#[test]
fn the_plan_and_implement_commands_use_the_snapshot_and_stdin_flow() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let plan = fs::read_to_string(root.join("commands/plan.md")).unwrap();
    assert!(plan.contains("playbook gate snapshot <topic-slug>"));
    assert!(
        !plan.contains("write the plan draft's current full content"),
        "the draft is written once to the shared file, then copied by the command"
    );
    assert!(
        !plan.contains("**Run Phase 1 inline**"),
        "Phase 1 is dispatched like the other two; inline is only the fallback"
    );
    for file in [
        "commands/plan.md",
        "commands/implement.md",
        "commands/adr.md",
    ] {
        let text = fs::read_to_string(root.join(file)).unwrap();
        assert!(
            !text.contains("<that file>") && !text.contains("<that-file>"),
            "{file}"
        );
        assert!(text.contains("gate record"), "{file}");
    }
}
