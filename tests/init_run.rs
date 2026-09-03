// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `playbook::init::run`, the module that composes
//! `merge`, `wire`, `shim` and `statusline` into `Command::Init`. Coverage
//! map:
//! - a fresh config gets fully wired: `fresh_config_gets_fully_wired`
//! - running twice is idempotent, no second-run changes:
//!   `running_init_twice_is_idempotent_with_no_second_run_changes`
//! - a hand-added user hook entry survives: `hand_added_hook_entry_survives_composed_init`
//! - a malformed `settings.json` fails cleanly, never panics, and never
//!   writes: `malformed_settings_json_fails_without_panicking_or_writing`
//!   (library level) and `binary_malformed_settings_json_exits_non_zero`
//!   (the real compiled binary, since the exit code itself is `main.rs`'s
//!   contract, not `run`'s)
//! - the reported summary matches what changed on disk: asserted inline in
//!   every test above via the returned `StepStatus`, plus
//!   `missing_self_root_skips_template_dependent_steps` and
//!   `unrecognised_shell_skips_shim_only` for the two ways a step is
//!   legitimately skipped rather than wired or failed
//! - regression pin: a full composed `init` on a clean scratch HOME must
//!   never write a guard command
//!   in its old `~/.claude/hooks/<name>.sh` path form, even when that form
//!   would itself resolve (a guard's script still ships in this repo, so a
//!   regression here would not be caught by "does the path exist"; only
//!   "does the guard still take path form at all" catches it):
//!   `zero_hook_commands_point_under_claude_hooks_dir_after_a_full_init`
//! - the backup scheme and skip-report:
//!   a withheld customisation produces a readable skip-report alongside the
//!   backup:
//!   `withheld_customisation_produces_a_skip_report_alongside_the_backup`;
//!   stale backup and skip-report files beyond the retain-5 threshold, at
//!   and past the boundary, are pruned by one real write:
//!   `stale_backup_and_skip_report_files_beyond_five_are_pruned_after_a_real_merge`;
//!   an idempotent re-run creates no new file in either family and prunes
//!   nothing:
//!   `idempotent_rerun_creates_no_backup_or_skip_report_and_prunes_nothing`;
//!   a fresh install's placeholder-then-merge sequence produces exactly one
//!   backup and no skip-report:
//!   `fresh_install_placeholder_produces_exactly_one_backup_and_no_skip_report`
//!
//! Every test uses a scratch directory standing in for `$HOME`; none read or
//! write the developer's real `~/.claude`.

#![allow(dead_code)]

use playbook::init::run::{run, InitOutcome, InitPaths, StepReport, StepStatus};
use playbook::init::shim::ShellKind;
use playbook::init::statusline::resolve_statusline_path;
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// The repo checkout root, where `settings.shared.json`, `shell/bash/cc.sh`,
/// `shell/zsh/cc.zsh`, `shell/shared/*.sh` and `statusline.sh` actually live,
/// standing in for `CLAUDE_PLUGIN_ROOT` on a real install.
fn self_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory under the OS temp dir, standing in for `$HOME`,
/// unique per call so parallel tests never collide and none of them ever
/// touch a real `~/.claude`.
fn scratch_home(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let home = env::temp_dir().join(format!(
        "playbook-init-run-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&home).expect("scratch home should be creatable");
    home
}

fn claude_home_of(home: &Path) -> PathBuf {
    home.join(".claude")
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent dir should be creatable");
    }
    fs::write(path, serde_json::to_string_pretty(value).unwrap())
        .expect("scratch settings.json should be writable");
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).expect("settings.json should be readable");
    serde_json::from_str(&text).expect("settings.json should be valid JSON")
}

/// Every `command` string found anywhere under `.hooks`, walking every
/// event, every group, and every hook entry. Mirrors `tests/init_wire.rs`'s
/// helper of the same name; duplicated rather than shared, since integration
/// test binaries in this crate each compile standalone.
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

/// The 11 hooks `wire` writes as a bare `playbook hook <name>` invocation.
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

