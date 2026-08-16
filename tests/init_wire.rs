// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `playbook::init::wire`, the module that writes
//! every hook Claude Code can invoke into `settings.json` as a bare
//! `playbook hook <name>` command and retires `hooks/hooks.json`.
//!
//! **Test isolation:** every test here operates on a fresh scratch directory
//! under the OS temp dir (`scratch_settings_path`), never on a real
//! `~/.claude/settings.json`. A test that wrote a developer's live settings
//! file would be a defect regardless of whether it passed.
//!
//! Coverage map, so every scenario named in the Work Unit brief is
//! traceable to one place below:
//! - Idempotence (assert on file bytes, not a return value):
//!   `running_wire_twice_writes_nothing_the_second_time`
//! - Every written command resolves to a real `HookName`, and is a bare
//!   name rather than a path (the "bare-name assumption"):
//!   `every_written_command_is_a_bare_playbook_hook_invocation_that_resolves`
//! - A pre-existing user hook entry survives wiring, unclobbered:
//!   `pre_existing_user_hook_entry_is_preserved_not_clobbered`
//! - Regression pin: no entry points under `~/.claude/hooks/` after wiring:
//!   `no_entry_points_under_claude_hooks_dir_after_wiring`
//! - The bare-name form survives a write-then-read round trip:
//!   `bare_name_form_survives_write_then_read_round_trip`
//! - Backup is timestamped and taken only when a change actually lands:
//!   `settings_json_is_backed_up_before_a_real_change_and_not_on_a_no_op`

#![allow(dead_code)]

use clap::ValueEnum;
use playbook::init::wire::wire;
use playbook::HookName;
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory under the OS temp dir, unique per call so
/// parallel tests never collide and none of them ever touch a real
/// `~/.claude/settings.json`.
fn scratch_dir(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = env::temp_dir().join(format!(
        "playbook-init-wire-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir should be creatable");
    dir
}

/// A `settings.json` path inside a fresh scratch directory; the file itself
/// is not created here, callers write it (or leave it absent to exercise
/// the fresh-install path).
fn scratch_settings_path(tag: &str) -> PathBuf {
    scratch_dir(tag).join("settings.json")
}

fn write_json(path: &PathBuf, value: &Value) {
    fs::write(path, serde_json::to_string_pretty(value).unwrap())
        .expect("scratch settings.json should be writable");
}

fn read_json(path: &PathBuf) -> Value {
    let text = fs::read_to_string(path).expect("wired settings.json should be readable");
    serde_json::from_str(&text).expect("wired settings.json should be valid JSON")
}

/// Every `command` string found anywhere under `.hooks` in `settings`,
/// walking every event, every group, and every hook entry.
fn all_hook_commands(settings: &Value) -> Vec<String> {
    let mut commands = Vec::new();
    let Some(hooks) = settings.get("hooks").and_then(Value::as_object) else {
        return commands;
    };
    for groups in hooks.values() {
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for group in groups {
            let Some(entries) = group.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for entry in entries {
                if let Some(cmd) = entry.get("command").and_then(Value::as_str) {
                    commands.push(cmd.to_string());
                }
            }
        }
    }
    commands
}

/// A `settings.json` shaped like the one `settings.shared.json` ships and a
/// real, unwired `~/.claude/settings.json` has today: the four safety guards
/// wired directly with legacy `~/.claude/hooks/*.sh` paths, and none of the
/// 11 functional hooks `hooks/hooks.json` used to register, since those
/// only ever lived in the retired plugin registry, never in `settings.json`
/// itself.
fn unwired_fixture() -> Value {
    json!({
        "$schema": "https://json.schemastore.org/claude-code-settings.json",
        "cleanupPeriodDays": 14,
        "permissions": {"allow": ["Read"], "deny": [], "ask": []},
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": "Bash",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "~/.claude/hooks/rm-workspace-guard.sh",
                            "if": "Bash(rm:*)",
                            "timeout": 10
                        },
                        {
                            "type": "command",
                            "command": "~/.claude/hooks/bg-await-guard.sh",
                            "timeout": 10
                        },
                        {
                            "type": "command",
                            "command": "~/.claude/hooks/no-dash-guard.sh",
                            "timeout": 10
                        },
                        {
                            "type": "command",
                            "command": "~/.claude/hooks/precommit-check.sh",
                            "if": "Bash(git commit:*)",
                            "timeout": 10
                        }
                    ]
                }
            ]
        },
        "skipAutoPermissionPrompt": false
    })
}

