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

// Refresh at the touch points: a folder synced once follows the plugin.

const REFRESH_FLAG: &str = "--refresh";
const NEW_VERSION: &str = "2.0.0";
const CLAIM_KEY: &str = "skills-sync/codex/refresh";
const NOTICE_KEY: &str = "notices/skills-sync";
const SESSION_PAYLOAD: &str = r#"{"source":"startup"}"#;
const NO_SPAWN_ENV: &str = "PLAYBOOK_TEST_NO_SPAWN";
const DAY_HOURS: u64 = 25;
const HOUR_SECS: u64 = 3600;
const CI_VARS: [&str; 7] = [
    "CI",
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "BUILDKITE",
    "CIRCLECI",
    "JENKINS_URL",
    "TF_BUILD",
];

/// The fixture at a new version with a changed `fix` skill.
fn publish_new_version(plugin: &Path) {
    fs::write(
        plugin.join(".claude-plugin/plugin.json"),
        format!(r#"{{"name":"playbook","version":"{NEW_VERSION}"}}"#),
    )
    .unwrap();
    fs::write(
        plugin.join("skills/fix/SKILL.md"),
        "---\nname: fix\ndescription: Fix a small bug\n---\nFix it better.\n",
    )
    .unwrap();
}

fn playbook_run(home: &Path, plugin: &Path, args: &[&str]) -> Output {
    playbook_command(home, plugin, args, &[])
        .output()
        .expect("playbook should start")
}

fn playbook_command(home: &Path, plugin: &Path, args: &[&str], env: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_playbook"));
    isolated::isolate(&mut cmd, home);
    cmd.env(PLUGIN_ENV, plugin)
        .env("PATH", "")
        .current_dir(home)
        .args(args);
    for var in CI_VARS {
        cmd.env_remove(var);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd
}

/// A session start with the detached spawn turned off, so the hook's own
/// writes are all a test can see.
fn hook(home: &Path, plugin: &Path) -> Output {
    use std::io::Write;
    if !cfg!(debug_assertions) {
        panic!("the spawn seam exists only in debug builds; run these tests without --release");
    }
    let mut cmd = playbook_command(
        home,
        plugin,
        &["hook", "session-init"],
        &[(NO_SPAWN_ENV, "1")],
    );
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(SESSION_PAYLOAD.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn root(home: &Path) -> PathBuf {
    home.join(".config/playbook")
}

fn state_value(home: &Path, key: &str) -> Option<String> {
    playbook::state::get(&root(home), key).unwrap()
}

fn skill_text(home: &Path) -> String {
    fs::read_to_string(skills_dir(home).join(SKILL_NAME).join("SKILL.md")).unwrap()
}

/// A home synced once at the fixture version, then published at version 2.
fn synced_then_upgraded() -> (PathBuf, PathBuf) {
    let (home, plugin) = codex_home();
    assert!(sync(&home, &plugin, &[]).status.success());
    publish_new_version(&plugin);
    (home, plugin)
}

#[test]
fn migrate_updates_a_synced_skill_after_the_plugin_changes() {
    // Arrange
    let (home, plugin) = synced_then_upgraded();

    // Act
    let out = playbook_run(&home, &plugin, &["migrate"]);

    // Assert
    assert!(
        stdout(&out).contains(&format!("updated {SKILL_NAME}")),
        "{}{}",
        stdout(&out),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(skill_text(&home).contains("Fix it better."));
}

#[test]
fn migrate_dry_run_says_would_update_and_writes_nothing() {
    // Arrange
    let (home, plugin) = synced_then_upgraded();
    let before = skill_text(&home);

    // Act
    let out = playbook_run(&home, &plugin, &["migrate", "--dry-run"]);

    // Assert
    assert!(
        stdout(&out).contains(&format!("would update {SKILL_NAME}")),
        "{}",
        stdout(&out)
    );
    assert_eq!(skill_text(&home), before);
    assert_eq!(state_value(&home, CLAIM_KEY), None);
}

#[test]
fn the_background_refresh_updates_the_folder_and_the_next_session_shows_it_once() {
    // Arrange
    let (home, plugin) = synced_then_upgraded();

    // Act
    let refresh = playbook_run(&home, &plugin, &["skills", "sync", REFRESH_FLAG]);
    let first = hook(&home, &plugin);
    let second = hook(&home, &plugin);

    // Assert
    assert!(refresh.status.success(), "{refresh:?}");
    assert!(stdout(&refresh).is_empty() && refresh.stderr.is_empty());
    assert!(skill_text(&home).contains("Fix it better."));
    assert!(
        stdout(&first).contains("refreshed the Codex skills"),
        "{}",
        stdout(&first)
    );
    assert!(!stdout(&second).contains("Codex skills"));
    assert_eq!(state_value(&home, NOTICE_KEY), None);
}

#[test]
fn the_hook_writes_no_skills_itself() {
    // Arrange
    let (home, plugin) = synced_then_upgraded();
    let before = skill_text(&home);

    // Act
    let out = hook(&home, &plugin);

    // Assert
    assert!(out.status.success(), "{out:?}");
    assert_eq!(skill_text(&home), before);
    assert_eq!(state_value(&home, CLAIM_KEY), None);
}

#[test]
fn an_agent_without_records_is_never_touched() {
    // Arrange
    let (home, plugin) = codex_home();

    // Act
    playbook_run(&home, &plugin, &["migrate"]);
    playbook_run(&home, &plugin, &["migrate", "--dry-run"]);
    playbook_run(&home, &plugin, &["skills", "sync", REFRESH_FLAG]);
    hook(&home, &plugin);

    // Assert
    assert!(!home.join(".agents").exists());
    assert_eq!(state_value(&home, CLAIM_KEY), None);
    assert_eq!(state_value(&home, NOTICE_KEY), None);
}

#[test]
fn two_refreshes_started_together_apply_once() {
    // Arrange
    let (home, plugin) = synced_then_upgraded();
    let start = || {
        playbook_command(&home, &plugin, &["skills", "sync", REFRESH_FLAG], &[])
            .spawn()
            .unwrap()
    };

    // Act
    let (mut a, mut b) = (start(), start());
    let (a, b) = (a.wait().unwrap(), b.wait().unwrap());

    // Assert
    assert!(a.success() && b.success());
    let notice = state_value(&home, NOTICE_KEY).expect("one summary");
    assert!(notice.contains("1 updated"), "{notice}");
    assert!(
        state_value(&home, CLAIM_KEY)
            .is_some_and(|c| c.starts_with(NEW_VERSION) && c.contains(" done ")),
        "{:?}",
        state_value(&home, CLAIM_KEY)
    );
}

#[cfg(unix)]
#[test]
fn a_failed_refresh_waits_a_day_before_it_runs_again() {
    use std::os::unix::fs::PermissionsExt;
    const READ_ONLY: u32 = 0o555;
    const WRITABLE: u32 = 0o755;
    // Arrange
    let (home, plugin) = synced_then_upgraded();
    let skills = skills_dir(&home);
    fs::set_permissions(&skills, fs::Permissions::from_mode(READ_ONLY)).unwrap();
    if fs::create_dir(skills.join("probe")).is_ok() {
        fs::remove_dir(skills.join("probe")).unwrap();
        return; // running as a user the mode does not bind
    }
    let refresh = || playbook_run(&home, &plugin, &["skills", "sync", REFRESH_FLAG]);

    // Act
    refresh();
    let failed = state_value(&home, CLAIM_KEY).expect("a claim");
    refresh();
    let after_second = state_value(&home, CLAIM_KEY);
    fs::set_permissions(&skills, fs::Permissions::from_mode(WRITABLE)).unwrap();
    let stale = format!(
        "{NEW_VERSION} failed {}",
        now_secs() - DAY_HOURS * HOUR_SECS
    );
    playbook::state::set(&root(&home), CLAIM_KEY, &stale).unwrap();
    refresh();

    // Assert
    assert!(
        failed.starts_with(NEW_VERSION) && failed.contains(" failed "),
        "{failed}"
    );
    assert_eq!(after_second.as_deref(), Some(failed.as_str()));
    assert!(skill_text(&home).contains("Fix it better."));
    assert!(state_value(&home, CLAIM_KEY).is_some_and(|c| c.contains(" done ")));
}

#[test]
fn the_hook_starts_nothing_right_after_a_failed_claim() {
    // Arrange
    let (home, plugin) = synced_then_upgraded();
    let claim = format!("{NEW_VERSION} failed {}", now_secs());
    playbook::state::set(&root(&home), CLAIM_KEY, &claim).unwrap();
    // A started child would move the claim to running, so an unchanged
    // claim shows the hook spawned nothing (the spawn seam stays off).

    // Act
    let out = playbook_command(&home, &plugin, &["hook", "session-init"], &[])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1500));

    // Assert
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        state_value(&home, CLAIM_KEY).as_deref(),
        Some(claim.as_str())
    );
    assert!(skill_text(&home).contains("Fix it."));
}

#[test]
fn the_hook_starts_a_detached_refresh_when_a_synced_agent_is_behind() {
    const WAIT: std::time::Duration = std::time::Duration::from_secs(10);
    const POLL: std::time::Duration = std::time::Duration::from_millis(50);
    // Arrange
    let (home, plugin) = synced_then_upgraded();

    // Act
    let out = playbook_command(&home, &plugin, &["hook", "session-init"], &[])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let deadline = std::time::Instant::now() + WAIT;
    while state_value(&home, NOTICE_KEY).is_none() && std::time::Instant::now() < deadline {
        std::thread::sleep(POLL);
    }

    // Assert
    assert!(out.status.success(), "{out:?}");
    assert!(skill_text(&home).contains("Fix it better."));
    assert!(state_value(&home, NOTICE_KEY).is_some());
}