/// The 4 safety guards, wired the same as `PORTED_HOOK_NAMES`.
const GUARD_HOOK_NAMES: &[&str] = &[
    "rm-workspace-guard",
    "bg-await-guard",
    "no-slop-guard",
    "precommit-check",
];

/// File names directly under `dir` that start with `prefix`, for asserting
/// on the backup and skip-report families (`settings.json.bak.` and
/// `settings-merge-skipped.`) without depending on directory iteration
/// order.
fn matching_entries(dir: &Path, prefix: &str) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(prefix))
        .collect()
}

fn find_step<'a>(outcome: &'a InitOutcome, name: &str) -> &'a StepReport {
    outcome
        .steps
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| {
            panic!(
                "no '{name}' step in {:?}",
                outcome.steps.iter().map(|s| s.name).collect::<Vec<_>>()
            )
        })
}

fn base_paths(home: &Path, shell_kind: Option<ShellKind>) -> InitPaths {
    InitPaths {
        self_root: Some(self_root()),
        claude_home: claude_home_of(home),
        home: home.to_path_buf(),
        shell_kind,
        system_prompt: false,
        aliases: true,
    }
}

#[test]
fn fresh_config_gets_fully_wired() {
    // Arrange: a machine with no `~/.claude` at all.
    let home = scratch_home("fresh");
    let claude_home = claude_home_of(&home);
    let paths = base_paths(&home, Some(ShellKind::Bash));

    // Act
    let outcome = run(&paths);

    // Assert: every step ran and actually changed something.
    assert!(
        outcome.ok(),
        "expected every step to succeed: {:?}",
        outcome
            .steps
            .iter()
            .map(StepReport::render)
            .collect::<Vec<_>>()
    );
    for step in &outcome.steps {
        // `system-prompt` is opt-in: `system_prompt: false` and no
        // pre-existing file correctly reports `Skipped` rather than
        // `Wired`, per `init::system_prompt`'s documented opt-in rules.
        // `tests/init_system_prompt.rs` covers this step's own scenarios.
        if step.name == "system-prompt" {
            assert_eq!(
                step.status,
                StepStatus::Skipped,
                "expected 'system-prompt' to be skipped without --system-prompt and no existing file: {}",
                step.detail
            );
            continue;
        }
        // `memory-migrate` and `memory-root-migrate` have nothing to
        // migrate on a fresh machine with no legacy `~/.claude/memory`.
        if step.name == "memory-migrate" || step.name == "memory-root-migrate" {
            assert_eq!(
                step.status,
                StepStatus::Skipped,
                "expected '{}' to be skipped with no legacy memory present: {}",
                step.name,
                step.detail
            );
            continue;
        }
        assert_eq!(
            step.status,
            StepStatus::Wired,
            "expected '{}' to be wired on a fresh config: {}",
            step.name,
            step.detail
        );
    }

    // Assert: every ported hook and every guard resolved into settings.json.
    let settings_path = claude_home.join("settings.json");
    let settings = read_json(&settings_path);
    let commands = all_hook_commands(&settings);
    for name in PORTED_HOOK_NAMES {
        let bare = format!("playbook hook {name}");
        assert!(
            commands.contains(&bare),
            "missing ported hook '{bare}' in {commands:?}"
        );
    }
    for name in GUARD_HOOK_NAMES {
        let bare = format!("playbook hook {name}");
        assert!(
            commands.contains(&bare),
            "missing guard '{bare}' in {commands:?}"
        );
    }

    // Assert: the merge baseline, the shim, and the statusline all landed.
    assert!(claude_home.join(".settings.base.json").is_file());
    let rc = fs::read_to_string(home.join(".bashrc")).expect(".bashrc should exist");
    assert!(rc.contains("shell/bash/cc.sh"));
    assert!(claude_home.join("shell/bash/cc.sh").is_file());
    let statusline_dest =
        resolve_statusline_path(&settings_path, &home).expect("statusLine.command should resolve");
    assert_eq!(
        fs::read(&statusline_dest).expect("placed statusline should be readable"),
        fs::read(self_root().join("statusline.sh"))
            .expect("shipped statusline.sh should be readable")
    );
}