#[test]
fn running_wire_twice_writes_nothing_the_second_time() {
    // Arrange
    let path = scratch_settings_path("idempotence");
    write_json(&path, &unwired_fixture());

    // Act
    wire(&path).expect("first wire should succeed");
    let bytes_after_first = fs::read(&path).expect("settings.json should exist after wiring");
    wire(&path).expect("second wire should succeed");
    let bytes_after_second = fs::read(&path).expect("settings.json should still exist");

    // Assert: file bytes, not a return value, per the brief's instruction.
    assert_eq!(
        bytes_after_first, bytes_after_second,
        "a second wire() call must not change settings.json at all"
    );
}

#[test]
fn running_wire_twice_from_a_fresh_install_writes_nothing_the_second_time() {
    // Arrange: no settings.json exists yet at all.
    let path = scratch_settings_path("idempotence-fresh");

    // Act
    wire(&path).expect("first wire should succeed on a fresh install");
    let bytes_after_first = fs::read(&path).expect("settings.json should now exist");
    wire(&path).expect("second wire should succeed");
    let bytes_after_second = fs::read(&path).expect("settings.json should still exist");

    // Assert
    assert_eq!(
        bytes_after_first, bytes_after_second,
        "a second wire() call on a freshly wired file must be a no-op"
    );
}

#[test]
fn every_written_command_is_a_bare_playbook_hook_invocation_that_resolves() {
    // Arrange
    let path = scratch_settings_path("resolves");
    write_json(&path, &unwired_fixture());

    // Act
    wire(&path).expect("wire should succeed");
    let settings = read_json(&path);
    let commands = all_hook_commands(&settings);

    // Assert: every command is bare (no path separator, no absolute path,
    // no legacy python/shell script suffix), and its hook name resolves to
    // a real HookName the same way clap would parse `playbook hook <name>`.
    assert!(!commands.is_empty(), "wiring should write hook commands");
    let mut resolved_names = std::collections::HashSet::new();
    for cmd in &commands {
        let name = cmd
            .strip_prefix("playbook hook ")
            .unwrap_or_else(|| panic!("command should be a bare 'playbook hook <name>': {cmd}"));
        assert!(
            !name.contains('/') && !name.contains('\\'),
            "hook name segment should carry no path separators: {cmd}"
        );
        HookName::from_str(name, false)
            .unwrap_or_else(|_| panic!("'{name}' should resolve to a real HookName: {cmd}"));
        resolved_names.insert(name.to_string());
    }

    // All 15 HookName variants should be wired at least once: this is the
    // pivot the whole Work Unit exists for, so pin it directly rather than
    // only checking the commands that happen to be present.
    let expected: std::collections::HashSet<&str> = [
        "session-init",
        "preread-edit-check",
        "preread-size-check",
        "search-counter",
        "memory-anchors",
        "post-edit-track",
        "rebuild-memory-graph",
        "auto-model-detect",
        "precompact-warn",
        "session-clean-exit",
        "memory-capture",
        "rm-workspace-guard",
        "bg-await-guard",
        "no-dash-guard",
        "precommit-check",
    ]
    .into_iter()
    .collect();
    let resolved_names: std::collections::HashSet<&str> =
        resolved_names.iter().map(String::as_str).collect();
    assert_eq!(
        resolved_names, expected,
        "wiring should register exactly the 15 declared HookName variants"
    );
}

