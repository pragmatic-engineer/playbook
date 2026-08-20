// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `playbook::init::wire`, the module that writes
//! every hook Claude Code can invoke into `settings.json` as a bare
//! `playbook hook <name>` command, retires `hooks/hooks.json`, and, for a
//! guard not named in `placed_guards`, removes its legacy command when
//! `claude_home` genuinely has no script for it left.
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
//!   and all 15 ported hooks (the 11 functional hooks plus the 4 safety
//!   guards, all ported as of WU-13) are exactly the ones wired that way:
//!   `every_ported_hook_command_is_a_bare_playbook_hook_invocation_that_resolves`
//! - A pre-existing user hook entry survives wiring, unclobbered:
//!   `pre_existing_user_hook_entry_is_preserved_not_clobbered`
//! - Done-When pin: no command anywhere under `.hooks` points under
//!   `~/.claude/hooks/` after wiring, now that all four guards are ported:
//!   `no_hook_command_points_under_claude_hooks_dir_after_wiring`
//! - The bare-name form survives a write-then-read round trip:
//!   `bare_name_form_survives_write_then_read_round_trip`
//! - Backup is timestamped and taken only when a change actually lands:
//!   `settings_json_is_backed_up_before_a_real_change_and_not_on_a_no_op`
//! - Regression pin for the defect itself: every hook name wired in binary
//!   form has a real (non-stub) Rust implementation behind it:
//!   `every_hook_wired_in_binary_form_has_a_non_stub_implementation`
//! - THE SUBTLE PART pinned directly: a guard omitted from `placed_guards`,
//!   with no script on `claude_home` either, is still wired in bare form.
//!   A guard's `ported` flag alone decides this now; `placed_guards` and
//!   script presence are irrelevant to it, so flipping the flag without
//!   fixing the loop (the exact near-miss this Work Unit had to avoid)
//!   would fail this test:
//!   `guard_is_wired_in_bare_form_even_when_absent_from_placed_guards_and_script_missing`
//! - A pre-existing legacy `~/.claude/hooks/<name>.sh` guard command is
//!   REPLACED by the bare form, not left to coexist as a duplicate entry:
//!   `legacy_guard_entry_is_replaced_by_bare_form_not_duplicated`

#![allow(dead_code)]

use clap::ValueEnum;
use playbook::init::wire::wire;
use playbook::HookName;
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// The four guard names `wire` accepts as `placed_guards`. Every test below
/// was written when `wire` always wired all four guards unconditionally, so
/// passing the full set here reproduces that behaviour exactly and none of
/// the existing assertions need to change.
const ALL_GUARDS: &[&str] = &[
    "rm-workspace-guard",
    "bg-await-guard",
    "no-dash-guard",
    "precommit-check",
];

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

