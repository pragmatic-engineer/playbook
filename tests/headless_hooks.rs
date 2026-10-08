// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The headless switch (`PLAYBOOK_HEADLESS`, with `CI=true` as an alias) as
//! seen from a real hook process: what session-init injects, that the Stop
//! hook stays quiet, and that the safety guards do not change.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const ASYNC_NOTE: &str = "Async and deferred-tool discipline";
const MEMORY_FACT: &str = "zebra-fact";

/// A scratch HOME with one global memory fact, and a scratch git repo with an
/// `origin` remote, so session-init finds a repo and a memory slice.
struct World {
    home: PathBuf,
    repo: PathBuf,
}

fn world(tag: &str) -> World {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("headless-{}-{tag}-{n}", std::process::id()));
    let home = root.join("home");
    let repo = root.join("repo");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed");
    };
    git(&["init", "-q"]);
    git(&[
        "remote",
        "add",
        "origin",
        "https://github.com/acme/widgets.git",
    ]);
    let mem = home.join(".config").join("playbook").join("memory");
    fs::create_dir_all(&mem).unwrap();
    let graph = format!(
        r#"{{"nodes":[{{"id":"g1","name":"{MEMORY_FACT}","description":"stripes","scope":"global","type":"user"}}],"edges":[]}}"#
    );
    fs::write(mem.join("memory.graph.json"), graph).unwrap();
    World { home, repo }
}

/// Runs `playbook hook <name>` with CI and the headless variables cleared
/// first, then `env` applied, so the outer environment never leaks in.
fn run_hook(w: &World, hook: &str, stdin: &str, env: &[(&str, &str)]) -> (String, i32) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_playbook"));
    command
        .args(["hook", hook])
        .current_dir(&w.repo)
        .env("HOME", &w.home)
        .env_remove("CI")
        .env_remove("PLAYBOOK_HEADLESS")
        .env_remove("PLAYBOOK_HEADLESS_MEMORY")
        .env_remove("HOOK_INPUT")
        .env_remove("CLAUDE_PLUGIN_ROOT")
        .env_remove("AUTO_LEARN_NUDGE")
        .env_remove("SKILLS_PRIMER")
        .env_remove("ASYNC_DISCIPLINE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
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
    )
}

fn start_payload() -> String {
    r#"{"hook_event_name":"SessionStart","source":"startup","session_id":"hl-session"}"#.to_string()
}

#[test]
fn interactive_session_start_still_injects_memory_and_the_async_note() {
    let w = world("interactive");

    let (out, code) = run_hook(&w, "session-init", &start_payload(), &[]);

    assert_eq!(code, 0);
    assert!(out.contains(MEMORY_FACT), "memory slice missing: {out}");
    assert!(out.contains(ASYNC_NOTE), "async note missing: {out}");
}

#[test]
fn headless_session_start_injects_neither_memory_nor_nudges() {
    let w = world("headless");

    let (out, code) = run_hook(
        &w,
        "session-init",
        &start_payload(),
        &[("PLAYBOOK_HEADLESS", "1")],
    );

    assert_eq!(code, 0);
    assert!(!out.contains(MEMORY_FACT), "memory leaked: {out}");
    assert!(!out.contains(ASYNC_NOTE), "nudge leaked: {out}");
}

#[test]
fn ci_true_alone_counts_as_headless_for_session_start() {
    let w = world("ci-alias");

    let (out, _) = run_hook(&w, "session-init", &start_payload(), &[("CI", "true")]);

    assert!(
        !out.contains(ASYNC_NOTE) && !out.contains(MEMORY_FACT),
        "{out}"
    );
}

#[test]
fn explicit_opt_out_restores_interactive_behavior_under_ci() {
    let w = world("optout");

    let (out, _) = run_hook(
        &w,
        "session-init",
        &start_payload(),
        &[("CI", "true"), ("PLAYBOOK_HEADLESS", "0")],
    );

    assert!(
        out.contains(MEMORY_FACT) && out.contains(ASYNC_NOTE),
        "{out}"
    );
}

#[test]
fn headless_memory_opt_in_injects_memory_but_still_no_nudges() {
    let w = world("memory-optin");

    let (out, _) = run_hook(
        &w,
        "session-init",
        &start_payload(),
        &[
            ("PLAYBOOK_HEADLESS", "1"),
            ("PLAYBOOK_HEADLESS_MEMORY", "1"),
        ],
    );

    assert!(out.contains(MEMORY_FACT), "opted-in memory missing: {out}");
    assert!(!out.contains(ASYNC_NOTE), "nudge leaked: {out}");
}

#[test]
fn headless_session_start_leaves_a_saved_handoff_unread() {
    let w = world("handoff");
    let mut save = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["handoff", "save", "--dir"])
        .arg(&w.repo)
        .env("HOME", &w.home)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    save.stdin
        .take()
        .unwrap()
        .write_all(b"# Session Handoff - probe\n\n1. HANDOFF-PROBE-MARKER\n")
        .unwrap();
    assert!(save.wait().unwrap().success());
    let dir = w.home.join(".config/playbook/runtime/handoff");
    let unread = |d: &Path| {
        fs::read_dir(d)
            .unwrap()
            .flatten()
            .filter(|e| e.path().is_file())
            .count()
    };
    assert_eq!(unread(&dir), 1);

    let (out, _) = run_hook(
        &w,
        "session-init",
        &start_payload(),
        &[("PLAYBOOK_HEADLESS", "1")],
    );

    assert!(!out.contains("HANDOFF-PROBE-MARKER"), "{out}");
    assert_eq!(unread(&dir), 1, "headless must not consume the handoff");
}

