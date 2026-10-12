// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The SessionStart migration check. With the binary it is
//! `playbook hook migration-check`; with no binary it is the plugin's
//! `bin/playbook` shim, which prints the same warning, installs nothing and
//! never fails the session. Both run against a throwaway HOME.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

/// The shim finds a Homebrew binary in a fixed system path, so the no-binary
/// tests cannot run on a machine that has one.
fn has_system_binary() -> bool {
    [
        "/opt/homebrew/bin/playbook",
        "/usr/local/bin/playbook",
        "/home/linuxbrew/.linuxbrew/bin/playbook",
    ]
    .iter()
    .any(|p| Path::new(p).exists())
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn home_with(settings: Option<&str>) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!(
        "playbook-migcheck-{}-{n}",
        playbook::testing::run_id()
    ));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(home.join(".claude")).unwrap();
    if let Some(text) = settings {
        fs::write(home.join(".claude/settings.json"), text).unwrap();
    }
    home
}

fn binary(home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["hook", "migration-check"])
        .env("HOME", home)
        .output()
        .unwrap()
}

/// The shim with no `playbook` anywhere it looks: a bare HOME and a PATH of
/// system directories only.
fn shim(home: &Path) -> Output {
    Command::new(Path::new(env!("PLAYBOOK_REPO_ROOT")).join("bin/playbook"))
        .args(["hook", "migration-check"])
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap()
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

#[test]
fn wired_settings_stay_silent() {
    let home = home_with(Some(
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"playbook hook session-init"}]}]}}"#,
    ));
    let o = binary(&home);
    assert!(o.status.success());
    assert!(out(&o).is_empty(), "got: {}", out(&o));
}

#[test]
fn unwired_settings_warn_to_run_setup() {
    let home = home_with(Some(r#"{"hooks":{"PreToolUse":[]}}"#));
    let o = binary(&home);
    assert!(o.status.success());
    assert!(out(&o).contains(r#""hookEventName":"SessionStart""#));
    assert!(out(&o).contains("Run /playbook:setup"));
}

#[test]
fn missing_or_malformed_settings_exit_zero_without_stderr_noise() {
    for settings in [None, Some("not even json {\n")] {
        let o = binary(&home_with(settings));
        assert!(o.status.success());
        assert!(
            o.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
}

#[test]
fn the_shim_without_a_binary_warns_and_installs_nothing() {
    if has_system_binary() {
        return;
    }
    let home = home_with(None);
    let o = shim(&home);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(out(&o).contains("Run /playbook:setup"));
    assert!(o.stderr.is_empty(), "no installer output expected");
    assert!(!home.join(".local").exists(), "the shim must not install");
}

#[test]
fn the_shim_and_the_binary_print_the_same_warning() {
    if has_system_binary() {
        return;
    }
    let home = home_with(None);
    let shim_json: serde_json::Value = serde_json::from_str(out(&shim(&home)).trim()).unwrap();
    let binary_json: serde_json::Value = serde_json::from_str(out(&binary(&home)).trim()).unwrap();
    assert_eq!(shim_json, binary_json);
}
