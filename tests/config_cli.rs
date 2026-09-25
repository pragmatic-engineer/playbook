// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `playbook config get/set/list`, spawning the real
//! compiled binary so the CLI-layer parsing and exit codes (not just the
//! `config::resolve`/`config::write::set` library calls) are proven. Every
//! test uses a scratch directory standing in for `$HOME`, and either a real
//! scratch git repo with a configured `origin` remote or a plain non-git
//! scratch directory, matching whichever the scenario needs; none touch the
//! developer's real `~/.config/playbook` or `~/.claude`.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory under the OS temp dir, unique per call so
/// parallel tests never collide.
fn scratch_dir(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-config-cli-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch dir should be creatable");
    dir
}

fn git(repo: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .expect("git command should spawn")
}

fn git_ok(repo: &Path, args: &[&str]) {
    let out = git(repo, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A real git repo with a configured `origin` remote, so `repo_slug()`
/// resolves to `test-owner/test-repo`. No commit is needed: `repo_slug`
/// only shells out to `git remote get-url origin`.
fn seeded_repo(tag: &str) -> PathBuf {
    let dir = scratch_dir(tag);
    git_ok(&dir, &["init", "-q", "-b", "main"]);
    git_ok(
        &dir,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/test-owner/test-repo.git",
        ],
    );
    dir
}

/// Run the real compiled binary with `cwd` and `$HOME` pinned to scratch
/// locations, so it never resolves against the developer's real config.
fn run_playbook(cwd: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .output()
        .expect("playbook binary should spawn")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn global_config_path(home: &Path) -> PathBuf {
    home.join(".config").join("playbook").join("config.json")
}

fn org_config_path(home: &Path) -> PathBuf {
    home.join(".config")
        .join("playbook")
        .join("orgs")
        .join("test-owner")
        .join("config.json")
}

fn repo_config_path(home: &Path) -> PathBuf {
    home.join(".config")
        .join("playbook")
        .join("repos")
        .join("test-owner")
        .join("test-repo")
        .join(".config")
        .join("config.json")
}

#[test]
fn get_with_nothing_configured_prints_default_labeled_as_default() {
    // Arrange
    let repo = seeded_repo("get-default");
    let home = scratch_dir("get-default-home");

    // Act
    let out = run_playbook(&repo, &home, &["config", "get", "autoReview.enabled"]);

    // Assert
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let stdout = stdout_of(&out);
    assert!(stdout.contains("autoReview.enabled"), "{stdout}");
    assert!(stdout.contains("true"), "{stdout}");
    assert!(stdout.contains("(source: default)"), "{stdout}");
}

#[test]
fn set_without_flags_writes_repo_tier_and_get_reflects_it() {
    // Arrange: seed the repo tier via a real `set` call before observing it.
    let repo = seeded_repo("set-repo-tier");
    let home = scratch_dir("set-repo-tier-home");
    let seed = run_playbook(
        &repo,
        &home,
        &["config", "set", "autoReview.enabled", "false"],
    );
    assert!(seed.status.success(), "stderr: {}", stderr_of(&seed));

    // Act
    let out = run_playbook(&repo, &home, &["config", "get", "autoReview.enabled"]);

    // Assert
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let stdout = stdout_of(&out);
    assert!(stdout.contains("false"), "{stdout}");
    assert!(stdout.contains("(source: repo)"), "{stdout}");
}

#[test]
fn set_with_global_flag_writes_the_global_tier_file() {
    // Arrange
    let repo = seeded_repo("set-global-tier");
    let home = scratch_dir("set-global-tier-home");

    // Act
    let out = run_playbook(
        &repo,
        &home,
        &["config", "set", "autoReview.type", "quick", "--global"],
    );

    // Assert
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let raw = fs::read_to_string(global_config_path(&home))
        .expect("global config file should have been written");
    let parsed: Value =
        serde_json::from_str(&raw).expect("global config file should be valid json");
    assert_eq!(parsed["autoReview"]["type"], Value::String("quick".into()));
    assert!(
        !org_config_path(&home).exists(),
        "org tier should not have been written"
    );
    assert!(
        !repo_config_path(&home).exists(),
        "repo tier should not have been written"
    );
}

#[test]
fn get_prints_a_string_value_unquoted_not_as_raw_json() {
    // Arrange: prose consumers (create-pull-request.md's Step 9, doctor.md's
    // effective-config line) match this output as a bare `deep`/`quick`
    // token, not a JSON-quoted `"deep"`.
    let repo = seeded_repo("get-string-unquoted");
    let home = scratch_dir("get-string-unquoted-home");

    // Act
    let out = run_playbook(&repo, &home, &["config", "get", "autoReview.type"]);

    // Assert
    let stdout = stdout_of(&out);
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    assert!(
        stdout.contains("autoReview.type: deep (source: default)"),
        "{stdout}"
    );
    assert!(!stdout.contains("\"deep\""), "{stdout}");
}

#[test]
fn set_with_both_org_and_global_flags_exits_non_zero_and_writes_nothing() {
    // Arrange
    let repo = seeded_repo("set-org-and-global");
    let home = scratch_dir("set-org-and-global-home");

    // Act
    let out = run_playbook(
        &repo,
        &home,
        &[
            "config",
            "set",
            "autoReview.enabled",
            "false",
            "--org",
            "--global",
        ],
    );

    // Assert
    assert!(!out.status.success());
    assert!(!stderr_of(&out).is_empty());
    assert!(!global_config_path(&home).exists());
    assert!(!org_config_path(&home).exists());
}

#[test]
fn get_with_a_typo_d_key_exits_non_zero_naming_the_valid_keys() {
    // Arrange
    let repo = seeded_repo("get-typo-key");
    let home = scratch_dir("get-typo-key-home");

    // Act
    let out = run_playbook(&repo, &home, &["config", "get", "autoReiw.enabled"]);

    // Assert
    assert!(!out.status.success());
    let stderr = stderr_of(&out);
    assert!(stderr.contains("unknown config key"), "{stderr}");
    assert!(stderr.contains("autoReview.enabled"), "{stderr}");
    assert!(stderr.contains("autoReview.type"), "{stderr}");
}

#[test]
fn set_with_an_unparseable_bool_value_exits_non_zero_and_writes_nothing() {
    // Arrange
    let repo = seeded_repo("set-notabool");
    let home = scratch_dir("set-notabool-home");

    // Act
    let out = run_playbook(
        &repo,
        &home,
        &["config", "set", "autoReview.enabled", "notabool"],
    );

    // Assert
    assert!(!out.status.success());
    assert!(!stderr_of(&out).is_empty());
    assert!(
        !repo_config_path(&home).exists(),
        "repo tier should not have been written"
    );
}

#[test]
fn set_with_an_out_of_enum_value_exits_non_zero_and_writes_nothing() {
    // Arrange
    let repo = seeded_repo("set-out-of-enum");
    let home = scratch_dir("set-out-of-enum-home");

    // Act
    let out = run_playbook(
        &repo,
        &home,
        &["config", "set", "autoReview.type", "medium"],
    );

    // Assert
    assert!(!out.status.success());
    assert!(!stderr_of(&out).is_empty());
    assert!(
        !repo_config_path(&home).exists(),
        "repo tier should not have been written"
    );
}

#[test]
fn list_shows_both_known_keys_with_value_and_source_tier() {
    // Arrange
    let repo = seeded_repo("list-keys");
    let home = scratch_dir("list-keys-home");

    // Act
    let out = run_playbook(&repo, &home, &["config", "list"]);

    // Assert
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    let stdout = stdout_of(&out);
    assert!(stdout.contains("autoReview.enabled"), "{stdout}");
    assert!(stdout.contains("autoReview.type"), "{stdout}");
    assert!(stdout.contains("(source: default)"), "{stdout}");
}

#[test]
fn get_outside_any_git_repo_still_works_falling_through_to_default() {
    // Arrange: a scratch dir with no `.git` at all.
    let cwd = scratch_dir("get-no-git");
    let home = scratch_dir("get-no-git-home");

    // Act
    let out = run_playbook(&cwd, &home, &["config", "get", "autoReview.enabled"]);

    // Assert
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    assert!(!stderr_of(&out).contains("panicked"));
    let stdout = stdout_of(&out);
    assert!(stdout.contains("(source: default)"), "{stdout}");
}

#[test]
fn list_outside_any_git_repo_still_works_falling_through_to_default() {
    // Arrange: a scratch dir with no `.git` at all.
    let cwd = scratch_dir("list-no-git");
    let home = scratch_dir("list-no-git-home");

    // Act
    let out = run_playbook(&cwd, &home, &["config", "list"]);

    // Assert
    assert!(out.status.success(), "stderr: {}", stderr_of(&out));
    assert!(!stderr_of(&out).contains("panicked"));
}

#[test]
fn set_outside_any_git_repo_with_default_repo_tier_fails_clearly_not_a_crash() {
    // Arrange: a scratch dir with no `.git` at all, so the repo tier `set`
    // defaults to has no repo context to build a path from.
    let cwd = scratch_dir("set-no-git");
    let home = scratch_dir("set-no-git-home");

    // Act
    let out = run_playbook(
        &cwd,
        &home,
        &["config", "set", "autoReview.enabled", "false"],
    );

    // Assert
    assert!(!out.status.success());
    let stderr = stderr_of(&out);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(!stderr.is_empty());
}
