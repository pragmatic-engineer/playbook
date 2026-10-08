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
fn setting_a_level_stores_it_and_leaves_claude_code_settings_alone() {
    let h = Home::new("roundtrip");
    fs::create_dir_all(h.0.join(".claude")).unwrap();
    let settings = h.0.join(".claude/settings.json");
    fs::write(&settings, r#"{"maxEffortLevel":"max"}"#).unwrap();

    let out = h.run(&["effort", "medium"]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(h.cap().as_deref(), Some("max"));
    assert_eq!(
        fs::read_to_string(&settings).unwrap(),
        r#"{"maxEffortLevel":"max"}"#
    );

    let out = h.run(&["effort", "auto"]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(h.cap().as_deref(), Some("max"));
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
    assert!(!h.0.join(".claude/settings.json").exists());
}

#[test]
fn init_never_writes_a_cap() {
    let h = Home::new("init");
    assert!(h.run(&["effort", "low"]).status.success());
    let out = h.run(&["init"]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(h.cap(), None);
}

#[test]
fn xhigh_under_a_claude_code_max_is_playbooks_to_apply() {
    let h = Home::new("xhigh-max");
    fs::create_dir_all(h.0.join(".claude")).unwrap();
    fs::write(
        h.0.join(".claude/settings.json"),
        r#"{"maxEffortLevel":"max"}"#,
    )
    .unwrap();
    assert!(h.run(&["effort", "xhigh"]).status.success());
    let body = text(&h.run(&["effort"]));
    assert!(body.contains("playbook effort.max: xhigh"), "{body}");
    assert!(body.contains("Claude Code maxEffortLevel: max"), "{body}");
    assert!(
        body.contains("effective ceiling: xhigh (playbook)"),
        "{body}"
    );
}

#[test]
fn a_lower_claude_code_ceiling_wins_in_the_status() {
    let h = Home::new("lower-claude");
    fs::create_dir_all(h.0.join(".claude")).unwrap();
    fs::write(
        h.0.join(".claude/settings.json"),
        r#"{"maxEffortLevel":"medium"}"#,
    )
    .unwrap();
    assert!(h.run(&["effort", "xhigh"]).status.success());
    let out = h.run(&["effort", "--json"]);
    let v: Value = serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap();
    assert_eq!(v["effective"], "medium");
    assert_eq!(v["winner"], "claude-code");
    assert_eq!(v["playbook"], "xhigh");
    assert_eq!(v["claudeCode"], "medium");
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