/// D6 regression pin: `wire`'s guard loop once gated a guard's bare
/// `playbook hook <name>` form on whether `init::guards` had actually placed
/// a script for it, so flipping a `GUARD_SPECS` entry's `ported` field to
/// `true` would have silently done nothing, since a ported guard is never
/// placed. All guards are ported now, so this must hold unconditionally
/// on a fresh install: not one `.hooks` command may name a path under
/// `~/.claude/hooks/`, regardless of whether that path happens to exist.
/// Asserting only "every command that IS a path resolves" would miss this
/// exact regression, since a guard's `.sh` script still ships in this repo
/// and would resolve if `wire` reverted to writing it; the invariant that
/// actually matters is that a ported guard never takes path form at all.
#[test]
fn zero_hook_commands_point_under_claude_hooks_dir_after_a_full_init() {
    // Arrange: a machine with no `~/.claude` at all, the same clean-install
    // shape `fresh_config_gets_fully_wired` exercises.
    let home = scratch_home("zero-hooks-dir");
    let claude_home = claude_home_of(&home);
    let paths = base_paths(&home, Some(ShellKind::Bash));

    // Act
    let outcome = run(&paths);

    // Assert
    assert!(outcome.ok(), "expected every step to succeed");
    let settings = read_json(&claude_home.join("settings.json"));
    let commands = all_hook_commands(&settings);
    assert!(
        !commands.is_empty(),
        "a fresh init should write hook commands"
    );
    for cmd in &commands {
        assert!(
            !cmd.contains("/.claude/hooks/"),
            "no hook command may point under ~/.claude/hooks/ after a full init: {cmd}"
        );
    }
}

/// WU-14 scenario 1: a customisation that collides with a template update
/// must produce a readable skip-report JSON file, not just a printed count,
/// alongside the `.bak.<epoch>` backup the write itself already takes.
#[test]
fn withheld_customisation_produces_a_skip_report_alongside_the_backup() {
    // Arrange: BASE differs from both what the template ships
    // (`cleanupPeriodDays: 14` in `settings.shared.json`) and what the user
    // customised, so the key is genuinely contested.
    let home = scratch_home("skip-report");
    let claude_home = claude_home_of(&home);
    write_json(
        &claude_home.join(".settings.base.json"),
        &json!({"cleanupPeriodDays": 999}),
    );
    write_json(
        &claude_home.join("settings.json"),
        &json!({"cleanupPeriodDays": 500}),
    );
    let paths = base_paths(&home, Some(ShellKind::Bash));

    // Act
    let outcome = run(&paths);

    // Assert
    assert!(
        outcome.ok(),
        "{:?}",
        outcome
            .steps
            .iter()
            .map(StepReport::render)
            .collect::<Vec<_>>()
    );
    assert_eq!(find_step(&outcome, "settings").status, StepStatus::Wired);

    let bak_files = matching_entries(&claude_home, "settings.json.bak.");
    assert_eq!(
        bak_files.len(),
        1,
        "expected exactly one backup file: {bak_files:?}"
    );

    let skip_files = matching_entries(&claude_home, "settings-merge-skipped.");
    assert_eq!(
        skip_files.len(),
        1,
        "expected exactly one skip-report file: {skip_files:?}"
    );
    let skip_content = fs::read_to_string(claude_home.join(&skip_files[0])).unwrap();
    let skip_json: Value = serde_json::from_str(&skip_content)
        .unwrap_or_else(|e| panic!("skip-report should be valid JSON: {e}: {skip_content}"));
    let entries = skip_json
        .as_array()
        .expect("skip-report should be a JSON array");
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one withheld key: {entries:?}"
    );
    assert_eq!(entries[0]["key"], "cleanupPeriodDays");
    assert_eq!(entries[0]["template_had"], 14);
    assert_eq!(entries[0]["yours"], 500);
}

