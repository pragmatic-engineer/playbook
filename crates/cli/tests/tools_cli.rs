// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook tools map`: the text, JSON and budget check a session start reads.

#[path = "support/isolated.rs"]
mod isolated;

use std::process::Output;
fn map(args: &[&str]) -> Output {
    isolated::playbook()
        .args(["tools", "map"])
        .args(args)
        .output()
        .expect("playbook should spawn")
}

#[test]
fn the_text_map_has_a_header_and_one_line_per_group() {
    let out = map(&[]);

    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(text.starts_with("playbook tools:"), "{text}");
    assert!(
        text.contains("\npath plans|implement|worktrees --create\n"),
        "{text}"
    );
    assert!(text.contains("\ngate record|snapshot|check\n"), "{text}");
}

#[test]
fn json_is_a_list_of_intents_and_commands() {
    let out = map(&["--json"]);

    let rows: Vec<serde_json::Value> =
        serde_json::from_slice(&out.stdout).expect("the output should be JSON");
    assert!(out.status.success());
    assert!(rows
        .iter()
        .any(|r| r["intent"] == "pr-reply" && r["command"] == "playbook pr reply"));
}

#[test]
fn check_passes_when_the_map_is_within_budget() {
    let out = map(&["--check"]);

    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("budget 300"));
}

#[test]
fn json_and_check_cannot_be_combined() {
    let out = map(&["--json", "--check"]);

    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot be used with"));
}
