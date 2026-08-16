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
//! - Every written command for a ported hook resolves to a real `HookName`,
//!   and is a bare name rather than a path (the "bare-name assumption"),
//!   and the 11 ported hooks are exactly the ones wired that way:
//!   `every_ported_hook_command_is_a_bare_playbook_hook_invocation_that_resolves`
//! - A pre-existing user hook entry survives wiring, unclobbered:
//!   `pre_existing_user_hook_entry_is_preserved_not_clobbered`
//! - Regression pin, scoped per the 2026-08-16 ADR amendment: only the four
//!   guards may still point under `~/.claude/hooks/` after wiring, since
//!   their Rust ports are still stubs until WU-13:
//!   `only_the_four_guards_still_point_under_claude_hooks_dir_after_wiring`
//! - The bare-name form survives a write-then-read round trip:
//!   `bare_name_form_survives_write_then_read_round_trip`
//! - Backup is timestamped and taken only when a change actually lands:
//!   `settings_json_is_backed_up_before_a_real_change_and_not_on_a_no_op`
//! - Regression pin for the defect itself: every hook name wired in binary
//!   form has a real (non-stub) Rust implementation behind it:
//!   `every_hook_wired_in_binary_form_has_a_non_stub_implementation`

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
fn every_ported_hook_command_is_a_bare_playbook_hook_invocation_that_resolves() {
    // Arrange
    let path = scratch_settings_path("resolves");
    write_json(&path, &unwired_fixture());

    // Act
    wire(&path).expect("wire should succeed");
    let settings = read_json(&path);
    let commands = all_hook_commands(&settings);

    // Assert: every command that takes the bare form is bare (no path
    // separator, no absolute path, no legacy python/shell script suffix)
    // and its hook name resolves to a real HookName the same way clap would
    // parse `playbook hook <name>`. The four guards deliberately do NOT
    // take this form yet; they are covered separately in
    // `only_the_four_guards_still_point_under_claude_hooks_dir_after_wiring`.
    assert!(!commands.is_empty(), "wiring should write hook commands");
    let mut resolved_names = std::collections::HashSet::new();
    for cmd in &commands {
        let Some(name) = cmd.strip_prefix("playbook hook ") else {
            continue;
        };
        assert!(
            !name.contains('/') && !name.contains('\\'),
            "hook name segment should carry no path separators: {cmd}"
        );
        HookName::from_str(name, false)
            .unwrap_or_else(|_| panic!("'{name}' should resolve to a real HookName: {cmd}"));
        resolved_names.insert(name.to_string());
    }

    // The 11 already-ported HookName variants should be wired in binary
    // form at least once: this is the pivot the whole Work Unit exists for,
    // so pin it directly rather than only checking the commands that happen
    // to be present. The four guards are excluded on purpose: their Rust
    // ports are still stubs until WU-13, so wiring them here would silently
    // disable them, which is exactly the defect the 2026-08-16 ADR
    // amendment records.
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
    ]
    .into_iter()
    .collect();
    let resolved_names: std::collections::HashSet<&str> =
        resolved_names.iter().map(String::as_str).collect();
    assert_eq!(
        resolved_names, expected,
        "wiring should register exactly the 11 ported HookName variants in binary form"
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
fn only_the_four_guards_still_point_under_claude_hooks_dir_after_wiring() {
    // Arrange: today's real shape, all four guards under ~/.claude/hooks/.
    let path = scratch_settings_path("no-hooks-dir-paths");
    write_json(&path, &unwired_fixture());

    // Act
    wire(&path).expect("wire should succeed");
    let settings = read_json(&path);
    let commands = all_hook_commands(&settings);

    // Assert: the 2026-08-16 ADR amendment moved the original "no command
    // may point under ~/.claude/hooks/" pin to WU-13, the unit where it
    // becomes true for every hook. Until then it holds for the 11 ported
    // hooks, and the four guards are the sole, explicit exception, since
    // their Rust ports are still stubs.
    let expected_guard_paths: std::collections::HashSet<String> = [
        "rm-workspace-guard",
        "bg-await-guard",
        "no-dash-guard",
        "precommit-check",
    ]
    .into_iter()
    .map(|name| format!("~/.claude/hooks/{name}.sh"))
    .collect();

    for cmd in &commands {
        if cmd.contains("/.claude/hooks/") {
            assert!(
                expected_guard_paths.contains(cmd),
                "only the four unported guards may still point under ~/.claude/hooks/: {cmd}"
            );
        }
    }
    for expected in &expected_guard_paths {
        assert!(
            commands.contains(expected),
            "each guard should still be wired to its working shell script: {expected}"
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

/// The literal body every stub hook module in `src/hooks/` starts life
/// with, per `src/hooks/mod.rs`'s own description of the pre-port shape
/// ("takes the parsed payload and does nothing"). A module whose source
/// still contains this exact line has not been ported yet, regardless of
/// what `wire()` claims to route it to.
const STUB_HOOK_BODY: &str = "pub fn run(_payload: &Payload) {}";

/// Regression pin for the defect this fix addresses: WU-8 wired every hook,
/// guards included, to the bare `playbook hook <name>` binary form while
/// the four guards' Rust modules were still empty stubs, which silently
/// disabled them (a shell guard denies a dangerous command; the Rust stub
/// prints nothing and exits 0). A test asserting only the command STRING
/// that was written would not have caught this, since the string was
/// correct; only the module behind it was not. This test instead reads
/// wire()'s own output back and checks each hook it wired in binary form
/// against its actual Rust source, so it keeps working unmodified once
/// WU-13 ports the guards and `GUARD_SPECS` starts wiring them the same
/// way: the moment a `HookSpec` flips to `ported: true`, this test starts
/// checking it too, with no hardcoded hook-name list to update by hand.
#[test]
fn every_hook_wired_in_binary_form_has_a_non_stub_implementation() {
    // Arrange: wire a fresh install, so every hook wire() currently wires
    // in binary form shows up in the output, without hardcoding which
    // those are.
    let path = scratch_settings_path("non-stub-binary-form");

    // Act
    wire(&path).expect("wire should succeed on a fresh install");
    let settings = read_json(&path);
    let commands = all_hook_commands(&settings);

    // Assert
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0;
    for cmd in &commands {
        let Some(name) = cmd.strip_prefix("playbook hook ") else {
            continue; // not wired in binary form, e.g. a guard still on its .sh script
        };
        checked += 1;
        let module_path = manifest_dir
            .join("src/hooks")
            .join(format!("{}.rs", name.replace('-', "_")));
        let source = fs::read_to_string(&module_path).unwrap_or_else(|err| {
            panic!("hook module for '{name}' should exist at {module_path:?}: {err}")
        });
        assert!(
            !source.contains(STUB_HOOK_BODY),
            "'{name}' is wired to `playbook hook {name}` in settings.json, but \
             {module_path:?} is still the empty stub. Wiring a hook to its binary \
             form before its Rust port lands silently disables it: this is the \
             exact WU-8 defect the 2026-08-16 ADR amendment records. Either port \
             the hook first, or leave it on its legacy command until the port lands."
        );
    }
    assert!(
        checked > 0,
        "wiring a fresh install should write at least one hook in binary form"
    );
}