#[test]
fn auto_model_detect_is_silent_headless_and_speaks_otherwise() {
    let w = world("amd");
    let prompt = r#"{"prompt":"Should we design a new schema and evaluate the tradeoffs between the two approaches?"}"#;

    let (interactive, _) = run_hook(&w, "auto-model-detect", prompt, &[]);
    let (headless, _) = run_hook(
        &w,
        "auto-model-detect",
        prompt,
        &[("PLAYBOOK_HEADLESS", "1")],
    );

    assert!(
        !interactive.trim().is_empty(),
        "the nudge must fire interactively"
    );
    assert_eq!(headless.trim(), "");
}

#[test]
fn safety_guards_behave_identically_headless_and_not() {
    let w = world("guards");
    let cases = [
        ("rm-workspace-guard", "rm -rf /etc/passwd"),
        ("rm-workspace-guard", "ls"),
        ("bg-await-guard", "sleep 30 &"),
        ("no-slop-guard", "echo hello"),
    ];
    for (guard, command) in cases {
        let payload = serde_json::json!({ "tool_input": { "command": command } }).to_string();
        let plain = run_hook(&w, guard, &payload, &[]);
        let headless = run_hook(&w, guard, &payload, &[("PLAYBOOK_HEADLESS", "1")]);
        let ci = run_hook(&w, guard, &payload, &[("CI", "true")]);
        assert_eq!(
            plain, headless,
            "{guard} {command:?} changed under PLAYBOOK_HEADLESS"
        );
        assert_eq!(plain, ci, "{guard} {command:?} changed under CI=true");
    }
    let deny = serde_json::json!({ "tool_input": { "command": "rm -rf /etc/passwd" } }).to_string();
    let (out, _) = run_hook(
        &w,
        "rm-workspace-guard",
        &deny,
        &[("PLAYBOOK_HEADLESS", "1")],
    );
    assert!(
        out.contains(r#""permissionDecision":"deny""#),
        "the guard must still deny: {out}"
    );
}

const STOP_SID: &str = "hl-stop";

fn stop_payload(extra_fields: &str) -> String {
    format!(r#"{{"hook_event_name":"Stop","session_id":"{STOP_SID}"{extra_fields}}}"#)
}

/// Writes the `capture-due` marker `playbook statusline` leaves, so the hook has a
/// reason to block. Returns the session directory.
fn arm_capture_due(w: &World) -> PathBuf {
    let dir = w
        .home
        .join(".config")
        .join("playbook")
        .join("runtime")
        .join(STOP_SID);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("capture-due"), "").unwrap();
    dir
}

#[test]
fn headless_stop_never_blocks_even_with_a_capture_marker() {
    let w = world("stop-headless");
    let dir = arm_capture_due(&w);

    let (out, code) = run_hook(
        &w,
        "memory-capture",
        &stop_payload(""),
        &[("PLAYBOOK_HEADLESS", "1")],
    );

    assert_eq!((out.trim(), code), ("", 0));
    assert!(
        !dir.join("capture-attempts").exists(),
        "a headless Stop must not even count an attempt"
    );
}

#[test]
fn ci_true_alone_stops_the_stop_hook_from_blocking() {
    let w = world("stop-ci");
    arm_capture_due(&w);

    let (out, code) = run_hook(&w, "memory-capture", &stop_payload(""), &[("CI", "true")]);

    assert_eq!((out.trim(), code), ("", 0));
}

#[test]
fn interactive_stop_still_blocks_with_the_reblock_count() {
    let w = world("stop-interactive");
    arm_capture_due(&w);

    let (out, code) = run_hook(&w, "memory-capture", &stop_payload(""), &[]);

    assert_eq!(code, 0);
    assert!(out.contains(r#""decision":"block""#), "output: {out}");
    assert!(out.contains("re-block 1 of 2"), "output: {out}");
}

#[test]
fn interactive_stop_ignores_stop_hook_active_so_the_bounded_reblock_still_escalates() {
    // ADR 0009 bounds the re-block by its own cap of 2. Honoring
    // `stop_hook_active` would end the escalation after the first block.
    let w = world("stop-active");
    arm_capture_due(&w);
    let active = stop_payload(r#","stop_hook_active":true"#);

    let (first, _) = run_hook(&w, "memory-capture", &active, &[]);
    let (second, _) = run_hook(&w, "memory-capture", &active, &[]);
    let (third, _) = run_hook(&w, "memory-capture", &active, &[]);

    assert!(first.contains("re-block 1 of 2"), "first: {first}");
    assert!(second.contains("re-block 2 of 2"), "second: {second}");
    assert_eq!(
        third.trim(),
        "",
        "the cap releases after two blocks: {third}"
    );
}
