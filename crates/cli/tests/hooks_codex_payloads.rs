// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The Rust Bash guards against payloads shaped like Codex hook input.
//!
//! Each file in `fixtures/codex/` is one payload plus a `_fixture` header that
//! names its origin, the guard to run and the expected outcome. A loop runs
//! every file, so a new fixture is covered without editing this test. Nothing
//! here registers a hook in Codex; it only pins what the guards print.

#[path = "support/isolated.rs"]
mod isolated;

use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("playbook-tests")
        .join(format!("codex-{tag}-{}-{n}", playbook::testing::run_id()));
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .status()
        .expect("git runs")
        .success();
    assert!(ok, "git {args:?} failed");
}

/// A repo with one staged file holding a debug line, for `precommit-check`.
fn staged_debug_repo() -> PathBuf {
    let dir = scratch("repo");
    git(&dir, &["init", "-q"]);
    fs::write(dir.join("a.js"), "console.log('x');\n").expect("write");
    git(&dir, &["add", "a.js"]);
    dir
}

/// Runs `playbook hook <guard>` with `stdin`. Returns (exit ok, stdout).
fn run_guard(guard: &str, stdin: &[u8], dir: Option<&Path>) -> (bool, String) {
    let mut cmd = isolated::playbook();
    cmd.env_remove("CI")
        .env_remove("PLAYBOOK_HEADLESS")
        .env_remove("HOOK_INPUT")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(["hook", guard])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let mut child = cmd.spawn().expect("playbook spawns");
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(stdin)
        .expect("stdin writes");
    let out = child.wait_with_output().expect("playbook exits");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

fn fixture_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codex");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("fixture dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files
}

fn hook_output(stdout: &str) -> Value {
    serde_json::from_str(stdout).unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {stdout}"))
}

#[test]
fn every_fixture_gets_the_outcome_it_declares() {
    let files = fixture_files();
    let mut seen = std::collections::BTreeSet::new();
    for f in &files {
        let v: Value = serde_json::from_str(&fs::read_to_string(f).expect("reads")).expect("JSON");
        seen.insert(
            v["_fixture"]["expect"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        );
    }
    for outcome in ["silent", "deny", "context", "rewrite"] {
        assert!(
            seen.contains(outcome),
            "no fixture expects {outcome}: {files:?}"
        );
    }
    for file in files {
        // Arrange
        let name = file.display().to_string();
        let raw = fs::read_to_string(&file).expect("fixture reads");
        let fixture: Value = serde_json::from_str(&raw).expect("fixture is JSON");
        let head = &fixture["_fixture"];
        let origin = head["origin"].as_str().unwrap_or_default();
        assert!(
            ["recorded", "schema-derived"].contains(&origin),
            "{name}: origin must be recorded or schema-derived"
        );
        if origin == "recorded" {
            assert!(
                head["codex_version"].is_string(),
                "{name}: a recorded fixture names the Codex version"
            );
        }
        let guard = head["guard"].as_str().expect("guard named");
        // Every fixture runs in a scratch dir, never the real checkout, and
        // its payload `cwd` points there too so no host path is read.
        let dir = match head["setup"].as_str() {
            Some("staged-debug-repo") => staged_debug_repo(),
            None => scratch("empty"),
            Some(other) => panic!("{name}: unknown setup {other}"),
        };
        let mut payload: Value = fixture.clone();
        payload["cwd"] = Value::String(dir.display().to_string());
        let raw = payload.to_string();

        // Act
        let (ok, stdout) = run_guard(guard, raw.as_bytes(), Some(&dir));

        // Assert
        assert!(ok, "{name}: a guard exits 0 even when it denies");
        match head["expect"].as_str().expect("expect named") {
            "silent" => assert!(
                stdout.trim().is_empty(),
                "{name}: expected no output: {stdout}"
            ),
            "deny" => {
                let out = hook_output(&stdout);
                assert_eq!(
                    out["hookSpecificOutput"]["permissionDecision"], "deny",
                    "{name}: {stdout}"
                );
                assert!(
                    out["hookSpecificOutput"]["permissionDecisionReason"].is_string(),
                    "{name}: a deny carries a reason"
                );
            }
            "context" => {
                let out = hook_output(&stdout);
                assert!(
                    out["hookSpecificOutput"]["additionalContext"]
                        .as_str()
                        .is_some_and(|c| !c.is_empty()),
                    "{name}: {stdout}"
                );
                assert!(
                    out["hookSpecificOutput"]["permissionDecision"].is_null(),
                    "{name}: a warning must not decide: {stdout}"
                );
            }
            "rewrite" => {
                // Pins today's shape: updatedInput and context, and NO
                // permissionDecision. The Codex docs describe a rewrite as
                // permissionDecision "allow" plus updatedInput, so whether
                // Codex applies this form is unverified.
                let out = hook_output(&stdout);
                let rewritten = out["hookSpecificOutput"]["updatedInput"]["command"]
                    .as_str()
                    .unwrap_or_default();
                assert!(rewritten.starts_with("git commit"), "{name}: {stdout}");
                assert!(!rewritten.contains("Co-Authored-By"), "{name}: {stdout}");
                assert!(
                    out["hookSpecificOutput"]["permissionDecision"].is_null(),
                    "{name}: update this test and the support page together: {stdout}"
                );
            }
            other => panic!("{name}: unknown expect {other}"),
        }
    }
}

#[test]
fn empty_stdin_is_allowed_by_every_bash_guard() {
    for guard in [
        "rm-workspace-guard",
        "precommit-check",
        "commit-message-sanitizer",
    ] {
        let (ok, stdout) = run_guard(guard, b"", None);
        assert!(ok, "{guard}: exits 0");
        assert!(stdout.trim().is_empty(), "{guard}: {stdout}");
    }
}

#[test]
fn non_json_stdin_is_allowed_by_every_bash_guard() {
    for guard in [
        "rm-workspace-guard",
        "precommit-check",
        "commit-message-sanitizer",
    ] {
        let (ok, stdout) = run_guard(guard, b"this is not json", None);
        assert!(ok, "{guard}: exits 0");
        assert!(stdout.trim().is_empty(), "{guard}: {stdout}");
    }
}

#[test]
fn apply_patch_without_shell_text_is_allowed_by_every_bash_guard() {
    // The guards read only `tool_input.command`, never `tool_name`, so this
    // passes because the patch holds no `rm` or `git commit`, not because of
    // the tool name. The next test pins the other side.
    let payload = serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "apply_patch",
        "tool_input": {"command": "*** Begin Patch\n*** Add File: a.txt\n+hi\n*** End Patch"}
    })
    .to_string();
    let dir = scratch("patch");
    for guard in [
        "rm-workspace-guard",
        "precommit-check",
        "commit-message-sanitizer",
    ] {
        let (ok, stdout) = run_guard(guard, payload.as_bytes(), Some(&dir));
        assert!(ok, "{guard}: exits 0");
        assert!(stdout.trim().is_empty(), "{guard}: {stdout}");
    }
}

#[test]
fn apply_patch_text_is_scanned_like_a_command() {
    // A patch whose context line reads `rm -rf ~` is denied, because the guard
    // splits `tool_input.command` into lines whatever the tool is.
    let payload = serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "apply_patch",
        "tool_input": {"command": "*** Begin Patch\n*** Update File: a.sh\n rm -rf ~\n*** End Patch"}
    })
    .to_string();
    let (ok, stdout) = run_guard(
        "rm-workspace-guard",
        payload.as_bytes(),
        Some(&scratch("patch2")),
    );
    assert!(ok);
    assert_eq!(
        hook_output(&stdout)["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
}