/// A scratch `claude_home` with no guard scripts on disk, standing in for
/// the third `wire()` argument in every test that does not care about the
/// existence-check gate itself. `wire` treats every unplaced guard as
/// genuinely dangling against a directory shaped like this one, which
/// matches every existing test's assumption from before that argument
/// existed.
fn scratch_claude_home(tag: &str) -> PathBuf {
    scratch_dir(tag)
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
    let claude_home = scratch_claude_home("idempotence-home");
    write_json(&path, &unwired_fixture());

    // Act
    wire(&path, ALL_GUARDS, &claude_home).expect("first wire should succeed");
    let bytes_after_first = fs::read(&path).expect("settings.json should exist after wiring");
    wire(&path, ALL_GUARDS, &claude_home).expect("second wire should succeed");
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
    let claude_home = scratch_claude_home("idempotence-fresh-home");

    // Act
    wire(&path, ALL_GUARDS, &claude_home).expect("first wire should succeed on a fresh install");
    let bytes_after_first = fs::read(&path).expect("settings.json should now exist");
    wire(&path, ALL_GUARDS, &claude_home).expect("second wire should succeed");
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
    let claude_home = scratch_claude_home("resolves-home");
    write_json(&path, &unwired_fixture());

    // Act
    wire(&path, ALL_GUARDS, &claude_home).expect("wire should succeed");
    let settings = read_json(&path);
    let commands = all_hook_commands(&settings);

    // Assert: every command that takes the bare form is bare (no path
    // separator, no absolute path, no legacy python/shell script suffix)
    // and its hook name resolves to a real HookName the same way clap would
    // parse `playbook hook <name>`. The four guards now take this form too,
    // since WU-13 ported all four to real Rust bodies.
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

    // All 15 HookName variants should be wired in binary form at least
    // once: this is the pivot the whole Work Unit exists for, so pin it
    // directly rather than only checking the commands that happen to be
    // present.
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
        "wiring should register exactly all 15 ported HookName variants in binary form"
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
    let claude_home = scratch_claude_home("preserve-user-entry-home");
    write_json(&path, &fixture);

    // Act
    wire(&path, ALL_GUARDS, &claude_home).expect("wire should succeed");
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
fn no_hook_command_points_under_claude_hooks_dir_after_wiring() {
    // Arrange: today's real pre-WU-13 shape, all four guards under
    // ~/.claude/hooks/, so wiring has to actually rewrite something rather
    // than trivially finding nothing to fix.
    let path = scratch_settings_path("no-hooks-dir-paths");
    let claude_home = scratch_claude_home("no-hooks-dir-paths-home");
    write_json(&path, &unwired_fixture());

    // Act
    wire(&path, ALL_GUARDS, &claude_home).expect("wire should succeed");
    let settings = read_json(&path);
    let commands = all_hook_commands(&settings);

    // Assert: the Done-When pin this Work Unit exists to satisfy. The
    // 2026-08-16 ADR amendment moved the original "no command may point
    // under ~/.claude/hooks/" pin to WU-13, the unit where all four guards
    // are ported and it becomes true unconditionally; there is no longer an
    // exception for the guards.
    for cmd in &commands {
        assert!(
            !cmd.contains("/.claude/hooks/"),
            "no hook command may point under ~/.claude/hooks/ after wiring: {cmd}"
        );
    }
    for name in [
        "rm-workspace-guard",
        "bg-await-guard",
        "no-dash-guard",
        "precommit-check",
    ] {
        assert!(
            commands.contains(&format!("playbook hook {name}")),
            "guard '{name}' should be wired to its bare binary form: {commands:?}"
        );
    }
}

#[test]
fn bare_name_form_survives_write_then_read_round_trip() {
    // Arrange
    let path = scratch_settings_path("round-trip");
    let claude_home = scratch_claude_home("round-trip-home");
    write_json(&path, &unwired_fixture());

    // Act
    wire(&path, ALL_GUARDS, &claude_home).expect("wire should succeed");
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
    let claude_home = scratch_claude_home("backup-home");
    write_json(&path, &unwired_fixture());
    let original_bytes = fs::read(&path).unwrap();

    // Act: first call changes the file, so it must back it up first.
    let first = wire(&path, ALL_GUARDS, &claude_home).expect("first wire should succeed");

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
    let second = wire(&path, ALL_GUARDS, &claude_home).expect("second wire should succeed");

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
    let claude_home = scratch_claude_home("non-stub-binary-form-home");

    // Act
    wire(&path, ALL_GUARDS, &claude_home).expect("wire should succeed on a fresh install");
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

/// THE SUBTLE PART, pinned directly. Before this Work Unit, a guard was
/// wired in `.sh` form only when its name was in `placed_guards`, and
/// otherwise had that command actively removed if `claude_home` had no
/// script for it. Flipping `GUARD_SPECS`'s `ported` field to `true` without
/// also fixing `wire`'s loop would leave a ported guard falling into that
/// same removal branch: nothing in `placed_guards` names it, no script
/// exists, so its OLD `.sh` command would be removed and nothing would take
/// its place, silently disabling the guard forever. This test constructs
/// exactly that near-miss (`placed_guards` empty, no scripts on
/// `claude_home` at all) and asserts every guard is wired in bare binary
/// form anyway, since `ported` alone decides it now.
#[test]
fn guard_is_wired_in_bare_form_even_when_absent_from_placed_guards_and_script_missing() {
    // Arrange: a fresh install, no settings.json, no placed guards, and a
    // claude_home with no hooks/ directory at all.
    let path = scratch_settings_path("guard-unconditional");
    let claude_home = scratch_claude_home("guard-unconditional-home");

    // Act: pass an empty placed_guards slice, the worst case for the old
    // placement-gate behaviour.
    wire(&path, &[], &claude_home).expect("wire should succeed");
    let settings = read_json(&path);
    let commands = all_hook_commands(&settings);

    // Assert
    for name in [
        "rm-workspace-guard",
        "bg-await-guard",
        "no-dash-guard",
        "precommit-check",
    ] {
        assert!(
            commands.contains(&format!("playbook hook {name}")),
            "guard '{name}' must be wired in bare form even when placed_guards is empty \
             and no script exists on claude_home: {commands:?}"
        );
        assert!(
            !commands.contains(&format!("~/.claude/hooks/{name}.sh")),
            "guard '{name}' must not be left on or reverted to its legacy .sh command: {commands:?}"
        );
    }
}

/// The scenario the brief asked to be added explicitly: a user's
/// pre-existing `settings.json` still carries a guard's legacy
/// `~/.claude/hooks/<name>.sh` command from before this fix shipped.
/// Wiring must REPLACE that entry with the bare form, not add a second
/// entry alongside it, since a guard firing twice on the same event is as
/// much a defect as it not firing at all.
#[test]
fn legacy_guard_entry_is_replaced_by_bare_form_not_duplicated() {
    // Arrange: the unwired fixture, today's real pre-WU-13 shape.
    let path = scratch_settings_path("legacy-guard-replaced");
    let claude_home = scratch_claude_home("legacy-guard-replaced-home");
    write_json(&path, &unwired_fixture());

    // Act
    wire(&path, ALL_GUARDS, &claude_home).expect("wire should succeed");
    let settings = read_json(&path);
    let bash_hooks_after = settings["hooks"]["PreToolUse"][0]["hooks"]
        .as_array()
        .unwrap();

    // Assert: exactly one entry per guard, and it is the bare form.
    for name in [
        "rm-workspace-guard",
        "bg-await-guard",
        "no-dash-guard",
        "precommit-check",
    ] {
        let bare = format!("playbook hook {name}");
        let matching: Vec<&Value> = bash_hooks_after
            .iter()
            .filter(|h| {
                h["command"] == bare || h["command"] == format!("~/.claude/hooks/{name}.sh")
            })
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "guard '{name}' should have exactly one entry after wiring, not a duplicate: \
             {bash_hooks_after:?}"
        );
        assert_eq!(
            matching[0]["command"], bare,
            "guard '{name}'s single entry should be the bare form: {matching:?}"
        );
    }
}
