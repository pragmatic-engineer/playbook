// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook hook policy-guard` as a real process: what it denies, what it
//! lets through, that it fails safe on bad input, and that it is fast.

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const HOME: &str = "/Users/tester";

fn run(stdin: &str, env: &[(&str, &str)]) -> (String, i32, Duration) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_playbook"));
    command
        .args(["hook", "policy-guard"])
        .env("HOME", HOME)
        .env_remove("POLICY_GUARD")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
    let started = Instant::now();
    let mut child = command.spawn().expect("playbook spawns");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
        started.elapsed(),
    )
}

fn bash(command: &str) -> String {
    json!({"tool_name": "Bash", "tool_input": {"command": command}}).to_string()
}

fn write_to(path: &str) -> String {
    json!({"tool_name": "Write", "tool_input": {"file_path": path, "content": "x"}}).to_string()
}

fn denial(stdout: &str) -> Option<String> {
    let v: Value = serde_json::from_str(stdout).ok()?;
    let inner = &v["hookSpecificOutput"];
    (inner["permissionDecision"] == "deny").then(|| {
        inner["permissionDecisionReason"]
            .as_str()
            .unwrap_or("")
            .to_string()
    })
}

#[test]
fn no_verify_and_signing_off_are_denied_with_a_reason() {
    for command in [
        "git commit --no-verify -m x",
        "git commit -n -m x",
        "git push --no-verify",
        "git commit --no-gpg-sign -m x",
        "git -c commit.gpgsign=false commit -m x",
    ] {
        let (out, code, _) = run(&bash(command), &[]);
        assert_eq!(code, 0, "{command}");
        assert!(denial(&out).is_some(), "{command} should be denied: {out}");
    }
}

#[test]
fn a_hand_run_gh_pr_create_is_routed_to_the_skill() {
    let (out, _, _) = run(&bash("gh pr create --title t --body b"), &[]);

    let reason = denial(&out).expect("denied");
    assert!(reason.contains("/playbook:create-pull-request"), "{reason}");
}

#[test]
fn the_skills_own_pr_command_and_ordinary_git_pass() {
    for command in [
        "playbook pr create --title t --body-file f",
        "git commit -S -s -m 'feat: x'",
        "git push origin feature",
        "gh pr view 1",
        "gh pr merge 1 --auto",
        "ls",
    ] {
        let (out, code, _) = run(&bash(command), &[]);
        assert_eq!((out.as_str(), code), ("", 0), "{command}");
    }
}

#[test]
fn writes_to_claude_memory_are_denied_and_the_playbook_store_is_not() {
    let (out, _, _) = run(
        &write_to("/Users/tester/.claude/projects/-Users-tester-repo/memory/MEMORY.md"),
        &[],
    );
    assert!(denial(&out)
        .expect("denied")
        .contains("~/.config/playbook/memory"));

    let (out, _, _) = run(&bash("echo x >> ~/.claude/memory/a.md"), &[]);
    assert!(denial(&out).is_some(), "{out}");

    let (out, code, _) = run(&write_to("/Users/tester/.config/playbook/memory/a.md"), &[]);
    assert_eq!(code, 0);
    assert!(denial(&out).is_none(), "{out}");
    let (out, _, _) = run(&bash("cat ~/.claude/projects/p/memory/a.md"), &[]);
    assert_eq!(out, "", "reads are fine");
}

#[test]
fn bad_input_is_allowed_silently() {
    for input in [
        "",
        "{",
        "[1,2]",
        r#"{"tool_input":{}}"#,
        r#"{"tool_input":{"command":7}}"#,
    ] {
        let (out, code, _) = run(input, &[]);
        assert_eq!((out.as_str(), code), ("", 0), "{input}");
    }
}

#[test]
fn the_off_switch_allows_everything() {
    let (out, code, _) = run(&bash("git commit --no-verify"), &[("POLICY_GUARD", "0")]);

    assert_eq!((out.as_str(), code), ("", 0));
}

#[test]
fn a_long_command_is_still_fast() {
    // Arrange: a 200 KB command that mentions git.
    let command = format!("git status && echo {}", "word ".repeat(40_000));

    // Act
    let (out, code, took) = run(&bash(&command), &[]);

    // Assert: generous bound, since a CI runner can be slow.
    assert_eq!((out.as_str(), code), ("", 0));
    assert!(took < Duration::from_secs(2), "{took:?}");
}

#[test]
fn writing_a_fact_file_shows_the_memory_format_and_does_not_deny() {
    // Arrange / Act
    let (out, code, _) = run(
        &write_to("/Users/tester/.config/playbook/memory/o/r/fact.md"),
        &[],
    );

    // Assert
    assert_eq!(code, 0);
    assert!(denial(&out).is_none(), "{out}");
    let v: Value = serde_json::from_str(&out).expect("json");
    let context = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        context.contains("supersedes") && context.contains("anchors:"),
        "{context}"
    );
}

#[test]
fn only_a_write_of_a_fact_file_shows_the_format() {
    // Arrange: an Edit, a graph file, and a file outside the store.
    let edit = json!({"tool_name": "Edit", "tool_input": {"file_path": "/Users/tester/.config/playbook/memory/a.md"}}).to_string();

    // Act / Assert
    assert_eq!(run(&edit, &[]).0, "");
    assert_eq!(
        run(
            &write_to("/Users/tester/.config/playbook/memory/memory.graph.json"),
            &[]
        )
        .0,
        ""
    );
    assert_eq!(run(&write_to("/Users/tester/repo/notes.md"), &[]).0, "");
}
