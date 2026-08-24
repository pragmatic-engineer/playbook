// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `playbook::init::guards`.
//!
//! **This suite was rewritten as a direct consequence of WU-13.** Before this
//! Work Unit, `place_guards` copied the four still-unported bash safety
//! guards from `self_root/hooks/` into `claude_home/hooks/`; this suite's
//! previous revision drove that copy end to end. WU-13 flips every
//! `wire::GUARD_SPECS` entry's `ported` field to `true`, so
//! `wire::unported_guard_names()` (the set `place_guards` iterates) is now
//! permanently empty: `place_guards` no longer copies anything, on any
//! machine, regardless of what `self_root` or `claude_home` hold. This
//! suite's own former doc comment anticipated exactly this: "When WU-13
//! flips a guard's `ported` flag to `true`, `GUARD_NAMES` above must be
//! updated by hand to drop it, or this test starts failing, on purpose."
//! `init::guards`' own module doc comment explains why the module stays
//! (not deleted until WU-14) even though it is now inert.
//!
//! Coverage map:
//! - `place_guards` itself returns a fully empty, unchanged `GuardOutcome`
//!   no matter what `self_root` or `claude_home` contain, including when
//!   every guard script is genuinely absent (which used to be a `failures`
//!   entry, not a silent no-op):
//!   `place_guards_is_a_permanent_no_op_regardless_of_shipped_or_missing_scripts`
//! - The composed `guards` step inside `init::run::run` never fails and
//!   places nothing on disk, even when the shipped tree ships no guard
//!   scripts at all:
//!   `guards_step_never_fails_and_places_nothing_on_disk`
//! - The behaviour change this whole Work Unit exists to produce: `wire`
//!   alone, with no help from `guards`, wires all four guard commands (and
//!   the 11 ported hooks) into `settings.json` in bare binary form, even
//!   when the shipped tree carries zero guard scripts:
//!   `all_hook_and_guard_commands_are_wired_via_wire_alone_even_when_no_guard_script_ships`
//! - THE REPAIR PATH, end to end: a `settings.json` that already carries a
//!   guard's legacy `~/.claude/hooks/<name>.sh` command from before this fix
//!   shipped is REPLACED by the bare form by a single `run`, on a machine
//!   whose shipped tree also has no guard scripts at all:
//!   `repair_path_replaces_a_pre_existing_dangling_guard_command_with_the_bare_form`
//!
//! Every test here operates on a scratch `self_root` and/or `claude_home`
//! under the OS temp dir; none read or write the developer's real
//! `~/.claude` or mutate the checked-out `hooks/*.sh` scripts this repo
//! ships.

#![allow(dead_code)]

use playbook::init::guards::place_guards;
use playbook::init::run::{run, InitPaths, StepStatus};
use playbook::init::shim::ShellKind;
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The four guard names, for asserting their commands land wired even
/// though `place_guards` no longer places anything for any of them.
const GUARD_NAMES: &[&str] = &[
    "rm-workspace-guard",
    "bg-await-guard",
    "no-dash-guard",
    "precommit-check",
];

/// The 11 hooks `wire` writes as a bare `playbook hook <name>` invocation,
/// matching the constant of the same name in `tests/init_run.rs`.
const PORTED_HOOK_NAMES: &[&str] = &[
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
];

/// The repo checkout root, where `settings.shared.json` lives, matching the
/// helper of the same name in `tests/init_run.rs` and `tests/init_shim.rs`.
fn self_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory under the OS temp dir, unique per call so
/// parallel tests never collide and none of them ever touch a real
/// `~/.claude` or the checked-out repo.
fn scratch_dir(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = env::temp_dir().join(format!(
        "playbook-init-guards-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir should be creatable");
    dir
}

/// A scratch `self_root` shipping no `hooks/` directory at all: the shipped
/// tree an old, incomplete package would have, standing in for the worst
/// case `place_guards` can now be handed. It no longer matters, since
/// `place_guards`'s loop body never runs regardless of what is here.
fn scratch_self_root_without_guards(tag: &str) -> PathBuf {
    scratch_dir(tag)
}

/// Same as `scratch_self_root_without_guards`, but also copies the real,
/// checked-out `settings.shared.json` into `self_root`, standing in for the
/// shipped tree `init::run::run` composes against: settings-merge, then
/// guards, then wire. The template still seeds all four guard commands
/// directly (see `settings.shared.json`), so this lets a test drive the
/// true composed path where the "settings" step seeds a guard command ahead
/// of `guards`/`hooks`.
fn scratch_self_root_with_template(tag: &str) -> PathBuf {
    let root = scratch_self_root_without_guards(tag);
    let template_src = self_root().join("settings.shared.json");
    let template_dst = root.join("settings.shared.json");
    fs::copy(&template_src, &template_dst)
        .unwrap_or_else(|err| panic!("copying settings.shared.json should succeed: {err}"));
    root
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).expect("settings.json should be readable");
    serde_json::from_str(&text).expect("settings.json should be valid JSON")
}

/// Every `command` string found anywhere under `.hooks`, walking every
/// event, every group, and every hook entry. Mirrors the helper of the same
/// name in `tests/init_wire.rs` and `tests/init_run.rs`; duplicated rather
/// than shared, since integration test binaries in this crate each compile
/// standalone.
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

