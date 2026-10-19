// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook hook disk-guard` and the SessionStart disk line, run as the
//! built binary in a scratch home.
//! Free space comes from `PLAYBOOK_TEST_DISK_FREE_GB`, which debug builds
//! read in place of the real value. Only the test that checks the
//! background cleanup sets `PLAYBOOK_TEST_DISK_TMP`, which points that
//! cleanup at a scratch folder. Without it the hook starts no cleanup.

#[path = "support/isolated.rs"]
mod isolated;

use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};

static SESSION: AtomicUsize = AtomicUsize::new(0);

/// A session id no other test uses.
fn session() -> String {
    format!(
        "disk-guard-{}-{}",
        playbook::testing::run_id(),
        SESSION.fetch_add(1, Ordering::Relaxed)
    )
}

fn hook(home: &Path, name: &str, free_gb: &str, payload: &str, envs: &[(&str, &str)]) -> String {
    let mut cmd = isolated::playbook_in(home);
    cmd.args(["hook", name])
        .env("PLAYBOOK_TEST_DISK_FREE_GB", free_gb)
        .env("PLAYBOOK_HEADLESS", "0")
        .current_dir(home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("hook spawns");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "hook {name} exited {:?}", out.status);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn bash(home: &Path, sid: &str, command: &str) -> String {
    serde_json::json!({
        "session_id": sid,
        "hook_event_name": "PreToolUse",
        "cwd": home,
        "tool_name": "Bash",
        "tool_input": {"command": command},
    })
    .to_string()
}

fn agent(home: &Path, sid: &str, tool: &str) -> String {
    serde_json::json!({
        "session_id": sid,
        "hook_event_name": "PreToolUse",
        "cwd": home,
        "tool_name": tool,
        "tool_input": {"prompt": "build it"},
    })
    .to_string()
}

fn denied(out: &str) -> bool {
    out.contains("\"permissionDecision\":\"deny\"")
}

#[test]
fn a_build_is_denied_below_the_floor_and_warned_below_the_warn_line() {
    let cases = [
        ("0", "deny"),
        ("1", "deny"),
        ("5.9", "deny"),
        ("6.0", "warn"),
        ("14.9", "warn"),
        ("15", "allow"),
    ];
    for (free, want) in cases {
        let home = isolated::fresh_home();
        let out = hook(
            &home,
            "disk-guard",
            free,
            &bash(&home, &session(), "cargo build"),
            &[],
        );
        let got = if denied(&out) {
            "deny"
        } else if out.contains("additionalContext") {
            "warn"
        } else if out.trim().is_empty() {
            "allow"
        } else {
            "other"
        };
        assert_eq!(got, want, "{free} GB free: {out}");
        if want == "deny" {
            assert!(out.contains("cargo build is blocked"), "{out}");
            assert!(out.contains("disk.blockFreeGb (6 GB)"), "{out}");
            assert!(out.contains("playbook disk clean"), "{out}");
            assert!(
                out.contains("playbook config set disk.blockFreeGb"),
                "{out}"
            );
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}

#[test]
fn recovery_commands_stay_allowed_at_zero() {
    let home = isolated::fresh_home();
    for command in [
        "cargo clean",
        "rm -rf /tmp/pb-old",
        "playbook disk clean",
        "df -g /",
        "du -sh .worktrees",
        "git status",
        "git worktree remove ../wt",
    ] {
        let out = hook(
            &home,
            "disk-guard",
            "0",
            &bash(&home, &session(), command),
            &[],
        );
        assert!(!denied(&out), "{command}: {out}");
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_new_agent_is_denied_only_below_the_floor() {
    let home = isolated::fresh_home();
    for tool in ["Agent", "Task"] {
        let out = hook(
            &home,
            "disk-guard",
            "5.9",
            &agent(&home, &session(), tool),
            &[],
        );
        assert!(denied(&out), "{tool}: {out}");
        assert!(out.contains("starting a new agent is blocked"), "{out}");
        let out = hook(
            &home,
            "disk-guard",
            "6.0",
            &agent(&home, &session(), tool),
            &[],
        );
        assert!(!denied(&out), "{tool}: {out}");
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_warning_comes_once_per_quiet_window_in_a_session() {
    let home = isolated::fresh_home();
    let sid = session();
    let first = hook(&home, "disk-guard", "10", &bash(&home, &sid, "ls"), &[]);
    assert!(first.contains("10.0 GB free"), "{first}");
    let second = hook(&home, "disk-guard", "10", &bash(&home, &sid, "ls"), &[]);
    assert!(second.trim().is_empty(), "{second}");
    let other = hook(
        &home,
        "disk-guard",
        "10",
        &bash(&home, &session(), "ls"),
        &[],
    );
    assert!(other.contains("10.0 GB free"), "{other}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn below_one_gb_the_ballast_is_deleted_and_the_message_says_so() {
    let home = isolated::fresh_home();
    let ballast = home.join(".cache/playbook/ballast/ballast.bin");
    std::fs::create_dir_all(ballast.parent().unwrap()).unwrap();
    std::fs::write(&ballast, vec![7u8; 4096]).unwrap();
    let out = hook(
        &home,
        "disk-guard",
        "0.5",
        &bash(&home, &session(), "cargo test"),
        &[],
    );
    assert!(!ballast.exists(), "the ballast is still there");
    assert!(out.contains("deleted the ballast file"), "{out}");
    assert!(denied(&out), "{out}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_off_switch_silences_the_guard() {
    let home = isolated::fresh_home();
    let out = hook(
        &home,
        "disk-guard",
        "0",
        &bash(&home, &session(), "cargo build"),
        &[("DISK_GUARD", "0")],
    );
    assert!(out.trim().is_empty(), "{out}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_first_denial_starts_one_background_cleanup_per_window() {
    let home = isolated::fresh_home();
    let tmp = home.join("scratch-tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let tmp = tmp.to_string_lossy().into_owned();
    let envs = [("PLAYBOOK_TEST_DISK_TMP", tmp.as_str())];
    let sid = session();
    let first = hook(
        &home,
        "disk-guard",
        "1",
        &bash(&home, &sid, "cargo build"),
        &envs,
    );
    assert!(denied(&first), "{first}");
    assert!(first.contains("has started"), "{first}");
    let second = hook(
        &home,
        "disk-guard",
        "1",
        &bash(&home, &sid, "cargo build"),
        &envs,
    );
    assert!(denied(&second), "{second}");
    assert!(!second.contains("has started"), "{second}");
}

#[test]
fn session_start_prints_the_warning_when_space_is_low() {
    let home = isolated::fresh_home();
    let payload = |sid: &str| {
        serde_json::json!({
            "session_id": sid,
            "hook_event_name": "SessionStart",
            "source": "startup",
            "cwd": home,
            "transcript_path": home.join("none.jsonl"),
        })
        .to_string()
    };
    let low = hook(&home, "session-init", "5", &payload(&session()), &[]);
    assert!(low.contains("Disk guard: 5.0 GB free"), "{low}");
    let fine = hook(&home, "session-init", "50", &payload(&session()), &[]);
    assert!(!fine.contains("Disk guard"), "{fine}");
    let _ = std::fs::remove_dir_all(&home);
}
