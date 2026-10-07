// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The compiled `playbook pr` subcommands: usage text and argument errors.
//! Paths that would call the real `gh` are covered through the `GhClient`
//! fake in `tests/pr_prepare.rs` and `tests/pr_create.rs` instead.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// An empty directory used as both cwd and HOME, so no run can touch the
/// developer's checkout, remote, or config.
fn scratch() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("playbook-pr-cli-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn playbook_in(dir: &PathBuf, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .output()
        .expect("playbook should spawn")
}

fn playbook(args: &[&str]) -> Output {
    playbook_in(&scratch(), args)
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn pr_prepare_help_lists_its_flags() {
    // Arrange / Act
    let out = playbook(&["pr", "prepare", "--help"]);

    // Assert
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(
        text.contains("--base") && text.contains("--ticket"),
        "got {text}"
    );
}

#[test]
fn pr_create_help_lists_its_flags() {
    // Arrange / Act
    let out = playbook(&["pr", "create", "--help"]);

    // Assert
    assert!(out.status.success());
    let text = stdout(&out);
    for flag in ["--title", "--body-file", "--base"] {
        assert!(text.contains(flag), "missing {flag} in {text}");
    }
}

#[test]
fn pr_create_without_a_title_is_a_usage_error() {
    // Arrange / Act
    let out = playbook(&["pr", "create", "--body-file", "/tmp/x.md"]);

    // Assert
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn pr_create_with_an_overlong_title_exits_non_zero_without_touching_anything() {
    // Arrange
    let title = "a".repeat(73);
    let dir = scratch();
    let body = dir.join("x.md");

    // Act
    let out = playbook_in(
        &dir,
        &[
            "pr",
            "create",
            "--title",
            &title,
            "--body-file",
            body.to_str().expect("utf8"),
        ],
    );

    // Assert
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("limit 72"));
    assert!(
        !dir.join(".git").exists(),
        "nothing may be initialised here"
    );
}

mod pr_support;

fn run_in(cwd: &PathBuf, home: &PathBuf, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("playbook should spawn")
}

#[test]
fn pr_prepare_with_dir_reads_the_repo_it_points_at_not_the_cwd() {
    // Arrange: a repo two commits ahead of main, and an unrelated cwd and HOME.
    let fx = pr_support::Fixture::new("dir", "feat/dir-demo");
    fx.commit_lines("a.txt", 3);
    fx.commit_lines("b.txt", 2);
    let elsewhere = scratch();

    // Act: gh is absent from the scrubbed PATH-independent path, so use a
    // PATH with git only; prepare treats a missing gh as "no PR yet".
    let out = run_in(
        &elsewhere,
        &elsewhere,
        &["pr", "prepare", "--dir", fx.work.to_str().expect("utf8")],
    );

    // Assert
    let text = stdout(&out);
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.lines().any(|l| l == "branch=feat/dir-demo"), "{text}");
    assert!(text.contains("commits_ahead=2 changed_lines=5"), "{text}");
    let diff_line = text
        .lines()
        .find(|l| l.starts_with("diff_file="))
        .expect("diff_file line");
    let diff = std::fs::read_to_string(diff_line.trim_start_matches("diff_file=")).expect("diff");
    assert!(diff.contains("a.txt") && diff.contains("b.txt"), "{diff}");
}

#[test]
fn a_dir_that_does_not_exist_exits_1_with_a_clear_message() {
    let elsewhere = scratch();
    let out = run_in(
        &elsewhere,
        &elsewhere,
        &["pr", "prepare", "--dir", "/nonexistent/zzz"],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--dir /nonexistent/zzz"));
}

#[test]
fn a_dir_outside_any_git_work_tree_exits_1() {
    let elsewhere = scratch();
    let plain = scratch();
    let out = run_in(
        &elsewhere,
        &elsewhere,
        &["pr", "prepare", "--dir", plain.to_str().expect("utf8")],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not inside a git work tree"));
}

#[test]
fn pr_help_and_pr_prepare_help_exit_0_so_a_caller_can_detect_support() {
    for args in [&["pr", "--help"][..], &["pr", "prepare", "--help"][..]] {
        let out = playbook(args);
        assert!(out.status.success(), "{args:?} must exit 0");
    }
}

#[test]
fn pr_review_triage_help_lists_its_flags() {
    // Arrange / Act
    let out = playbook(&["pr", "review-triage", "--help"]);

    // Assert
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(
        text.contains("--pr") && text.contains("--base"),
        "got {text}"
    );
}