#[test]
fn place_guards_is_a_permanent_no_op_regardless_of_shipped_or_missing_scripts() {
    // Arrange: a self_root that ships no hooks/ directory, and a claude_home
    // with nothing on it either. Before WU-13 this shape would have produced
    // four `GuardError::MissingSource` failures; it no longer does, because
    // the loop `place_guards` runs over is empty from the start.
    let self_root = scratch_self_root_without_guards("no-op");
    let claude_home = scratch_dir("no-op-claude-home");

    // Act
    let outcome = place_guards(&self_root, &claude_home);

    // Assert: every field is empty, and nothing changed.
    assert!(outcome.placed.is_empty(), "{:?}", outcome.placed);
    assert!(
        outcome.already_current.is_empty(),
        "{:?}",
        outcome.already_current
    );
    assert!(outcome.wired.is_empty(), "{:?}", outcome.wired);
    assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
    assert!(!outcome.changed());
}

#[test]
fn guards_step_never_fails_and_places_nothing_on_disk() {
    // Arrange: the real settings.shared.json template, but self_root ships
    // no guard scripts at all.
    let self_root = scratch_self_root_with_template("step-no-op");
    let home = scratch_dir("step-no-op-home");
    let claude_home = home.join(".claude");
    let paths = InitPaths {
        self_root: Some(self_root),
        claude_home: claude_home.clone(),
        home: home.clone(),
        shell_kind: Some(ShellKind::Bash),
        system_prompt: false,
        aliases: true,
    };

    // Act
    let outcome = run(&paths);

    // Assert: the guards step is present and did not fail.
    let guards_step = outcome
        .steps
        .iter()
        .find(|s| s.name == "guards")
        .expect("a 'guards' step should be present in every run");
    assert_ne!(guards_step.status, StepStatus::Failed);

    // Assert: no guard script landed on claude_home, since place_guards
    // never attempts to place one any more.
    for name in GUARD_NAMES {
        let dest = claude_home.join("hooks").join(format!("{name}.sh"));
        assert!(
            !dest.exists(),
            "guard '{name}' should not be placed on disk: {}",
            dest.display()
        );
    }
}

/// The behaviour change this whole Work Unit exists to produce: `wire`
/// alone wires every guard command, with no help from `guards` at all.
#[test]
fn all_hook_and_guard_commands_are_wired_via_wire_alone_even_when_no_guard_script_ships() {
    // Arrange: the real template, self_root shipping zero guard scripts.
    let self_root = scratch_self_root_with_template("wire-alone");
    let home = scratch_dir("wire-alone-home");
    let claude_home = home.join(".claude");
    let paths = InitPaths {
        self_root: Some(self_root),
        claude_home: claude_home.clone(),
        home: home.clone(),
        shell_kind: Some(ShellKind::Bash),
        system_prompt: false,
        aliases: true,
    };

    // Act
    run(&paths);

    // Assert: all four guard commands are wired in bare form.
    let settings = read_json(&claude_home.join("settings.json"));
    let commands = all_hook_commands(&settings);
    for name in GUARD_NAMES {
        let bare = format!("playbook hook {name}");
        assert!(
            commands.contains(&bare),
            "guard '{name}' should be wired via wire() alone: {commands:?}"
        );
    }

    // Assert: the 11 ported hooks are still wired too; nothing about the
    // guards' placement (or lack of it) should cost them.
    for name in PORTED_HOOK_NAMES {
        let bare = format!("playbook hook {name}");
        assert!(
            commands.contains(&bare),
            "missing ported hook '{bare}' in {commands:?}"
        );
    }
}

/// THE REPAIR PATH, end to end. A machine set up before this fix shipped
/// already has a `settings.json` naming a guard's legacy `.sh` script
/// directly. A single `run` against a shipped tree with no guard scripts at
/// all still REPLACES that stale command with the bare binary form, rather
/// than merely removing it (the pre-WU-13 behaviour) or leaving it in place.
#[test]
fn repair_path_replaces_a_pre_existing_dangling_guard_command_with_the_bare_form() {
    // Arrange: self_root ships the template but no guard scripts at all, and
    // claude_home already has a settings.json naming one guard's script
    // directly, from before this fix shipped.
    let target = "bg-await-guard";
    let self_root = scratch_self_root_with_template("repair-path");
    let home = scratch_dir("repair-path-home");
    let claude_home = home.join(".claude");
    fs::create_dir_all(&claude_home).expect("claude_home should be creatable");
    let stale_settings = serde_json::json!({
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": "Bash",
                    "hooks": [
                        {
                            "type": "command",
                            "command": format!("~/.claude/hooks/{target}.sh")
                        }
                    ]
                }
            ]
        }
    });
    fs::write(
        claude_home.join("settings.json"),
        serde_json::to_string_pretty(&stale_settings).unwrap(),
    )
    .expect("stale settings.json should be writable");
    let paths = InitPaths {
        self_root: Some(self_root),
        claude_home: claude_home.clone(),
        home: home.clone(),
        shell_kind: Some(ShellKind::Bash),
        system_prompt: false,
        aliases: true,
    };

    // Act
    run(&paths);

    // Assert: the stale legacy command is gone, replaced by the bare form.
    let settings = read_json(&claude_home.join("settings.json"));
    let commands = all_hook_commands(&settings);
    assert!(
        !commands.contains(&format!("~/.claude/hooks/{target}.sh")),
        "the pre-existing dangling command for '{target}' should be gone: {commands:?}"
    );
    assert!(
        commands.contains(&format!("playbook hook {target}")),
        "'{target}' should be repaired to its bare binary form, not merely removed: {commands:?}"
    );
}
