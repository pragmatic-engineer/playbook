// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook doctor models`: the tier table, overrides and the fallback chain.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pb-doctor-models-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join(".claude")).unwrap();
    dir
}

fn run(home: &PathBuf, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["doctor", "models"])
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn it_lists_every_tier_and_the_chain_ccc_passes() {
    let h = home("plain");
    let t = run(&h, &[]);
    assert!(
        t.contains("sonnet (balanced): claude-sonnet-5-5, falls back to claude-sonnet-5"),
        "{t}"
    );
    assert!(
        t.contains("fallback chain ccc passes: claude-opus-5,claude-sonnet-5,claude-haiku-4-5"),
        "{t}"
    );
}

#[test]
fn a_user_fallback_is_reported_as_replacing_ours() {
    let h = home("user");
    fs::write(
        h.join(".claude/settings.json"),
        r#"{"fallbackModel":"sonnet"}"#,
    )
    .unwrap();
    let j: serde_json::Value = serde_json::from_str(&run(&h, &["--json"])).unwrap();
    assert_eq!(j["chainSource"], "user");
}

#[test]
fn xhigh_in_force_is_called_out_and_trims_the_chain() {
    let h = home("xhigh");
    fs::write(
        h.join(".claude/settings.json"),
        r#"{"effortLevel":"xhigh"}"#,
    )
    .unwrap();
    let t = run(&h, &[]);
    assert!(
        t.contains("claude-haiku-4-5") && !t.contains("claude-opus-5,"),
        "{t}"
    );
    assert!(t.contains("reject it, so the chain starts at Haiku"), "{t}");
    let j: serde_json::Value = serde_json::from_str(&run(&h, &["--json"])).unwrap();
    assert_eq!(j["chain"], "claude-haiku-4-5");
    assert_eq!(j["effort"], "xhigh");
}

#[test]
fn an_override_shows_in_the_table() {
    let h = home("override");
    let set = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["config", "set", "--global", "models.opus", "claude-opus-5"])
        .current_dir(&h)
        .env("HOME", &h)
        .output()
        .unwrap();
    assert!(set.status.success());
    let t = run(&h, &[]);
    assert!(t.contains("override claude-opus-5"), "{t}");
}
