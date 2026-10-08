// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Binary-spawn tests for `playbook trust`: exit code, stdout, and stderr
//! against a scratch `$HOME`, never the real `~/.claude.json`.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch_home(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-trust-it-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch home is creatable");
    dir.canonicalize().expect("scratch home resolves")
}

fn trust(home: &PathBuf, path: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["trust", path])
        .env("HOME", home)
        .output()
        .expect("playbook binary runs")
}

#[test]
fn trusts_a_new_path_in_a_valid_file_and_prints_nothing() {
    // Arrange
    let home = scratch_home("valid");
    fs::write(home.join(".claude.json"), r#"{"projects":{}}"#).expect("fixture is writable");

    // Act
    let out = trust(&home, "/work/new");

    // Assert
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join(".claude.json")).expect("readable"))
            .expect("valid json");
    assert_eq!(doc["projects"]["/work/new"]["hasTrustDialogAccepted"], true);
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn missing_claude_json_exits_zero_silently_and_creates_nothing() {
    // Arrange
    let home = scratch_home("missing");

    // Act
    let out = trust(&home, "/work/new");

    // Assert
    assert!(out.status.success());
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    assert!(!home.join(".claude.json").exists());
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn malformed_json_exits_zero_and_warns_on_stderr() {
    // Arrange
    let home = scratch_home("malformed");
    fs::write(home.join(".claude.json"), "{ not json").expect("fixture is writable");

    // Act
    let out = trust(&home, "/work/new");

    // Assert
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "a trust failure must never fail the caller"
    );
    assert!(
        stderr.contains("playbook trust:") && stderr.contains("parse"),
        "got: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(home.join(".claude.json")).expect("readable"),
        "{ not json"
    );
    let _ = fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn unwritable_directory_exits_zero_and_warns_on_stderr() {
    use std::os::unix::fs::PermissionsExt;

    // Arrange
    let home = scratch_home("readonly");
    fs::write(home.join(".claude.json"), "{}").expect("fixture is writable");
    fs::set_permissions(&home, fs::Permissions::from_mode(0o555)).expect("chmod works");

    // Act
    let out = trust(&home, "/work/new");

    // Assert
    fs::set_permissions(&home, fs::Permissions::from_mode(0o755)).expect("restore works");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success());
    assert!(
        stderr.contains("playbook trust:") && stderr.contains("write"),
        "got: {stderr}"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn a_relative_path_exits_zero_warns_and_writes_nothing() {
    // Arrange
    let home = scratch_home("relative");
    fs::write(home.join(".claude.json"), "{}").expect("fixture is writable");

    // Act
    let out = trust(&home, "relative/dir");

    // Assert
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success());
    assert!(stderr.contains("must be absolute"), "got: {stderr}");
    assert_eq!(
        fs::read_to_string(home.join(".claude.json")).expect("readable"),
        "{}"
    );
    let _ = fs::remove_dir_all(&home);
}

#[test]
fn help_describes_the_path_argument() {
    // Act
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["trust", "--help"])
        .output()
        .expect("playbook binary runs");

    // Assert
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(stdout.contains("<PATH>"), "got: {stdout}");
}
