// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The compiled `playbook pr` subcommands: usage text and argument errors.
//! Paths that would call the real `gh` are covered through the `GhClient`
//! fake in `tests/pr_prepare.rs` and `tests/pr_create.rs` instead.

use std::process::{Command, Output};

fn playbook(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(args)
        .output()
        .expect("playbook should spawn")
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

    // Act
    let out = playbook(&[
        "pr",
        "create",
        "--title",
        &title,
        "--body-file",
        "/tmp/x.md",
    ]);

    // Assert
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("limit 72"));
}
