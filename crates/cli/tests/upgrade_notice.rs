// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The upgrade notice from a user report (ADR 0034, 2026-10-19 amendment):
//! a session start listed one item and named `playbook migrate`, and the
//! same message came back after `playbook migrate` and after a plugin
//! update. Each item is now one bullet with its own command, `migrate`
//! prints the same list and counts it as seen, and a fixed item is gone.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Home {
    home: PathBuf,
}

impl Home {
    fn new(tag: &str) -> Home {
        let home = std::env::temp_dir().join(format!(
            "playbook-upgrade-notice-{tag}-{}",
            playbook::testing::run_id()
        ));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join(".claude/plugins")).unwrap();
        // Settings with nothing open, so only the plugin item is.
        fs::write(
            home.join(".claude/settings.json"),
            r#"{"attribution":{"commit":"","pr":"","sessionUrl":false}}"#,
        )
        .unwrap();
        Home { home }
    }

    /// A cached plugin `version` that needs binary `min`.
    fn plugin(&self, version: &str, min: &str) -> PathBuf {
        let dir = self
            .home
            .join(".claude/plugins/cache/pragmatic-engineer/playbook")
            .join(version);
        fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        fs::create_dir_all(dir.join("skills")).unwrap();
        fs::write(
            dir.join(".claude-plugin/plugin.json"),
            format!(
                r#"{{"name":"playbook","version":"{version}","metadata":{{"minBinaryVersion":"{min}"}}}}"#
            ),
        )
        .unwrap();
        dir
    }

    /// What `claude plugin install` or `update` leaves in installed_plugins.json.
    fn install(&self, dir: &Path, version: &str) {
        fs::write(
            self.home.join(".claude/plugins/installed_plugins.json"),
            format!(
                r#"{{"version":2,"plugins":{{"playbook@pragmatic-engineer":[{{"scope":"user","installPath":"{}","version":"{version}"}}]}}}}"#,
                dir.display()
            ),
        )
        .unwrap();
    }

    fn playbook(&self, args: &[&str], stdin: &str, plugin_root: Option<&Path>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_playbook"));
        command
            .args(args)
            .current_dir(&self.home)
            .env("HOME", &self.home)
            .env("PATH", "/usr/bin:/bin")
            .env_remove("CLAUDE_PLUGIN_ROOT")
            .env_remove("PLAYBOOK_PLUGIN_DIR")
            .env_remove("HOOK_INPUT")
            .env_remove("CI")
            .env_remove("PLAYBOOK_HEADLESS")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(root) = plugin_root {
            command.env("CLAUDE_PLUGIN_ROOT", root);
        }
        let mut child = command.spawn().unwrap();
        use std::io::Write;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn session(&self, plugin_root: Option<&Path>) -> String {
        let out = self.playbook(
            &["hook", "session-init"],
            r#"{"source":"startup"}"#,
            plugin_root,
        );
        assert_eq!(out.status.code(), Some(0));
        let value: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        value["systemMessage"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    fn stdout(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&self.playbook(args, "", None).stdout).into_owned()
    }
}

/// A plugin one minor line behind the binary, or `None` on a `0.x.0`
/// binary with no line before it.
fn older_line() -> Option<String> {
    let mut parts = VERSION.split('.').map(|p| p.parse::<u64>().unwrap());
    let (major, minor) = (parts.next()?, parts.next()?);
    (minor > 0).then(|| format!("{major}.{}.0", minor - 1))
}

#[test]
fn the_reported_notice_lists_the_item_with_its_command_and_does_not_repeat() {
    let Some(old) = older_line() else {
        return;
    };
    // Arrange: a fresh install whose plugin is one line behind.
    let h = Home::new("report");
    let stale = h.plugin(&old, &old);
    h.install(&stale, &old);

    // Act: the session from the report, `playbook migrate`, two more
    // sessions, then a plugin update and one more session.
    let first = h.session(None);
    let migrate = h.stdout(&["migrate"]);
    let status = h.stdout(&["migrate", "--status"]);
    let second = h.session(None);
    let third = h.session(None);
    let current = h.plugin(VERSION, VERSION);
    h.install(&current, VERSION);
    let after_update = h.stdout(&["migrate", "--status"]);
    let fourth = h.session(None);

    // Assert
    assert!(first.contains("1 item needs you:\n  - "), "{first}");
    assert!(
        first.contains("    Fix: claude plugin update playbook@pragmatic-engineer"),
        "{first}"
    );
    assert!(
        !first.contains("item(s) need you: run `playbook migrate`"),
        "{first}"
    );
    let bullet = |text: &str| {
        text.lines()
            .skip_while(|l| !l.starts_with("  - "))
            .take(3)
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(bullet(&migrate), bullet(&first), "one renderer: {migrate}");
    assert_eq!(bullet(&status), bullet(&first), "one renderer: {status}");
    assert!(
        migrate.starts_with(
            "nothing to fix in playbook's own files\nStill left for you, 1 item needs you:"
        ),
        "{migrate}"
    );
    assert_eq!(second, "", "shown less than a day ago");
    assert_eq!(third, "");
    assert!(
        after_update.ends_with("nothing needs you\n"),
        "{after_update}"
    );
    assert_eq!(fourth, "");
}

#[test]
fn a_session_from_a_stale_plugin_folder_says_to_restart() {
    let Some(old) = older_line() else {
        return;
    };
    // Arrange: Claude Code installed the current plugin, and this session
    // still runs from the old cache folder.
    let h = Home::new("stale");
    let stale = h.plugin(&old, &old);
    let current = h.plugin(VERSION, VERSION);
    h.install(&current, VERSION);

    // Act
    let from_stale = h.session(Some(&stale));
    let from_current = Home::new("stale-current");
    let dir = from_current.plugin(VERSION, VERSION);
    from_current.install(&dir, VERSION);
    let fresh = from_current.session(Some(&dir));

    // Assert
    assert!(
        from_stale.contains(&format!("plugin {old} from an old folder")),
        "{from_stale}"
    );
    assert!(
        from_stale.contains("Fix: exit this session and start Claude Code again"),
        "{from_stale}"
    );
    assert!(
        !from_stale.contains("claude plugin update"),
        "the installed plugin is current: {from_stale}"
    );
    assert_eq!(fresh, "", "a fresh install has nothing to say");
}

#[test]
fn a_consent_item_shows_once_then_stays_quiet_for_the_day() {
    // Arrange: the status line still runs the old script, a setup change
    // the user has not made yet.
    let h = Home::new("consent");
    let script = playbook::init::statusline::playbook_statusline_path(&h.home);
    fs::write(
        h.home.join(".claude/settings.json"),
        format!(
            r#"{{"statusLine":{{"type":"command","command":"bash {}"}}}}"#,
            script.display()
        ),
    )
    .unwrap();

    // Act
    let first = h.session(None);
    let second = h.session(None);
    let status = h.stdout(&["migrate", "--status"]);

    // Assert
    assert!(first.contains("1 item needs you:\n  - "), "{first}");
    assert!(
        first.contains("Fix: playbook setup --statusline"),
        "{first}"
    );
    assert_eq!(second, "", "shown less than a day ago");
    assert!(
        status.contains("Fix: playbook setup --statusline"),
        "{status}"
    );
}