/// WU-14 scenarios 2, 3 and 4: table-driven, since all three share the same
/// "seed N fabricated-epoch files, run one real merge, count survivors"
/// shape and differ only in file family and prior count. Cases 1 and 2 pin
/// the exact 5-to-6 boundary for the backup family (5 prior -> 5 survive,
/// then 6 prior -> 5 survive); case 3 is the symmetric check for the
/// skip-report family.
#[test]
fn stale_backup_and_skip_report_files_beyond_five_are_pruned_after_a_real_merge() {
    struct Case {
        tag: &'static str,
        prior_count: u64,
        prefix: &'static str,
        suffix: &'static str,
        seed_content: &'static str,
    }

    let cases = [
        Case {
            tag: "backups-at-threshold",
            prior_count: 5,
            prefix: "settings.json.bak.",
            suffix: "",
            seed_content: "stale",
        },
        Case {
            tag: "backups-over-threshold",
            prior_count: 6,
            prefix: "settings.json.bak.",
            suffix: "",
            seed_content: "stale",
        },
        Case {
            tag: "skip-reports-over-threshold",
            prior_count: 6,
            prefix: "settings-merge-skipped.",
            suffix: ".json",
            seed_content: "[]",
        },
    ];

    for case in cases {
        // Arrange: fabricated, distinct epochs seeded directly as file
        // names, not by looping real writes, since `backup_then_write`'s
        // epoch has 1-second granularity and a tight loop can collide on the
        // same file name.
        let home = scratch_home(case.tag);
        let claude_home = claude_home_of(&home);
        fs::create_dir_all(&claude_home).unwrap();
        for i in 0..case.prior_count {
            let epoch = 1_000_000_000u64 + i;
            fs::write(
                claude_home.join(format!("{}{epoch}{}", case.prefix, case.suffix)),
                case.seed_content,
            )
            .unwrap();
        }
        // A genuinely contested key, so every case's run writes both a real
        // backup and a real skip-report, regardless of which family the
        // case is pinning.
        write_json(
            &claude_home.join(".settings.base.json"),
            &json!({"cleanupPeriodDays": 999}),
        );
        write_json(
            &claude_home.join("settings.json"),
            &json!({"cleanupPeriodDays": 500}),
        );
        let paths = base_paths(&home, Some(ShellKind::Bash));

        // Act
        let outcome = run(&paths);

        // Assert
        assert!(
            outcome.ok(),
            "{}: {:?}",
            case.tag,
            outcome
                .steps
                .iter()
                .map(StepReport::render)
                .collect::<Vec<_>>()
        );
        let survivors = matching_entries(&claude_home, case.prefix);
        assert_eq!(
            survivors.len(),
            5,
            "{}: {} prior file(s) plus one new real write should retain exactly 5: {survivors:?}",
            case.tag,
            case.prior_count
        );
    }
}

/// WU-14 scenario 5: an idempotent re-run must create no new backup or
/// skip-report file, and must not prune either family, even when both
/// already sit well past the retain-5 threshold.
#[test]
fn idempotent_rerun_creates_no_backup_or_skip_report_and_prunes_nothing() {
    // Arrange: run once to reach a stable, merged state (a real write, one
    // backup taken, no skip-report since nothing was withheld on a fresh
    // install), then seed extra stale files in both families beyond the
    // retain-5 threshold.
    let home = scratch_home("idempotent-skip");
    let claude_home = claude_home_of(&home);
    let paths = base_paths(&home, Some(ShellKind::Bash));
    let first = run(&paths);
    assert!(first.ok());

    for i in 0..6u64 {
        let epoch = 1_000_000_000u64 + i;
        fs::write(
            claude_home.join(format!("settings.json.bak.{epoch}")),
            "stale",
        )
        .unwrap();
        fs::write(
            claude_home.join(format!("settings-merge-skipped.{epoch}.json")),
            "[]",
        )
        .unwrap();
    }
    let bak_count_before = matching_entries(&claude_home, "settings.json.bak.").len();
    let skip_count_before = matching_entries(&claude_home, "settings-merge-skipped.").len();

    // Act: a second, idempotent run.
    let second = run(&paths);

    // Assert: settings reports no change, and neither seeded family was
    // touched at all, proving pruning did not fire.
    assert!(second.ok());
    assert_eq!(
        find_step(&second, "settings").status,
        StepStatus::AlreadyCorrect
    );
    let bak_count_after = matching_entries(&claude_home, "settings.json.bak.").len();
    let skip_count_after = matching_entries(&claude_home, "settings-merge-skipped.").len();
    assert_eq!(
        bak_count_after, bak_count_before,
        "an idempotent re-run must not create or prune backup files"
    );
    assert_eq!(
        skip_count_after, skip_count_before,
        "an idempotent re-run must not create or prune skip-report files"
    );
}

