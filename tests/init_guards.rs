// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `playbook::init::guards`, the module that copies
//! the four still-unported bash safety guards from `self_root/hooks/` into
//! `claude_home/hooks/` and reports, in a total `GuardOutcome`, which ones
//! landed (`wired`) and which did not (`failures`), so `init::wire` never
//! writes a `settings.json` command for a guard that is not on disk.
//!
//! Coverage map, so every scenario named in the Work Unit brief is
//! traceable to one place below:
//! - Fresh placement, all four guards land as executable regular files, and
//!   the full set is returned:
//!   `fresh_placement_puts_all_four_guards_as_executable_files_and_returns_all_four_names`
//! - Idempotence, a second call reports everything already current:
//!   `second_call_reports_all_four_already_current_and_changed_is_false`
//! - The executable bit is repaired, not assumed, pinning
//!   `is_already_current`'s deliberate bit check:
//!   `stripped_executable_bit_is_repaired_not_treated_as_current`
//! - A missing shipped source lands in `failures` naming the guard, without
//!   stopping the other three from being placed:
//!   `missing_source_fails_with_the_missing_guards_name`
//! - THE ORDERING PIN: a guard placement failure costs only the missing
//!   guard's command in `settings.json`, never the three guards that did
//!   land and never the eleven ported hooks, the regression pin for the
//!   `hook-rename-lockstep-settings` incident (~110 silent errors over 28
//!   hours on 2026-08-11). Drives the real `settings.shared.json` through
//!   `init::run::run`, whose `.hooks` still seeds all four guard commands
//!   (see `settings.shared.json`), so the missing guard's command really
//!   does reach `settings.json` first and `wire`'s existence-gated removal
//!   is what has to take it out again, not merely omit it:
//!   `guard_placement_failure_leaves_no_guard_command_but_still_wires_the_ported_hooks`
//! - The companion positive case: with all four guards shipped and the real
//!   template in play, all four guard commands and all four files land, and
//!   none is removed:
//!   `all_four_guards_present_with_real_template_wires_all_four_and_places_all_four_files`
//! - Derivation, not duplication, the set `place_guards` places matches the
//!   four guards `wire::GUARD_SPECS` still marks unported:
//!   `placed_guard_set_is_exactly_the_four_currently_unported_guards`
//! - THE REPAIR PATH, end to end: a `settings.json` that already carries a
//!   dangling guard command from before this fix shipped, on a machine whose
//!   shipped tree also lacks that guard's script, is cleaned up by a single
//!   `run`:
//!   `repair_path_removes_a_pre_existing_dangling_guard_command_end_to_end`
//!
//! Every test here operates on a scratch `self_root` and/or `claude_home`
//! under the OS temp dir; none read or write the developer's real
//! `~/.claude` or mutate the checked-out `hooks/*.sh` scripts this repo
//! ships. Tests that need a shipped tree missing a guard build their own
//! scratch copy rather than deleting anything from the real checkout, since
//! other tests running in parallel read that same checkout as their
//! `self_root`.

#![allow(dead_code)]

use playbook::init::guards::{place_guards, GuardError};
use playbook::init::run::{run, InitPaths, StepStatus};
use playbook::init::shim::ShellKind;
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The repo checkout root, where the real `hooks/*.sh` guard scripts,
/// `settings.shared.json`, `shell/bash/cc.sh`, `shell/zsh/cc.zsh`,
/// `shell/shared/*.sh` and `statusline.sh` all live, matching the helper of
/// the same name in `tests/init_run.rs` and `tests/init_shim.rs`.
fn self_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The four guard names `place_guards` should place, mirroring
/// `wire::GUARD_SPECS` filtered on `!ported`. `wire::GUARD_SPECS` is
/// private and `wire::unported_guard_names()` is `pub(crate)`, so neither is
/// reachable from this integration test binary; this list is asserted
/// directly against `place_guards`'s own return value in
/// `placed_guard_set_is_exactly_the_four_currently_unported_guards` below,
/// with a comment there on what WU-13 must do to this list by hand.
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

/// A scratch `self_root` holding a copy of every real guard script under
/// `hooks/`, standing in for `CLAUDE_PLUGIN_ROOT`. `place_guards` reads
/// nothing else from `self_root`, so this is the whole fixture most tests
/// below need. `skip`, when present, is left out of the copy, letting a
/// test exercise a shipped tree missing one guard without ever touching the
/// real repo checkout other tests read from concurrently.
fn scratch_self_root_with_guards(tag: &str, skip: Option<&str>) -> PathBuf {
    let root = scratch_dir(tag);
    let hooks_dir = root.join("hooks");
    fs::create_dir_all(&hooks_dir).expect("hooks dir should be creatable");
    for name in GUARD_NAMES {
        if Some(*name) == skip {
            continue;
        }
        let src = self_root().join("hooks").join(format!("{name}.sh"));
        let dst = hooks_dir.join(format!("{name}.sh"));
        fs::copy(&src, &dst)
            .unwrap_or_else(|err| panic!("copying {name}.sh should succeed: {err}"));
    }
    root
}

