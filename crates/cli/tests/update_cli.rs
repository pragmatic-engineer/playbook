// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook update` wiring through the real binary. Nothing here touches the
//! network.

#![cfg(unix)]

use std::process::Command;

fn playbook() -> Command {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
}

#[test]
fn update_and_its_hidden_alias_share_one_help() {
    for name in ["update", "upgrade"] {
        let out = playbook().args([name, "--help"]).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{name} --help failed");
        for flag in ["--check", "--list", "--pre", "--yes"] {
            assert!(text.contains(flag), "{name} help lacks {flag}: {text}");
        }
    }
}

/// A home whose cached release check is stale and names a newer release.
fn home_with_a_stale_newer_release(tag: &str) -> std::path::PathBuf {
    let home = std::env::temp_dir().join(format!(
        "playbook-update-check-{tag}-{}",
        std::process::id()
    ));
    let root = home.join(".config/playbook");
    std::fs::create_dir_all(&root).unwrap();
    playbook::state::set(
        &root,
        playbook::update::check::CACHE_KEY,
        r#"{"checked_at":0,"latest":"999.0.0"}"#,
    )
    .unwrap();
    home
}

fn cache(home: &std::path::Path) -> String {
    playbook::state::get(
        &home.join(".config/playbook"),
        playbook::update::check::CACHE_KEY,
    )
    .unwrap()
    .unwrap()
}

#[test]
fn json_output_and_hooks_stay_untouched_by_the_release_check() {
    // Arrange
    let home = home_with_a_stale_newer_release("json");
    let before = cache(&home);

    // Act
    let json = playbook()
        .args(["mode", "status", "--json"])
        .env("HOME", &home)
        .env_remove("CI")
        .output()
        .unwrap();
    let hook = playbook()
        .args(["hook", "session-init"])
        .env("HOME", &home)
        .env_remove("CI")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();

    // Assert
    let stdout = String::from_utf8_lossy(&json.stdout);
    assert!(
        serde_json::from_str::<serde_json::Value>(&stdout).is_ok(),
        "{stdout}"
    );
    for out in [&json, &hook] {
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!stderr.contains("is available"), "{stderr}");
    }
    assert_eq!(
        cache(&home),
        before,
        "a skipped run must not claim a refresh"
    );
}
