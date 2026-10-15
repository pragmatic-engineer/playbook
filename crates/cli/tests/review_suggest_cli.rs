// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook review suggest` on a local branch: no network, no `gh`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A scratch home, so neither the user's config nor their global git settings reach a test.
fn home(dir: &Path) -> PathBuf {
    let home = dir.join("home");
    fs::create_dir_all(&home).unwrap();
    home
}

fn isolate(cmd: &mut Command, dir: &Path) {
    cmd.env("HOME", home(dir))
        .env("XDG_CONFIG_HOME", home(dir).join(".config"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
}

fn git(dir: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    isolate(&mut cmd, dir);
    let out = cmd.arg("-C").arg(dir).args(args).output().expect("git");
    assert!(out.status.success(), "git {args:?}");
}

/// A repo with `main`, then a `work` branch that writes each `(path, lines)`.
fn repo(name: &str, files: &[(&str, usize)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pb-suggest-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "t"]);
    git(&dir, &["config", "user.email", "t@example.com"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    fs::write(dir.join("base.txt"), "base\n").unwrap();
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    git(&dir, &["switch", "-q", "-c", "work"]);
    for (path, lines) in files {
        let full = dir.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, "x\n".repeat(*lines)).unwrap();
    }
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-q", "--allow-empty", "-m", "work"]);
    dir
}

fn suggest(dir: &Path, extra: &[&str]) -> (i32, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_playbook"));
    isolate(&mut cmd, dir);
    let out = cmd
        .args(["review", "suggest", "--dir"])
        .arg(dir)
        .args(if extra.iter().any(|a| a.starts_with("--base")) {
            vec![]
        } else {
            vec!["--base", "main"]
        })
        .args(extra)
        .output()
        .expect("playbook");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    )
}

#[test]
fn a_small_code_change_is_a_high_confidence_quick() {
    let dir = repo("small", &[("src/a.rs", 10)]);
    let (code, out, _) = suggest(&dir, &[]);
    assert_eq!(code, 0);
    assert!(
        out.starts_with("recommendation=quick\nconfidence=high\n"),
        "{out}"
    );
    assert!(
        out.contains("reason: Small change: 10 lines in 1 files."),
        "{out}"
    );
}

#[test]
fn a_workflow_change_is_a_high_confidence_deep_in_json() {
    let dir = repo("wf", &[(".github/workflows/ci.yml", 3)]);
    let (code, out, _) = suggest(&dir, &["--json"]);
    assert_eq!(code, 0);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["recommendation"], "deep");
    assert_eq!(v["confidence"], "high");
    assert_eq!(v["rules"][0], "workflows");
    assert_eq!(v["signals"]["areas"][0], "workflows");
}

#[test]
fn a_close_call_is_low_confidence() {
    let dir = repo("mid", &[("src/a.rs", 100), ("tests/a.rs", 50)]);
    let (_, out, _) = suggest(&dir, &[]);
    assert!(
        out.starts_with("recommendation=quick\nconfidence=low\n"),
        "{out}"
    );
}

#[test]
fn no_changes_and_a_bad_base_fail_with_a_message() {
    let dir = repo("none", &[]);
    let (code, _, err) = suggest(&dir, &[]);
    assert_eq!(code, 1);
    assert!(err.contains("no changed files"), "{err}");
    let (code, _, err) = suggest(&dir, &["--base", "nope"]);
    assert_eq!(code, 1);
    assert!(err.contains("not found"), "{err}");
    let (code, _, err) = suggest(&dir, &["--base=-x"]);
    assert_eq!(code, 1);
    assert!(err.contains("as an option"), "{err}");
}

#[test]
fn a_config_key_moves_the_cut() {
    let dir = repo("cfg", &[("src/a.rs", 10)]);
    let mut set = Command::new(env!("CARGO_BIN_EXE_playbook"));
    isolate(&mut set, &dir);
    let out = set
        .args([
            "config",
            "set",
            "--global",
            "review.suggest.quickLines",
            "5",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (_, out, _) = suggest(&dir, &[]);
    assert!(
        out.starts_with("recommendation=quick\nconfidence=low\n"),
        "{out}"
    );
}