/// Same as `scratch_self_root_with_guards`, but also copies the real,
/// checked-out `settings.shared.json` into `self_root`, standing in for the
/// whole shipped tree `init::run::run` composes against: settings-merge,
/// then guards, then wire. The template still seeds all four guard commands
/// directly (see `settings.shared.json`: they stay legitimate template
/// content until WU-13 ports the guards' Rust bodies), so including it here
/// is deliberate, not incidental: it lets a test drive the true composed
/// path where the "settings" step seeds a guard command ahead of
/// `guards`/`hooks`, and `wire`'s existence-gated removal is what has to
/// clean up any command left dangling by a guard that could not be placed,
/// rather than that command simply never being written in the first place.
fn scratch_self_root_with_template(tag: &str, skip: Option<&str>) -> PathBuf {
    let root = scratch_self_root_with_guards(tag, skip);
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
fn fresh_placement_puts_all_four_guards_as_executable_files_and_returns_all_four_names() {
    // Arrange
    let self_root = scratch_self_root_with_guards("fresh", None);
    let claude_home = scratch_dir("fresh-claude-home");

    // Act
    let outcome = place_guards(&self_root, &claude_home);

    // Assert: every guard was freshly placed, none already current, none failed.
    assert_eq!(outcome.placed.len(), GUARD_NAMES.len());
    assert!(outcome.already_current.is_empty());
    assert!(outcome.failures.is_empty());
    assert!(outcome.changed());
    assert_eq!(
        outcome.wired,
        GUARD_NAMES.to_vec(),
        "place_guards should return every guard name in GUARD_SPECS order"
    );

    for name in GUARD_NAMES {
        let dest = claude_home.join("hooks").join(format!("{name}.sh"));
        let metadata = fs::metadata(&dest)
            .unwrap_or_else(|err| panic!("{name} should be placed at {dest:?}: {err}"));
        assert!(
            metadata.is_file(),
            "{name} should be placed as a regular file"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                metadata.permissions().mode() & 0o777,
                0o755,
                "{name} should be placed as mode 0755"
            );
        }
    }
}

#[test]
fn second_call_reports_all_four_already_current_and_changed_is_false() {
    // Arrange
    let self_root = scratch_self_root_with_guards("idempotent", None);
    let claude_home = scratch_dir("idempotent-claude-home");
    place_guards(&self_root, &claude_home);

    // Act
    let outcome = place_guards(&self_root, &claude_home);

    // Assert
    assert!(
        outcome.placed.is_empty(),
        "a second call should write nothing: {:?}",
        outcome.placed
    );
    assert_eq!(outcome.already_current.len(), GUARD_NAMES.len());
    assert!(outcome.failures.is_empty());
    assert!(!outcome.changed());
    assert_eq!(outcome.wired, GUARD_NAMES.to_vec());
}

#[cfg(unix)]
#[test]
fn stripped_executable_bit_is_repaired_not_treated_as_current() {
    use std::os::unix::fs::PermissionsExt;

    // Arrange: place fresh, then strip the executable bit from one guard
    // without touching its bytes.
    let self_root = scratch_self_root_with_guards("repair", None);
    let claude_home = scratch_dir("repair-claude-home");
    place_guards(&self_root, &claude_home);
    let stripped_name = "rm-workspace-guard";
    let dest = claude_home
        .join("hooks")
        .join(format!("{stripped_name}.sh"));
    fs::set_permissions(&dest, fs::Permissions::from_mode(0o644))
        .expect("stripping the executable bit should succeed");

    // Act
    let outcome = place_guards(&self_root, &claude_home);

    // Assert: identical bytes with a stripped bit must not count as
    // current, since `is_already_current` deliberately checks the
    // executable bit as well as the content.
    assert!(
        outcome.placed.contains(&dest),
        "the guard with a stripped executable bit should be rewritten: {:?}",
        outcome.placed
    );
    let mode = fs::metadata(&dest)
        .expect("rewritten guard should be readable")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o755,
        "the guard should be executable again after repair"
    );
}

