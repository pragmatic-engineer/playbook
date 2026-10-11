// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Binary-level tests for `playbook sessions`, run against a scratch `$HOME`.
//! The fixture history is older than the default window, so it lists nothing.

use std::fs;
use std::process::Command;

fn run(args: &[&str]) -> (bool, String) {
    let home =
        std::env::temp_dir().join(format!("playbook-sessions-{}", playbook::testing::run_id()));
    let project = home.join(".claude/projects/proj-one");
    fs::create_dir_all(&project).unwrap();
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/usage");
    fs::copy(
        fixtures.join("usage/proj-one/s1.jsonl"),
        project.join("s1.jsonl"),
    )
    .unwrap();
    fs::copy(
        fixtures.join("account/claude.json"),
        home.join(".claude.json"),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(args)
        .env("HOME", home.canonicalize().unwrap())
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&home);
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn history_older_than_the_window_lists_no_sessions() {
    let (ok, text) = run(&["sessions"]);
    assert!(ok);
    assert_eq!(text, "No recent sessions.\n");
}

#[test]
fn json_has_a_generation_time_and_an_empty_session_list() {
    let (ok, text) = run(&["sessions", "--json"]);
    assert!(ok);
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(doc["generated_at"].as_i64().unwrap() > 0);
    assert_eq!(doc["sessions"].as_array().unwrap().len(), 0);
}

#[test]
fn the_list_subcommand_is_the_same_as_the_bare_command() {
    assert_eq!(run(&["sessions", "list"]), run(&["sessions"]));
}
