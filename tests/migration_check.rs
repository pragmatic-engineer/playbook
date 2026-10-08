// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `hooks/migration-check.sh`, the SessionStart hook that runs before
//! `playbook` is installed. It stays shell on purpose; this drives it with a
//! throwaway HOME. Wired settings stay silent; unwired, missing or malformed
//! settings never break the session.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn home_with(settings: Option<&str>) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!("playbook-migcheck-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(home.join(".claude")).unwrap();
    if let Some(text) = settings {
        fs::write(home.join(".claude/settings.json"), text).unwrap();
    }
    home
}

fn run(home: &Path) -> (i32, String, String) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("hooks/migration-check.sh");
    let out = Command::new("bash")
        .arg(script)
        .env("HOME", home)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn wired_settings_stay_silent() {
    let home = home_with(Some(
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"playbook hook session-init"}]}]}}"#,
    ));
    let (code, out, _) = run(&home);
    assert_eq!(code, 0);
    assert!(out.is_empty(), "got: {out}");
}

#[test]
fn unwired_settings_warn_to_rerun_the_installer() {
    let home = home_with(Some(
        r#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"~/.claude/hooks/other.sh"}]}]}}"#,
    ));
    let (code, out, _) = run(&home);
    assert_eq!(code, 0);
    assert!(
        out.contains(r#""hookEventName":"SessionStart""#),
        "got: {out}"
    );
    assert!(out.contains("Re-run the installer"), "got: {out}");
}

#[test]
fn a_missing_settings_file_warns_without_stderr_noise() {
    let home = home_with(None);
    let (code, out, err) = run(&home);
    assert_eq!(code, 0);
    assert!(!out.is_empty());
    assert!(err.is_empty(), "got: {err}");
}

#[test]
fn a_malformed_settings_file_exits_zero_without_stderr_noise() {
    let home = home_with(Some("not even json {\n"));
    let (code, _, err) = run(&home);
    assert_eq!(code, 0);
    assert!(err.is_empty(), "got: {err}");
}
