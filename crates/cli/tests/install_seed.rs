// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `install.sh --yes` seeds `settings.json` from the shipped template through
//! the real `playbook init`: a fresh install, and a user hook that survives.

#[path = "support/shell_fixture.rs"]
mod fx;

use fx::{bin_dir_with_playbook, repo_root, stderr, stdout, Work};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Run the real installer with the real `playbook` already in the bin folder.
/// Returns (home, log) and keeps the scratch dir alive through `work`.
fn install(work: &Work, preseed: Option<&str>) -> (PathBuf, String) {
    let home = work.dir("home");
    let bin = bin_dir_with_playbook(work);
    if let Some(body) = preseed {
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::write(home.join(".claude/settings.json"), body).unwrap();
    }
    let out = Command::new("bash")
        .arg(repo_root().join("install.sh"))
        .arg("--yes")
        .env_remove("SHELL")
        .env_remove("CLAUDE_PLUGIN_ROOT")
        .env_remove("CLAUDE_CODE_ENABLE_TELEMETRY")
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .env("PLAYBOOK_SKIP_DOWNLOAD", "1")
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
    let (home, _) = install(&work, None);
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
    // The installer's init --yes also turns telemetry on, and writes nothing
    // else into env.
    let env = settings["env"].as_object().expect("telemetry env block");
    assert_eq!(env["CLAUDE_CODE_ENABLE_TELEMETRY"], "1");
    assert!(
        env.keys()
            .all(|k| k == "CLAUDE_CODE_ENABLE_TELEMETRY" || k.starts_with("OTEL_")),
        "{settings}"
    );
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
    let (home, _) = install(&work, Some(user));
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
