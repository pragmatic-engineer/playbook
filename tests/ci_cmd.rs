// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `playbook ci` through the real binary: no stdin, an empty HOME, and
//! `CI=true`, exactly how a pipeline runs it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ci-cmd-{}-{tag}-{n}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

struct Run {
    stdout: String,
    code: i32,
}

/// Runs `playbook ci` from `cwd` with an empty HOME, `CI=true`, and stdin closed.
fn ci(cwd: &Path, home: &Path, args: &[&str]) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .arg("ci")
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("CI", "true")
        .stdin(Stdio::null())
        .output()
        .expect("playbook spawns");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        code: out.status.code().unwrap_or(-1),
    }
}

fn repo_root() -> &'static str {
    env!("CARGO_MANIFEST_DIR")
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {args:?} failed");
}

/// A tiny checkout: the plugin marker and a copy of the real `agents/`, with
/// one agent given a model the validator rejects.
fn failing_checkout(tag: &str) -> PathBuf {
    let dir = scratch(tag);
    git(&dir, &["init", "-q"]);
    fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
    fs::copy(
        Path::new(repo_root()).join(".claude-plugin/plugin.json"),
        dir.join(".claude-plugin/plugin.json"),
    )
    .unwrap();
    fs::create_dir_all(dir.join("agents")).unwrap();
    for entry in fs::read_dir(Path::new(repo_root()).join("agents"))
        .unwrap()
        .flatten()
    {
        fs::copy(entry.path(), dir.join("agents").join(entry.file_name())).unwrap();
    }
    let critic = dir.join("agents/critic.md");
    let text = fs::read_to_string(&critic).unwrap();
    assert!(text.contains("model: sonnet"), "fixture assumption");
    fs::write(&critic, text.replace("model: sonnet", "model: gpt")).unwrap();
    git(&dir, &["add", "-A"]);
    dir
}

#[test]
fn this_repo_passes_every_check_from_an_unrelated_cwd() {
    let home = scratch("home-pass");

    let run = ci(&scratch("elsewhere"), &home, &["--dir", repo_root()]);

    assert_eq!(run.code, 0, "{}", run.stdout);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert!(
        lines[0].starts_with("PASS manifest: check-manifest: OK"),
        "{lines:?}"
    );
    assert!(lines[1].starts_with("PASS agents: "), "{lines:?}");
    assert!(lines[2].starts_with("PASS settings: "), "{lines:?}");
    assert_eq!(lines[3], "ci: 3 passed, 0 failed, 0 skipped");
    assert_eq!(
        fs::read_dir(&home).unwrap().count(),
        0,
        "nothing is written to HOME"
    );
}

#[test]
fn one_failing_check_exits_one_and_names_the_problem() {
    let checkout = failing_checkout("fail");

    let run = ci(&checkout, &scratch("home-fail"), &[]);

    assert_eq!(run.code, 1, "{}", run.stdout);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert!(lines[0].starts_with("PASS manifest: "), "{lines:?}");
    assert!(lines[1].starts_with("FAIL agents: "), "{lines:?}");
    assert!(run.stdout.contains("model 'gpt'"), "{}", run.stdout);
    assert!(run
        .stdout
        .contains("SKIP settings: no shared settings templates"));
    assert_eq!(*lines.last().unwrap(), "ci: 1 passed, 1 failed, 1 skipped");
}

#[test]
fn outside_a_checkout_everything_is_skipped_with_exit_zero() {
    let run = ci(&scratch("empty"), &scratch("home-skip"), &[]);

    assert_eq!(run.code, 0);
    assert_eq!(
        run.stdout,
        "SKIP manifest: not a playbook checkout\n\
         SKIP agents: not a playbook checkout\n\
         SKIP settings: not a playbook checkout\n\
         ci: 0 passed, 0 failed, 3 skipped"
    );
}

#[test]
fn json_output_is_one_object_with_the_exact_field_names() {
    let checkout = failing_checkout("json");

    let run = ci(&checkout, &scratch("home-json"), &["--json"]);

    assert_eq!(run.code, 1);
    assert_eq!(
        run.stdout.lines().count(),
        1,
        "nothing but the JSON: {}",
        run.stdout
    );
    let value: serde_json::Value = serde_json::from_str(&run.stdout).unwrap();
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["checks", "failed", "passed", "skipped"]);
    assert_eq!(
        (
            value["passed"].as_u64(),
            value["failed"].as_u64(),
            value["skipped"].as_u64()
        ),
        (Some(1), Some(1), Some(1))
    );
    let checks = value["checks"].as_array().unwrap();
    let names: Vec<&str> = checks.iter().map(|c| c["name"].as_str().unwrap()).collect();
    let statuses: Vec<&str> = checks
        .iter()
        .map(|c| c["status"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["manifest", "agents", "settings"]);
    assert_eq!(statuses, ["pass", "fail", "skip"]);
    assert!(checks.iter().all(|c| c["detail"].is_string()));
}

#[test]
fn a_missing_dir_is_an_error_not_a_skip() {
    let run = ci(
        &scratch("cwd"),
        &scratch("home-bad"),
        &["--dir", "/definitely/not/here"],
    );

    assert_eq!(run.code, 1);
    assert_eq!(run.stdout, "");
}