#[test]
fn pre_existing_user_hook_entry_is_preserved_not_clobbered() {
    // Arrange: the same unwired fixture, plus a hand-added hook entry in the
    // very same PreToolUse/Bash group the guards live in, and a second
    // hand-added entry on an event/matcher pair wire() never manages at all.
    let mut fixture = unwired_fixture();
    let bash_hooks = fixture["hooks"]["PreToolUse"][0]["hooks"]
        .as_array_mut()
        .unwrap();
    bash_hooks.push(json!({
        "type": "command",
        "command": "my-custom-bash-guard.sh"
    }));
    fixture["hooks"]["Notification"] = json!([
        {
            "hooks": [
                {"type": "command", "command": "my-custom-notifier.sh"}
            ]
        }
    ]);
    let path = scratch_settings_path("preserve-user-entry");
    write_json(&path, &fixture);

    // Act
    wire(&path).expect("wire should succeed");
    let settings = read_json(&path);

    // Assert
    let bash_hooks_after = settings["hooks"]["PreToolUse"][0]["hooks"]
        .as_array()
        .unwrap();
    assert!(
        bash_hooks_after
            .iter()
            .any(|h| h["command"] == "my-custom-bash-guard.sh"),
        "a user's hand-added Bash guard hook must survive wiring: {bash_hooks_after:?}"
    );
    assert_eq!(
        settings["hooks"]["Notification"][0]["hooks"][0]["command"], "my-custom-notifier.sh",
        "an entire event wire() does not manage must survive untouched"
    );
}

#[test]
fn no_entry_points_under_claude_hooks_dir_after_wiring() {
    // Arrange: today's real shape, all four guards under ~/.claude/hooks/.
    let path = scratch_settings_path("no-hooks-dir-paths");
    write_json(&path, &unwired_fixture());

    // Act
    wire(&path).expect("wire should succeed");
    let settings = read_json(&path);
    let commands = all_hook_commands(&settings);

    // Assert: the exact failure this Work Unit fixes, pinned directly.
    for cmd in &commands {
        assert!(
            !cmd.contains("/.claude/hooks/"),
            "no settings.json hook command may point under ~/.claude/hooks/ after wiring: {cmd}"
        );
    }
}

#[test]
fn bare_name_form_survives_write_then_read_round_trip() {
    // Arrange
    let path = scratch_settings_path("round-trip");
    write_json(&path, &unwired_fixture());

    // Act
    wire(&path).expect("wire should succeed");
    let reread = read_json(&path);

    // Assert: settings.json accepts a bare-name hook command from a
    // plugin-independent entry, the same shape a hand-written
    // "rtk hook claude" entry already takes; the write survives being read
    // back byte-for-string exactly, not just structurally.
    let session_start_command = reread["hooks"]["SessionStart"][0]["hooks"][0]["command"].clone();
    assert_eq!(
        session_start_command,
        Value::String("playbook hook session-init".to_string())
    );
}

#[test]
fn settings_json_is_backed_up_before_a_real_change_and_not_on_a_no_op() {
    // Arrange
    let dir = scratch_dir("backup");
    let path = dir.join("settings.json");
    write_json(&path, &unwired_fixture());
    let original_bytes = fs::read(&path).unwrap();

    // Act: first call changes the file, so it must back it up first.
    let first = wire(&path).expect("first wire should succeed");

    // Assert
    let backup_path = first
        .backup_path
        .expect("a real change must produce a backup path");
    assert!(
        backup_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("settings.json.bak."),
        "backup file name should be timestamped: {backup_path:?}"
    );
    let backup_bytes = fs::read(&backup_path).expect("backup file should exist on disk");
    assert_eq!(
        backup_bytes, original_bytes,
        "the backup should be a snapshot of settings.json before wiring changed it"
    );

    // Act: second call is a no-op, so it must take no further backup.
    let entries_before = fs::read_dir(&dir).unwrap().count();
    let second = wire(&path).expect("second wire should succeed");

    // Assert
    assert!(
        second.backup_path.is_none(),
        "a no-op wire() call should not report a new backup"
    );
    let entries_after = fs::read_dir(&dir).unwrap().count();
    assert_eq!(
        entries_before, entries_after,
        "a no-op wire() call should not create a second backup file"
    );
}