#[test]
fn missing_source_fails_with_the_missing_guards_name() {
    // Arrange: the shipped tree is missing no-dash-guard.sh.
    let missing = "no-dash-guard";
    let self_root = scratch_self_root_with_guards("missing-source", Some(missing));
    let claude_home = scratch_dir("missing-source-claude-home");

    // Act
    let outcome = place_guards(&self_root, &claude_home);

    // Assert: exactly one failure, naming the missing guard.
    assert_eq!(
        outcome.failures.len(),
        1,
        "only the missing guard should fail: {:?}",
        outcome.failures
    );
    match &outcome.failures[0] {
        GuardError::MissingSource { name, path } => {
            assert_eq!(*name, missing);
            assert_eq!(*path, self_root.join("hooks").join(format!("{missing}.sh")));
        }
        other => panic!("expected GuardError::MissingSource naming '{missing}', got {other:?}"),
    }

    // Assert: the missing guard never reaches `wired`, but the three other
    // guards still land, proving this is a total function that does not
    // stop at the first failure.
    assert!(!outcome.wired.contains(&missing));
    assert_eq!(
        outcome.wired.len(),
        GUARD_NAMES.len() - 1,
        "the three guards whose source is present should still be placed: {:?}",
        outcome.wired
    );
}

/// THE ORDERING PIN. Drives the whole of `init::run::run` against an
/// `InitPaths` whose `self_root` is missing one guard's `.sh` file, and
/// asserts, by parsing the resulting `settings.json`, that a guard failure
/// costs only the missing guard's command, never the three guards that DID
/// land and never the eleven ported hooks. A `settings.json` naming a script
/// that is not on disk fails open and silent: that is the
/// `hook-rename-lockstep-settings` incident, roughly 110 silent errors over
/// 28 hours on 2026-08-11, which is exactly the failure `init::run`'s step
/// ordering (`guards` before `hooks`) exists to prevent.
///
/// Deliberately includes the real `settings.shared.json` via
/// `scratch_self_root_with_template`, rather than avoiding it. The template
/// DOES seed all four guard commands (`settings.shared.json`'s `.hooks`
/// still carries them; see that file and `shell/gen-shared-settings.py`), so
/// the "settings" step pre-bakes `bg-await-guard`'s command into
/// `settings.json` before `guards`/`hooks` ever run, exactly the seeded,
/// then-orphaned shape a fresh install produces when one guard's script
/// fails to ship. Including the template now exercises the true composed
/// path, settings-merge then guards then wire, rather than sidestepping it:
/// `wire` is what has to notice `bg-await-guard`'s command names a script
/// that never landed on `claude_home` and remove it, since `guards` never
/// reports it in `wired`.
#[test]
fn guard_placement_failure_leaves_no_guard_command_but_still_wires_the_ported_hooks() {
    // Arrange
    let missing = "bg-await-guard";
    let self_root = scratch_self_root_with_template("ordering-pin", Some(missing));
    let home = scratch_dir("ordering-pin-home");
    let claude_home = home.join(".claude");
    let paths = InitPaths {
        self_root: Some(self_root),
        claude_home: claude_home.clone(),
        home: home.clone(),
        shell_kind: Some(ShellKind::Bash),
        system_prompt: false,
    };

    // Act
    let outcome = run(&paths);

    // Assert: the run's outcome is not ok, and specifically the guards step
    // failed.
    assert!(!outcome.ok(), "a missing guard source should fail the run");
    let guards_step = outcome
        .steps
        .iter()
        .find(|s| s.name == "guards")
        .expect("a 'guards' step should be present in every run");
    assert_eq!(guards_step.status, StepStatus::Failed);

    let settings = read_json(&claude_home.join("settings.json"));
    let commands = all_hook_commands(&settings);

    // Assert: no command in settings.json mentions the missing guard's name.
    // The template seeded it during the "settings" step, same as the other
    // three; `wire` is what removed it again once `guards` reported it was
    // never placed and `claude_home` confirmed its script never landed.
    assert!(
        commands.iter().all(|cmd| !cmd.contains(missing)),
        "wire should have removed the missing guard's dangling command '{missing}': {commands:?}"
    );

    // Assert: settings.json DOES contain the three guards that landed,
    // proving partial placement is wired rather than discarded wholesale.
    for name in GUARD_NAMES.iter().filter(|&&n| n != missing) {
        let legacy = format!("~/.claude/hooks/{name}.sh");
        assert!(
            commands.contains(&legacy),
            "the landed guard '{name}' should still be wired: {commands:?}"
        );
    }

    // Assert: the eleven ported hooks are still wired; a guard failure must
    // not cost them.
    for name in PORTED_HOOK_NAMES {
        let bare = format!("playbook hook {name}");
        assert!(
            commands.contains(&bare),
            "missing ported hook '{bare}' in {commands:?}"
        );
    }
}