/// WU-14 scenario 6: a fresh install's placeholder-then-merge sequence still
/// backs up the placeholder (pre-existing behaviour, per `run.rs:336-345`),
/// producing exactly one backup file; nothing is withheld on a fresh
/// install, so no skip-report file is produced.
#[test]
fn fresh_install_placeholder_produces_exactly_one_backup_and_no_skip_report() {
    // Arrange: no prior `~/.claude` at all.
    let home = scratch_home("fresh-placeholder-backup");
    let claude_home = claude_home_of(&home);
    let paths = base_paths(&home, Some(ShellKind::Bash));

    // Act
    let outcome = run(&paths);

    // Assert
    assert!(outcome.ok());
    assert_eq!(find_step(&outcome, "settings").status, StepStatus::Wired);
    let bak_files = matching_entries(&claude_home, "settings.json.bak.");
    assert_eq!(
        bak_files.len(),
        1,
        "expected exactly one backup file from the placeholder write: {bak_files:?}"
    );
    let skip_files = matching_entries(&claude_home, "settings-merge-skipped.");
    assert!(
        skip_files.is_empty(),
        "a fresh install should withhold nothing: {skip_files:?}"
    );
}

#[test]
fn running_init_twice_is_idempotent_with_no_second_run_changes() {
    // Arrange
    let home = scratch_home("idempotent");
    let claude_home = claude_home_of(&home);
    let settings_path = claude_home.join("settings.json");
    let base_path = claude_home.join(".settings.base.json");
    let rc_path = home.join(".bashrc");
    let statusline_dest = claude_home.join("statusline.sh");

    // Act: first run wires everything.
    let first = run(&base_paths(&home, Some(ShellKind::Bash)));
    assert!(first.ok(), "first run should succeed");
    let after_first = (
        fs::read(&settings_path).unwrap(),
        fs::read(&base_path).unwrap(),
        fs::read(&rc_path).unwrap(),
        fs::read(&statusline_dest).unwrap(),
    );

    // Act: second run against the exact same paths.
    let second = run(&base_paths(&home, Some(ShellKind::Bash)));

    // Assert: nothing is reported as a change the second time.
    assert!(second.ok());
    for step in &second.steps {
        // `system-prompt`, `memory-migrate` and `memory-root-migrate` stay
        // `Skipped` on both runs: none has anything to act on in this fixture.
        let expected = if step.name == "system-prompt"
            || step.name == "memory-migrate"
            || step.name == "memory-root-migrate"
        {
            StepStatus::Skipped
        } else {
            StepStatus::AlreadyCorrect
        };
        assert_eq!(
            step.status, expected,
            "expected '{}' to report no change on a second run: {}",
            step.name, step.detail
        );
    }

    // Assert: every file this run touches is byte-identical to after the
    // first run. This is what actually catches a spurious second-run
    // rewrite; the status check above only catches a wrong REPORT.
    let after_second = (
        fs::read(&settings_path).unwrap(),
        fs::read(&base_path).unwrap(),
        fs::read(&rc_path).unwrap(),
        fs::read(&statusline_dest).unwrap(),
    );
    assert_eq!(
        after_first, after_second,
        "a second run must not rewrite any file"
    );
}

