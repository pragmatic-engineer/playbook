// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Upgrade simulation (ADR 0034): a home laid out the way playbook 0.25 left
//! it meets this binary. The session hook heals playbook's own files and
//! names what needs the user, `doctor --fix` lists the same, and only `init`
//! changes settings.json and the shell rc file.

use playbook::init::run::{run, InitPaths};
use playbook::init::shim::ShellKind;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const DEAD_SESSION: &str = "bash ~/.claude/hooks/session-init.sh";
const DEAD_GUARD: &str = "~/.claude/hooks/pre-tool-guard.sh";
const MINE: &str = "bash ~/.claude/hooks/mine.sh";

struct V025Home {
    home: PathBuf,
}

impl V025Home {
    fn new() -> V025Home {
        let home = std::env::temp_dir().join(format!(
            "playbook-upgrade-v025-{}",
            playbook::testing::run_id()
        ));
        let _ = fs::remove_dir_all(&home);
        let claude = home.join(".claude");
        let statusline = playbook::init::statusline::playbook_statusline_path(&home);
        write(&claude.join("hooks/mine.sh"), "#!/bin/sh\n");
        write(
            &claude.join("settings.json"),
            &format!(
                "{:#}\n",
                json!({
                    "model": "opus",
                    "statusLine": {"type": "command", "command": format!("bash {}", statusline.display())},
                    "hooks": {
                        "SessionStart": [{"hooks": [{"type": "command", "command": DEAD_SESSION}]}],
                        "PreToolUse": [{"matcher": "Bash", "hooks": [
                            {"type": "command", "command": DEAD_GUARD},
                            {"type": "command", "command": MINE}
                        ]}]
                    }
                })
            ),
        );
        write(
            &home.join(".zshrc"),
            "export EDITOR=vim\nsource ~/.claude/shell/zsh/cc.zsh\n",
        );
        write(&home.join("notes.md"), "my own notes\n");
        let cache = claude.join("plugins/cache/pragmatic-engineer/playbook");
        for version in ["0.24.0", "0.25.0"] {
            write(
                &cache.join(version).join(".claude-plugin/plugin.json"),
                &format!(r#"{{"name":"playbook","version":"{version}"}}"#),
            );
            fs::create_dir_all(cache.join(version).join("skills")).unwrap();
        }
        write(&cache.join("0.24.0/.orphaned_at"), "1");
        write(
            &claude.join("plugins/installed_plugins.json"),
            &format!(
                r#"{{"version":2,"plugins":{{"playbook@pragmatic-engineer":[{{"scope":"user","installPath":"{}","version":"0.25.0"}}],"playbook-hooks@pragmatic-engineer":[{{"scope":"user"}}]}}}}"#,
                cache.join("0.25.0").display()
            ),
        );
        let root = playbook::common::paths::playbook_root_from(&home);
        write(
            &root.join("hooks/lib/config-hash.sh"),
            "config_hash() { :; }\n",
        );
        V025Home { home }
    }

    fn settings(&self) -> PathBuf {
        self.home.join(".claude/settings.json")
    }

    fn user_files(&self) -> Vec<Vec<u8>> {
        [
            self.settings(),
            self.home.join(".zshrc"),
            self.home.join("notes.md"),
        ]
        .iter()
        .map(|p| fs::read(p).unwrap())
        .collect()
    }

    fn playbook(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(args)
            .current_dir(&self.home)
            .env("HOME", &self.home)
            .env("SHELL", "/bin/zsh")
            .env_remove("CLAUDE_PLUGIN_ROOT")
            .env_remove("PLAYBOOK_PLUGIN_DIR")
            .env_remove("HOOK_INPUT")
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn session_message(&self) -> String {
        let out = self.playbook(&["hook", "session-init"], r#"{"source":"startup"}"#);
        assert_eq!(out.status.code(), Some(0));
        let value: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        value["systemMessage"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    fn init(&self) {
        let outcome = run(&InitPaths {
            plugin_root: None,
            self_root: Some(PathBuf::from(env!("PLAYBOOK_REPO_ROOT"))),
            claude_home: self.home.join(".claude"),
            home: self.home.clone(),
            shell_kind: Some(ShellKind::Zsh),
            system_prompt: false,
            aliases: true,
            hooks: true,
            statusline: true,
            security: false,
            telemetry: false,
            claude_override: false,
            telemetry_replace_off: false,
            only: false,
            short_name: false,
            path_setup: None,
            repo: None,
        });
        assert!(outcome.ok());
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn hook_commands(settings: &Path) -> Vec<String> {
    let value: Value = serde_json::from_str(&fs::read_to_string(settings).unwrap()).unwrap();
    let mut out = Vec::new();
    for groups in value["hooks"].as_object().unwrap().values() {
        for group in groups.as_array().unwrap() {
            for entry in group["hooks"].as_array().unwrap() {
                out.push(entry["command"].as_str().unwrap().to_string());
            }
        }
    }
    out
}

#[test]
fn a_v025_home_heals_its_own_files_and_leaves_the_rest_to_init() {
    // Arrange
    let h = V025Home::new();
    let user_files = h.user_files();
    let config_hash =
        playbook::common::paths::playbook_root_from(&h.home).join("hooks/lib/config-hash.sh");

    // Act: the first session after the binary upgrade, then a second one.
    let first = h.session_message();
    let second = h.session_message();
    let fix = h.playbook(&["doctor", "--fix"], "");

    // Assert: playbook's own file healed, user files untouched, one command each.
    assert!(!config_hash.exists());
    assert_eq!(h.user_files(), user_files);
    assert!(
        first.contains("of its own files after the upgrade"),
        "{first}"
    );
    for message in [&first, &second] {
        for part in [
            "3 change(s) to your settings or shell files need `playbook init` and `playbook init --hooks` and `playbook init --statusline`",
            "2 item(s) need you: run `playbook doctor --fix`",
        ] {
            assert!(message.contains(part), "{message}");
        }
    }
    assert!(!second.contains("updated"), "{second}");
    let report = String::from_utf8_lossy(&fix.stdout);
    for line in [
        "claude plugin uninstall playbook-hooks@pragmatic-engineer",
        "claude plugin update playbook@pragmatic-engineer",
        "0014-orphaned-plugin-caches: 1 old plugin folder(s)",
        "(run `playbook init --hooks`)",
    ] {
        assert!(report.contains(line), "{report}");
    }
    assert_eq!(h.user_files(), user_files);

    // Act: the user runs init.
    h.init();
    let after_init = fs::read(h.settings()).unwrap();
    h.init();

    // Assert: dead entries gone, the user's own hook kept, settings backed up,
    // the old launcher line rewritten, and a second init changes nothing.
    let commands = hook_commands(&h.settings());
    assert!(commands.iter().any(|c| c == MINE), "{commands:?}");
    assert!(!commands
        .iter()
        .any(|c| c == DEAD_SESSION || c == DEAD_GUARD));
    let backups: Vec<_> = fs::read_dir(h.home.join(".claude"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("settings.json.bak.")
        })
        .collect();
    assert!(!backups.is_empty());
    let zshrc = fs::read_to_string(h.home.join(".zshrc")).unwrap();
    assert!(zshrc.starts_with("export EDITOR=vim\n"), "{zshrc}");
    assert!(!zshrc.contains("cc.zsh"), "{zshrc}");
    assert_eq!(
        fs::read(h.home.join("notes.md")).unwrap(),
        b"my own notes\n"
    );
    assert_eq!(fs::read(h.settings()).unwrap(), after_init);
    let settled = h.session_message();
    assert!(!settled.contains("playbook init"), "{settled}");
    assert!(settled.contains("2 item(s) need you"), "{settled}");
}
