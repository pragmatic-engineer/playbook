// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook effort` end to end: the ceiling reaches `maxEffortLevel` in
//! `settings.json`, `playbook init` keeps it applied, and `auto` clears only
//! a cap playbook wrote.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "playbook-effort-cli-{}-{tag}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Home(dir)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(args)
            .current_dir(&self.0)
            .env("HOME", &self.0)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CLAUDE_PLUGIN_ROOT")
            .output()
            .expect("playbook should spawn")
    }

    fn cap(&self) -> Option<String> {
        let text = fs::read_to_string(self.0.join(".claude/settings.json")).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        v.get("maxEffortLevel")?.as_str().map(str::to_string)
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn a_level_caps_effort_and_auto_removes_it() {
    let h = Home::new("roundtrip");

    let out = h.run(&["effort", "medium"]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(h.cap().as_deref(), Some("medium"));

    let out = h.run(&["effort", "auto"]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(h.cap(), None);
}

#[test]
fn an_unknown_level_fails_and_writes_nothing() {
    let h = Home::new("unknown");
    let out = h.run(&["effort", "turbo"]);
    assert!(!out.status.success());
    assert!(
        text(&out).contains("unknown effort level"),
        "{}",
        text(&out)
    );
    assert_eq!(h.cap(), None);
}

#[test]
fn init_reapplies_the_configured_cap() {
    let h = Home::new("init");
    assert!(h.run(&["effort", "low"]).status.success());
    fs::write(h.0.join(".claude/settings.json"), "{}\n").unwrap();

    let out = h.run(&["init"]);

    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(h.cap().as_deref(), Some("low"));
}

#[test]
fn status_shows_playbook_claude_code_and_the_effective_level() {
    let h = Home::new("status");
    assert!(h.run(&["effort", "high"]).status.success());
    let body = text(&h.run(&["effort"]));
    assert!(body.contains("playbook effort.max: high"), "{body}");
    assert!(body.contains("Claude Code maxEffortLevel: none"), "{body}");
    assert!(body.contains("effective ceiling: high"), "{body}");
}

#[test]
fn a_lower_claude_code_cap_wins_and_is_left_alone() {
    let h = Home::new("lower-claude");
    fs::create_dir_all(h.0.join(".claude")).unwrap();
    fs::write(
        h.0.join(".claude/settings.json"),
        r#"{"maxEffortLevel":"medium"}"#,
    )
    .unwrap();

    let out = h.run(&["effort", "xhigh"]);

    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(h.cap().as_deref(), Some("medium"));
    let body = text(&h.run(&["effort"]));
    assert!(body.contains("playbook effort.max: xhigh"), "{body}");
    assert!(body.contains("effective ceiling: medium"), "{body}");
}

#[test]
fn a_higher_claude_code_cap_is_replaced_then_restored_on_auto() {
    let h = Home::new("higher-claude");
    fs::create_dir_all(h.0.join(".claude")).unwrap();
    fs::write(
        h.0.join(".claude/settings.json"),
        r#"{"maxEffortLevel":"max"}"#,
    )
    .unwrap();

    assert!(h.run(&["effort", "xhigh"]).status.success());
    assert_eq!(h.cap().as_deref(), Some("xhigh"));
    let body = text(&h.run(&["effort"]));
    assert!(body.contains("Claude Code maxEffortLevel: max"), "{body}");
    assert!(body.contains("effective ceiling: xhigh"), "{body}");

    assert!(h.run(&["effort", "auto"]).status.success());
    assert_eq!(h.cap().as_deref(), Some("max"));
}

#[test]
fn config_set_accepts_the_key_and_rejects_a_bad_value() {
    let h = Home::new("config");
    assert!(h
        .run(&["config", "set", "--global", "effort.max", "xhigh"])
        .status
        .success());
    assert!(!h
        .run(&["config", "set", "--global", "effort.max", "huge"])
        .status
        .success());
}
