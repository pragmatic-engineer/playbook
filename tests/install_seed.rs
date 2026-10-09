// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `install.sh` seeds `settings.json` from the shipped template through the
//! real `playbook init`: a fresh install, a user hook that survives, and a
//! `settings.json` shipped in the source tree that must never be copied.

#[path = "support/shell_fixture.rs"]
mod fx;

use fx::{bin_dir_with_playbook, repo_root, stderr, stdout, Work};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Run the real installer with an otherwise empty source holding the template.
/// Returns (home, log) and keeps the scratch dir alive through `work`.
fn install(
    work: &Work,
    preseed: Option<&str>,
    shipped_settings: Option<&str>,
) -> (PathBuf, String) {
    let src = work.dir("src");
    let home = work.dir("home");
    let bin = bin_dir_with_playbook(work);
    fs::copy(
        repo_root().join("settings.shared.json"),
        src.join("settings.shared.json"),
    )
    .unwrap();
    if let Some(body) = shipped_settings {
        fs::write(src.join("settings.json"), body).unwrap();
    }
    if let Some(body) = preseed {
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::write(home.join(".claude/settings.json"), body).unwrap();
    }
    let out = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .args(["--no-setup", "--yes"])
        .env_remove("SHELL")
        .env("PLAYBOOK_SRC", &src)
        .env("PLAYBOOK_BIN_DIR", &bin)
        .env("CLAUDE_HOME", home.join(".claude"))
        .env("HOME", &home)
        .output()
        .expect("bash spawns");
    let log = format!("{}{}", stdout(&out), stderr(&out));
    assert!(out.status.success(), "install failed: {log}");
    (home, log)
}

fn settings_of(home: &Path) -> Value {
    let text = fs::read_to_string(home.join(".claude/settings.json")).expect("settings.json");
    serde_json::from_str(&text).expect("settings.json is valid JSON")
}

/// How many hook entries run `playbook hook ...`.
fn ported_hooks(settings: &Value) -> usize {
    let mut n = 0;
    let mut stack = vec![settings.get("hooks").cloned().unwrap_or(Value::Null)];
    while let Some(v) = stack.pop() {
        match v {
            Value::Object(map) => {
                if let Some(cmd) = map.get("command").and_then(Value::as_str) {
                    if cmd.starts_with("playbook hook ") {
                        n += 1;
                    }
                }
                stack.extend(map.into_values());
            }
            Value::Array(items) => stack.extend(items),
            _ => {}
        }
    }
    n
}

#[test]
fn a_fresh_install_seeds_settings_and_wires_the_ported_hooks() {
    let work = Work::new("seed-fresh");
    let (home, _) = install(&work, None, None);
    let settings = settings_of(&home);

    let template: Value = serde_json::from_str(
        &fs::read_to_string(repo_root().join("settings.shared.json")).unwrap(),
    )
    .unwrap();
    for (key, want) in template.as_object().unwrap() {
        if key == "hooks" || key == "permissions" || key == "env" {
            continue;
        }
        assert_eq!(
            settings.get(key),
            Some(want),
            "key {key} differs from the template"
        );
    }
    // The security defaults are opt-in: a default install leaves them out.
    assert!(settings.get("permissions").is_none(), "{settings}");
    assert!(
        settings["env"].get("DISABLE_AUTOUPDATER").is_none(),
        "{settings}"
    );
    assert_eq!(settings["env"]["DO_NOT_TRACK"], "1");
    assert!(
        ported_hooks(&settings) >= 18,
        "got {}",
        ported_hooks(&settings)
    );
}

#[test]
fn a_user_authored_hook_survives_with_the_ported_hooks_beside_it() {
    let work = Work::new("seed-preserve");
    let user = r#"{"hooks":{"Notification":[{"hooks":[{"type":"command","command":"/opt/my-custom-notify.sh"}]}]}}"#;
    let (home, _) = install(&work, Some(user), None);
    let settings = settings_of(&home);
    assert_eq!(
        settings["hooks"]["Notification"][0]["hooks"][0]["command"],
        "/opt/my-custom-notify.sh"
    );
    assert!(
        ported_hooks(&settings) >= 18,
        "got {}",
        ported_hooks(&settings)
    );
}

#[test]
fn the_copy_loop_skips_a_settings_json_shipped_in_the_source() {
    let work = Work::new("seed-skip");
    let (home, _) = install(&work, None, Some("SHIPPED SENTINEL must never land\n"));
    let raw = fs::read_to_string(home.join(".claude/settings.json")).unwrap();
    assert!(!raw.contains("SHIPPED SENTINEL"), "sentinel landed");
    serde_json::from_str::<Value>(&raw).expect("valid JSON");
}