#[test]
fn hand_added_hook_entry_survives_composed_init() {
    // Arrange: a `settings.json` with a hook entry the template does not
    // know about, alongside the four guards a real unwired install ships.
    let home = scratch_home("hand-added");
    let claude_home = claude_home_of(&home);
    let hand_added_command = "~/.claude/hooks/my-custom-guard.sh";
    write_json(
        &claude_home.join("settings.json"),
        &json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [
                            {"type": "command", "command": hand_added_command}
                        ]
                    }
                ]
            }
        }),
    );

    // Act
    let outcome = run(&base_paths(&home, Some(ShellKind::Bash)));

    // Assert
    assert!(
        outcome.ok(),
        "{:?}",
        outcome
            .steps
            .iter()
            .map(StepReport::render)
            .collect::<Vec<_>>()
    );
    let settings = read_json(&claude_home.join("settings.json"));
    let commands = all_hook_commands(&settings);
    assert!(
        commands.contains(&hand_added_command.to_string()),
        "hand-added entry was clobbered: {commands:?}"
    );
    // The composed run still wired the ported hooks alongside it.
    assert!(commands.contains(&"playbook hook session-init".to_string()));
}

#[test]
fn malformed_settings_json_fails_without_panicking_or_writing() {
    // Arrange
    let home = scratch_home("malformed");
    let claude_home = claude_home_of(&home);
    let settings_path = claude_home.join("settings.json");
    fs::create_dir_all(&claude_home).unwrap();
    let malformed = "{not valid json";
    fs::write(&settings_path, malformed).unwrap();

    // Act
    let outcome = run(&base_paths(&home, Some(ShellKind::Bash)));

    // Assert: the run reports failure, with a useful message, not a panic
    // (reaching this line at all proves that), and the broken file is left
    // exactly as it was rather than being partially rewritten.
    assert!(!outcome.ok());
    let settings_step = find_step(&outcome, "settings");
    assert_eq!(settings_step.status, StepStatus::Failed);
    assert!(!settings_step.detail.is_empty());
    let hooks_step = find_step(&outcome, "hooks");
    assert_eq!(hooks_step.status, StepStatus::Failed);
    assert!(
        hooks_step.detail.contains("not valid JSON"),
        "{}",
        hooks_step.detail
    );
    assert_eq!(fs::read_to_string(&settings_path).unwrap(), malformed);
}

#[test]
fn missing_self_root_skips_template_dependent_steps() {
    // Arrange: `CLAUDE_PLUGIN_ROOT` unresolved, modelled as `self_root: None`.
    let home = scratch_home("no-root");
    let paths = InitPaths {
        self_root: None,
        claude_home: claude_home_of(&home),
        home: home.clone(),
        shell_kind: Some(ShellKind::Bash),
        system_prompt: false,
        aliases: true,
    };

    // Act
    let outcome = run(&paths);

    // Assert: a missing template is not a failure, it is a clean skip, and
    // `hooks` still runs since `wire::wire` needs no template at all.
    assert!(outcome.ok());
    assert_eq!(find_step(&outcome, "settings").status, StepStatus::Skipped);
    assert_eq!(find_step(&outcome, "shim").status, StepStatus::Skipped);
    assert_eq!(
        find_step(&outcome, "statusline").status,
        StepStatus::Skipped
    );
    assert_eq!(find_step(&outcome, "hooks").status, StepStatus::Wired);
    assert!(!home.join(".bashrc").is_file());
}

#[test]
fn unrecognised_shell_skips_shim_only() {
    // Arrange: `$SHELL` names neither bash nor zsh.
    let home = scratch_home("weird-shell");
    let paths = base_paths(&home, None);

    // Act
    let outcome = run(&paths);

    // Assert
    assert!(outcome.ok());
    let shim_step = find_step(&outcome, "shim");
    assert_eq!(shim_step.status, StepStatus::Skipped);
    assert!(shim_step.detail.contains("$SHELL"), "{}", shim_step.detail);
    assert_eq!(find_step(&outcome, "settings").status, StepStatus::Wired);
    assert_eq!(find_step(&outcome, "hooks").status, StepStatus::Wired);
    assert!(!home.join(".bashrc").is_file());
    assert!(!home.join(".zshrc").is_file());
}

