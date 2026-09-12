// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Binary-spawn tests for `playbook doctor plugin-version` and
//! `playbook doctor statusline-command`, the subcommands
//! `commands/doctor.md`'s Layer 6 and Layer 5 blocks call in place of `jq`.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A scratch JSON file holding `contents`, cleaned up on drop.
struct Fixture {
    path: PathBuf,
}

impl Fixture {
    fn new(tag: &str, contents: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "playbook-doctor-field-cmd-{tag}-{}-{n}.json",
            std::process::id()
        ));
        fs::write(&path, contents).expect("fixture file should be writable");
        Self { path }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(args)
        .output()
        .expect("playbook binary should spawn")
}

fn stdout_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn plugin_version_prints_the_version_field() {
    // Arrange
    let f = Fixture::new("plugin-version", r#"{"version": "0.15.0"}"#);

    // Act
    let out = run(&["doctor", "plugin-version", f.path.to_str().unwrap()]);

    // Assert
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout_of(&out), "0.15.0\n");
}

#[test]
fn plugin_version_prints_an_empty_line_for_a_missing_file() {
    // Arrange
    let path = std::env::temp_dir().join("playbook-doctor-field-cmd-does-not-exist.json");

    // Act
    let out = run(&["doctor", "plugin-version", path.to_str().unwrap()]);

    // Assert
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout_of(&out), "\n");
}

#[test]
fn statusline_command_prints_the_nested_field() {
    // Arrange
    let f = Fixture::new(
        "statusline-command",
        r#"{"statusLine": {"command": "~/.claude/statusline.sh"}}"#,
    );

    // Act
    let out = run(&["doctor", "statusline-command", f.path.to_str().unwrap()]);

    // Assert
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout_of(&out), "~/.claude/statusline.sh\n");
}

#[test]
fn statusline_command_prints_an_empty_line_when_not_configured() {
    // Arrange
    let f = Fixture::new("statusline-not-configured", r#"{}"#);

    // Act
    let out = run(&["doctor", "statusline-command", f.path.to_str().unwrap()]);

    // Assert
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout_of(&out), "\n");
}
