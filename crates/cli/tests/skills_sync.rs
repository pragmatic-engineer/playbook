// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook skills sync` against a fixture plugin and a scratch home. No
//! test reads or writes a real home: `isolate` points every home at scratch.

#[path = "support/isolated.rs"]
mod isolated;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const PLUGIN_ENV: &str = "PLAYBOOK_PLUGIN_DIR";
const NOTHING_TO_CHANGE: &str = "nothing to change";
const NOT_FOUND_LINE: &str = "no other agents found (looked for Codex at ~/.codex)";
const SKILL_NAME: &str = "playbook-fix";
const COMMAND_NAME: &str = "playbook-ship";
const EXIT_FAILURE: i32 = 1;
const EXIT_USAGE: i32 = 2;

/// A plugin with one skill and one command, in its own scratch folder.
fn fixture_plugin(home: &Path) -> PathBuf {
    let plugin = home.join("fixture-plugin");
    fs::create_dir_all(plugin.join("skills/fix")).unwrap();
    fs::create_dir_all(plugin.join("commands")).unwrap();
    fs::create_dir_all(plugin.join(".claude-plugin")).unwrap();
    fs::write(
        plugin.join(".claude-plugin/plugin.json"),
        r#"{"name":"playbook","version":"1.2.3"}"#,
    )
    .unwrap();
    fs::write(
        plugin.join("skills/fix/SKILL.md"),
        "---\nname: fix\ndescription: Fix a small bug\n---\nFix it.\n",
    )
    .unwrap();
    fs::write(
        plugin.join("commands/ship.md"),
        "---\ndescription: Ship the change\n---\nShip it.\n",
    )
    .unwrap();
    plugin
}

/// A home with `.codex/` and a fixture plugin.
fn codex_home() -> (PathBuf, PathBuf) {
    let home = isolated::fresh_home();
    fs::create_dir_all(home.join(".codex")).unwrap();
    let plugin = fixture_plugin(&home);
    (home, plugin)
}

fn sync(home: &Path, plugin: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_playbook"));
    isolated::isolate(&mut cmd, home);
    // An empty PATH so a real `codex` on the machine never counts as found.
    cmd.env(PLUGIN_ENV, plugin)
        .env("PATH", "")
        .args(["skills", "sync"])
        .args(args);
    cmd.output().expect("playbook should start")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn skills_dir(home: &Path) -> PathBuf {
    home.join(".agents/skills")
}

#[test]
fn sync_creates_the_playbook_folders() {
    // Arrange
    let (home, plugin) = codex_home();

    // Act
    let out = sync(&home, &plugin, &[]);

    // Assert
    assert!(out.status.success(), "{out:?}");
    assert!(skills_dir(&home)
        .join(SKILL_NAME)
        .join("SKILL.md")
        .is_file());
    assert!(skills_dir(&home)
        .join(COMMAND_NAME)
        .join("SKILL.md")
        .is_file());
}

#[test]
fn a_second_run_has_nothing_to_change() {
    // Arrange
    let (home, plugin) = codex_home();
    sync(&home, &plugin, &[]);

    // Act
    let out = sync(&home, &plugin, &[]);

    // Assert
    assert!(out.status.success(), "{out:?}");
    assert!(stdout(&out).contains(NOTHING_TO_CHANGE), "{}", stdout(&out));
}

#[test]
fn dry_run_writes_nothing_and_says_would() {
    // Arrange
    let (home, plugin) = codex_home();

    // Act
    let out = sync(&home, &plugin, &["--dry-run"]);

    // Assert
    assert!(out.status.success(), "{out:?}");
    assert!(stdout(&out).contains("would"), "{}", stdout(&out));
    assert!(!skills_dir(&home).exists());
}

#[test]
fn json_prints_the_report_lines_as_an_array() {
    // Arrange
    let (home, plugin) = codex_home();

    // Act
    let out = sync(&home, &plugin, &["--json"]);

    // Assert
    assert!(out.status.success(), "{out:?}");
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("JSON output");
    let lines = value.as_array().expect("an array");
    assert!(lines.iter().all(serde_json::Value::is_string));
    assert!(lines.len() > 1);
}

#[test]
fn no_codex_prints_the_not_found_line_and_exits_zero() {
    // Arrange
    let home = isolated::fresh_home();
    let plugin = fixture_plugin(&home);

    // Act
    let out = sync(&home, &plugin, &[]);

    // Assert
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout(&out).trim(), NOT_FOUND_LINE);
    assert!(!skills_dir(&home).exists());
}

#[test]
fn a_missing_plugin_exits_one_with_the_resolver_line() {
    // Arrange
    let (home, _) = codex_home();
    let missing = home.join("no-such-plugin");

    // Act
    let out = sync(&home, &missing, &[]);

    // Assert
    assert_eq!(out.status.code(), Some(EXIT_FAILURE), "{out:?}");
    let text = format!("{}{}", stdout(&out), String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("playbook plugin not found"), "{text}");
}

#[test]
fn an_unknown_agent_exits_two_with_the_supported_list() {
    // Arrange
    let (home, plugin) = codex_home();

    // Act
    let out = sync(&home, &plugin, &["--agent", "cursor"]);

    // Assert
    assert_eq!(out.status.code(), Some(EXIT_USAGE), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("codex"));
}

#[cfg(unix)]
#[test]
fn an_unwritable_agents_folder_exits_one_with_a_failed_line_and_valid_json() {
    use std::os::unix::fs::PermissionsExt;
    const READ_ONLY: u32 = 0o555;
    const WRITABLE: u32 = 0o755;
    // Arrange
    let (home, plugin) = codex_home();
    let agents = home.join(".agents");
    fs::create_dir_all(&agents).unwrap();
    fs::set_permissions(&agents, fs::Permissions::from_mode(READ_ONLY)).unwrap();
    let probe = agents.join("probe");
    if fs::create_dir(&probe).is_ok() {
        fs::remove_dir(&probe).unwrap();
        return; // running as a user the mode does not bind
    }

    // Act
    let plain = sync(&home, &plugin, &[]);
    let json = sync(&home, &plugin, &["--json"]);

    // Assert
    fs::set_permissions(&agents, fs::Permissions::from_mode(WRITABLE)).unwrap();
    assert_eq!(plain.status.code(), Some(EXIT_FAILURE), "{plain:?}");
    assert!(
        stdout(&plain).lines().any(|l| l.starts_with("failed ")),
        "{}",
        stdout(&plain)
    );
    assert_eq!(json.status.code(), Some(EXIT_FAILURE), "{json:?}");
    let value: serde_json::Value = serde_json::from_str(&stdout(&json)).expect("JSON output");
    assert!(value.as_array().expect("an array").len() > 1);
}
