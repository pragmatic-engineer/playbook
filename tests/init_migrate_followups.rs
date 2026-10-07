// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Follow-ups to ADR 0016: edited files are kept, the gate move runs from
//! init inside a repo, and `playbook doctor pending-migrations` reports edits.

use playbook::init::run::{run, InitPaths, StepStatus};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pb-followups-{}-{tag}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A shipped tree with a prompt and one skill, in a version-named dir.
fn plugin_root(base: &Path, prompt: &str, skill: &str) -> PathBuf {
    let root = base.join("plugin").join("9.9.9");
    fs::create_dir_all(root.join("prompts")).unwrap();
    fs::create_dir_all(root.join("skills/demo")).unwrap();
    fs::write(root.join("prompts/SYSTEM_PROMPT.md"), prompt).unwrap();
    fs::write(root.join("skills/demo/SKILL.md"), skill).unwrap();
    root
}

fn paths(home: &Path, root: &Path, repo: Option<(PathBuf, PathBuf)>) -> InitPaths {
    InitPaths {
        self_root: Some(root.to_path_buf()),
        claude_home: home.join(".claude"),
        home: home.to_path_buf(),
        shell_kind: None,
        system_prompt: true,
        aliases: false,
        repo,
    }
}

fn prompt_dest(home: &Path) -> PathBuf {
    home.join(".config/playbook/prompts/SYSTEM_PROMPT.md")
}

#[test]
fn an_edited_prompt_is_left_in_place_and_an_unedited_one_still_updates() {
    let base = scratch("prompt");
    let home = base.join("home");
    let root = plugin_root(&base, "v1", "s");
    run(&paths(&home, &root, None));

    fs::write(root.join("prompts/SYSTEM_PROMPT.md"), "v2").unwrap();
    run(&paths(&home, &root, None));
    assert_eq!(fs::read_to_string(prompt_dest(&home)).unwrap(), "v2");

    fs::write(prompt_dest(&home), "mine").unwrap();
    fs::write(root.join("prompts/SYSTEM_PROMPT.md"), "v3").unwrap();
    let outcome = run(&paths(&home, &root, None));
    assert_eq!(fs::read_to_string(prompt_dest(&home)).unwrap(), "mine");
    let step = outcome
        .steps
        .iter()
        .find(|s| s.name == "system-prompt")
        .unwrap();
    assert_eq!(step.status, StepStatus::Skipped);
    assert!(outcome.warnings.iter().any(|w| w.contains("delete it")));

    fs::remove_file(prompt_dest(&home)).unwrap();
    run(&paths(&home, &root, None));
    assert_eq!(fs::read_to_string(prompt_dest(&home)).unwrap(), "v3");
}

#[test]
fn a_modified_skill_is_reported_and_not_rewritten() {
    let base = scratch("skill");
    let home = base.join("home");
    let root = plugin_root(&base, "p", "shipped");
    let first = run(&paths(&home, &root, None));
    assert!(first.warnings.is_empty());

    fs::write(root.join("skills/demo/SKILL.md"), "edited").unwrap();
    let second = run(&paths(&home, &root, None));
    assert!(second.warnings.iter().any(|w| w.contains("demo")));
    assert_eq!(
        fs::read_to_string(root.join("skills/demo/SKILL.md")).unwrap(),
        "edited"
    );
}

#[test]
fn an_edited_statusline_is_kept_and_reported() {
    let base = scratch("statusline");
    let home = base.join("home");
    let root = plugin_root(&base, "p", "s");
    fs::write(root.join("statusline.sh"), "v1").unwrap();
    run(&paths(&home, &root, None));
    let dest = home.join(".config/playbook/statusline.sh");
    fs::write(&dest, "mine").unwrap();
    fs::write(root.join("statusline.sh"), "v2").unwrap();
    let outcome = run(&paths(&home, &root, None));
    assert_eq!(fs::read_to_string(&dest).unwrap(), "mine");
    assert!(outcome.warnings.iter().any(|w| w.contains("statusline")));
}

fn legacy_repo(base: &Path) -> (PathBuf, PathBuf) {
    let repo = base.join("repo");
    fs::create_dir_all(repo.join(".claude/plans")).unwrap();
    fs::write(repo.join(".claude/plans/p.md"), "plan").unwrap();
    (repo, base.join("dest"))
}

#[test]
fn the_gate_move_runs_from_init_inside_a_repo_and_not_outside() {
    let base = scratch("gate");
    let home = base.join("home");
    let root = plugin_root(&base, "p", "s");
    let (repo, dest) = legacy_repo(&base);

    run(&paths(&home, &root, None));
    assert!(repo.join(".claude/plans/p.md").exists());
    assert!(!dest.join("plans/p.md").exists());

    run(&paths(&home, &root, Some((repo.clone(), dest.clone()))));
    assert!(dest.join("plans/p.md").exists());
    assert!(!repo.join(".claude/plans/p.md").exists());
}

fn pending(home: &Path, root: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["doctor", "pending-migrations"])
        .env("HOME", home)
        .env("CLAUDE_PLUGIN_ROOT", root)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn doctor_prints_pending_items_only_when_something_is_pending() {
    let base = scratch("doctor");
    let home = base.join("home");
    let root = plugin_root(&base, "p", "shipped");
    run(&paths(&home, &root, None));
    assert_eq!(pending(&home, &root), "");

    fs::write(root.join("skills/demo/SKILL.md"), "edited").unwrap();
    let out = pending(&home, &root);
    assert!(out.contains("0004-skills-edited") && out.contains("demo"));
}