/// `aliases: false` (the default, absent `--aliases`) must skip the `shim`
/// step entirely, the same all-or-nothing gate `setup-local.sh`'s own Step 4
/// uses, so a caller like `setup-local.sh` can call `playbook init` for
/// guards and settings without silently installing a shell launcher the
/// user did not opt into this run.
#[test]
fn aliases_false_skips_shim_entirely() {
    // Arrange
    let home = scratch_home("no-aliases");
    let paths = InitPaths {
        self_root: Some(self_root()),
        claude_home: claude_home_of(&home),
        home: home.clone(),
        shell_kind: Some(ShellKind::Bash),
        system_prompt: false,
        aliases: false,
    };

    // Act
    let outcome = run(&paths);

    // Assert
    assert!(outcome.ok());
    let shim_step = find_step(&outcome, "shim");
    assert_eq!(shim_step.status, StepStatus::Skipped);
    assert!(
        shim_step.detail.contains("--aliases"),
        "{}",
        shim_step.detail
    );
    assert!(!home.join(".bashrc").is_file());
    assert!(!home.join(".zshrc").is_file());
    // The other steps still ran; only `shim` is gated by `aliases`.
    assert_eq!(find_step(&outcome, "settings").status, StepStatus::Wired);
    assert_eq!(find_step(&outcome, "hooks").status, StepStatus::Wired);
}

/// Spawns the real compiled binary rather than calling `run` directly: the
/// non-zero exit code on failure is `main.rs`'s contract, not `init::run`'s,
/// so only a real process boundary proves it.
#[test]
fn binary_malformed_settings_json_exits_non_zero() {
    // Arrange
    let home = scratch_home("binary-malformed");
    let claude_home = claude_home_of(&home);
    fs::create_dir_all(&claude_home).unwrap();
    fs::write(claude_home.join("settings.json"), "{not valid json").unwrap();

    // Act
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .arg("init")
        .env("HOME", &home)
        .env("CLAUDE_PLUGIN_ROOT", self_root())
        .env("SHELL", "/bin/bash")
        .output()
        .expect("playbook binary should spawn");

    // Assert
    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("step(s) failed"), "{stderr}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("FAILED"), "{stdout}");
}

/// The other half of the "fully wired" and "idempotent" library-level tests,
/// proven through the real binary: a clean scratch `$HOME` wires everything
/// on the first `playbook init` and exits 0 unchanged on the second.
#[test]
fn binary_clean_init_exits_zero_and_is_idempotent() {
    // Arrange
    let home = scratch_home("binary-clean");
    let run_init = || {
        Command::new(env!("CARGO_BIN_EXE_playbook"))
            .arg("init")
            .env("HOME", &home)
            .env("CLAUDE_PLUGIN_ROOT", self_root())
            .env("SHELL", "/bin/bash")
            .output()
            .expect("playbook binary should spawn")
    };

    // Act
    let first = run_init();
    let second = run_init();

    // Assert
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let first_out = String::from_utf8_lossy(&first.stdout);
    let second_out = String::from_utf8_lossy(&second.stdout);
    assert!(
        first_out.lines().all(|l| !l.contains(": ok -")),
        "first run should report only changes: {first_out}"
    );
    assert!(
        second_out.lines().all(|l| !l.contains(": wired -")),
        "second run should report no changes: {second_out}"
    );
}

/// The `memory-migrate` step must appear in `run()`'s reported steps.
#[test]
fn memory_migrate_step_appears_in_reported_steps() {
    // Arrange
    let home = scratch_home("memory-migrate-presence");
    let paths = base_paths(&home, Some(ShellKind::Bash));

    // Act
    let outcome = run(&paths);

    // Assert
    find_step(&outcome, "memory-migrate");
}
