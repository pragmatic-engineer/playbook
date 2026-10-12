// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The hint shown when the plugin was seen installed and is now gone, and
//! the cases that must stay silent.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const HINT: &str = "playbook plugin was removed, run `playbook uninstall --yes` to clean up";
const LISTED: &str =
    r#"{"version":2,"plugins":{"playbook@pragmatic-engineer":[{"scope":"user"}]}}"#;
const EMPTY: &str = r#"{"version":2,"plugins":{}}"#;

fn scratch_home(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!(
        "playbook-removed-hint-{tag}-{}",
        playbook::testing::run_id()
    ));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(home.join(".claude/plugins")).unwrap();
    home
}

fn set_installed(home: &Path, body: &str) {
    fs::write(home.join(".claude/plugins/installed_plugins.json"), body).unwrap();
}

fn mark_seen(home: &Path) {
    fs::create_dir_all(home.join(".config/playbook")).unwrap();
    fs::write(home.join(".config/playbook/plugin-seen"), "1\n").unwrap();
}

fn statusline(home: &Path) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .arg("statusline")
        .env("HOME", home)
        .env_remove("PLAYBOOK_IGNORE_PLUGIN_DISABLED")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"{}").unwrap();
    String::from_utf8_lossy(&child.wait_with_output().unwrap().stdout).into_owned()
}

fn session_start(home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["hook", "session-init"])
        .env("HOME", home)
        .env(
            "HOOK_INPUT",
            r#"{"source":"startup","session_id":"removed-hint"}"#,
        )
        .env_remove("CI")
        .env_remove("PLAYBOOK_HEADLESS")
        .current_dir(home)
        .output()
        .unwrap()
}

fn context(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_seen_then_removed_plugin_shows_the_hint_in_the_status_line() {
    let home = scratch_home("sl-removed");
    set_installed(&home, EMPTY);
    mark_seen(&home);
    assert!(statusline(&home).contains(HINT));
}

#[test]
fn a_user_who_never_had_the_plugin_sees_no_hint_in_the_status_line() {
    let home = scratch_home("sl-never");
    set_installed(&home, EMPTY);
    assert!(!statusline(&home).contains("plugin was removed"));
}

#[test]
fn an_installed_plugin_shows_no_hint_in_the_status_line() {
    let home = scratch_home("sl-installed");
    set_installed(&home, LISTED);
    mark_seen(&home);
    assert!(!statusline(&home).contains("plugin was removed"));
}

#[test]
fn the_config_key_hides_the_hint_in_the_status_line() {
    let home = scratch_home("sl-off");
    set_installed(&home, EMPTY);
    mark_seen(&home);
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["config", "set", "--global", "hints.pluginRemoved", "false"])
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(!statusline(&home).contains("plugin was removed"));
}

#[test]
fn session_start_adds_the_hint_once_when_the_plugin_was_removed() {
    let home = scratch_home("ss-removed");
    set_installed(&home, EMPTY);
    mark_seen(&home);
    let shown = context(&session_start(&home));
    assert_eq!(shown.matches(HINT).count(), 1, "{shown}");
}

#[test]
fn session_start_stays_quiet_for_a_user_who_never_had_the_plugin() {
    let home = scratch_home("ss-never");
    set_installed(&home, EMPTY);
    assert!(!context(&session_start(&home)).contains("plugin was removed"));
}

#[test]
fn init_records_an_installed_plugin_and_never_edits_claude_plugin_files() {
    let home = scratch_home("init-records");
    set_installed(&home, LISTED);
    let before = fs::read_to_string(home.join(".claude/plugins/installed_plugins.json")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["init", "--yes", "--no-path"])
        .env("HOME", &home)
        .env("SHELL", "/bin/zsh")
        .env("CLAUDE_PLUGIN_ROOT", env!("CARGO_MANIFEST_DIR"))
        .current_dir(&home)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(home.join(".config/playbook/plugin-seen").is_file());
    assert_eq!(
        fs::read_to_string(home.join(".claude/plugins/installed_plugins.json")).unwrap(),
        before
    );
}