/// The companion positive case to the ordering pin above: with all four
/// guards shipped and the real `settings.shared.json` in play, `run` wires
/// all four guard commands into `settings.json` and places all four guard
/// files on disk, executable. Proves a fully shipped tree costs nothing: the
/// template seeds all four commands, `guards` places all four scripts, and
/// `wire`'s existence check confirms every one of them resolves, so none is
/// removed.
#[test]
fn all_four_guards_present_with_real_template_wires_all_four_and_places_all_four_files() {
    // Arrange
    let self_root = scratch_self_root_with_template("all-guards", None);
    let home = scratch_dir("all-guards-home");
    let claude_home = home.join(".claude");
    let paths = InitPaths {
        self_root: Some(self_root),
        claude_home: claude_home.clone(),
        home: home.clone(),
        shell_kind: Some(ShellKind::Bash),
        system_prompt: false,
    };

    // Act
    let outcome = run(&paths);

    // Assert: the guards step itself did not fail. `self_root` here only
    // ships `hooks/` and `settings.shared.json`, the two inputs
    // `place_guards` and the "settings" step read, not the whole shipped
    // tree (`shell/`, `statusline.sh`) other unrelated steps need, so this
    // deliberately checks the "guards" step rather than `outcome.ok()`.
    let guards_step = outcome
        .steps
        .iter()
        .find(|s| s.name == "guards")
        .expect("a 'guards' step should be present in every run");
    assert_ne!(guards_step.status, StepStatus::Failed);

    // Assert: all four guard commands are wired, and all four files landed
    // on disk as executables.
    let settings = read_json(&claude_home.join("settings.json"));
    let commands = all_hook_commands(&settings);
    for name in GUARD_NAMES {
        let legacy = format!("~/.claude/hooks/{name}.sh");
        assert!(
            commands.contains(&legacy),
            "guard '{name}' should be wired: {commands:?}"
        );
        let dest = claude_home.join("hooks").join(format!("{name}.sh"));
        assert!(
            dest.is_file(),
            "guard '{name}' should be placed on disk at {}",
            dest.display()
        );
    }
}

/// Derivation, not duplication: `place_guards` must place exactly the four
/// guards `wire::GUARD_SPECS` currently marks `ported: false`, derived at
/// runtime through `wire::unported_guard_names()` rather than restated as a
/// second hardcoded list inside `guards::place_guards` (see that module's
/// doc comment, point 1). `wire::unported_guard_names()` is `pub(crate)` and
/// `wire::GUARD_SPECS` is private, so neither is reachable from this
/// integration test binary; this test instead pins the four names it
/// should currently derive to. **When WU-13 flips a guard's `ported` flag
/// to `true`, `GUARD_NAMES` above must be updated by hand to drop it**, or
/// this test starts failing, on purpose: a silent three-guard shrink here
/// must never pass unnoticed.
#[test]
fn placed_guard_set_is_exactly_the_four_currently_unported_guards() {
    // Arrange
    let self_root = scratch_self_root_with_guards("derivation", None);
    let claude_home = scratch_dir("derivation-claude-home");

    // Act
    let outcome = place_guards(&self_root, &claude_home);

    // Assert
    let wired_set: std::collections::HashSet<&str> = outcome.wired.into_iter().collect();
    let expected_set: std::collections::HashSet<&str> = GUARD_NAMES.iter().copied().collect();
    assert_eq!(
        wired_set, expected_set,
        "place_guards should place exactly the four guards GUARD_SPECS still marks unported"
    );
}

/// THE REPAIR PATH, end to end. A machine set up before this fix shipped
/// already has a `settings.json` naming a guard script that never existed;
/// the shipped tree it is re-running `init` against still lacks that
/// guard's source too, so `guards` cannot place it on this run either. A
/// single `run` must still clean the dangling command up: `wire`'s
/// existence check runs against `claude_home`, not against whether the
/// command predates this run, so a stale hand-written entry and one the
/// "settings" step just seeded are repaired identically.
#[test]
fn repair_path_removes_a_pre_existing_dangling_guard_command_end_to_end() {
    // Arrange: self_root ships the template but is missing one guard's
    // script, and claude_home already has a settings.json naming that
    // guard's script directly, from before this fix shipped.
    let missing = "bg-await-guard";
    let self_root = scratch_self_root_with_template("repair-path", Some(missing));
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
                            "command": format!("~/.claude/hooks/{missing}.sh")
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
    };

    // Act
    run(&paths);

    // Assert: the pre-existing dangling command is gone.
    let settings = read_json(&claude_home.join("settings.json"));
    let commands = all_hook_commands(&settings);
    assert!(
        commands.iter().all(|cmd| !cmd.contains(missing)),
        "the pre-existing dangling command for '{missing}' should be repaired away: {commands:?}"
    );

    // Assert: the three guards whose scripts DID ship still land.
    for name in GUARD_NAMES.iter().filter(|&&n| n != missing) {
        let legacy = format!("~/.claude/hooks/{name}.sh");
        assert!(
            commands.contains(&legacy),
            "the landed guard '{name}' should still be wired: {commands:?}"
        );
    }
}
