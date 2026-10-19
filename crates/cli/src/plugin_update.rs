// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Updates the installed playbook plugin through the `claude` CLI after
//! `playbook update` and `playbook migrate` (ADR 0034, 2026-10-19
//! amendment). It runs `claude plugin update <id>` for the playbook plugin
//! only, never for another plugin. A missing CLI, a failed update, a plugin
//! that is off in Claude Code or `update.plugin` set to false never fails
//! the command: the result names the command to run by hand.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The config key that turns the plugin update off.
pub const CONFIG_KEY: &str = "update.plugin";

/// Set by `playbook update` for the new binary's `migrate`, which then
/// leaves the plugin to the update itself, so it runs once.
pub const SKIP_ENV: &str = "PLAYBOOK_SKIP_PLUGIN_UPDATE";

/// How long `claude plugin update` may take.
const TIMEOUT: Duration = Duration::from_secs(120);

/// What a plugin update did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// `claude plugin update` ran and the installed version moved.
    Updated {
        id: String,
        from: Option<String>,
        to: Option<String>,
    },
    /// `claude plugin update` ran and the version stayed.
    Current { id: String, version: Option<String> },
    /// Nothing ran, for this reason.
    Skipped(String),
    /// The playbook plugin is not installed, so there is nothing to say.
    NotInstalled,
    /// `claude` is not on PATH.
    NoCli { id: String },
    /// `claude plugin update` failed or timed out.
    Failed { id: String, why: String },
}

impl Outcome {
    /// One line for the user, with the command to run when it did not run.
    /// `None` when the plugin is not installed.
    pub fn line(&self) -> Option<String> {
        Some(match self {
            Outcome::NotInstalled => return None,
            Outcome::Updated { id, from, to } => format!(
                "plugin: updated {id}{}{}. Restart Claude Code to load it",
                from.as_deref()
                    .map(|v| format!(" from {v}"))
                    .unwrap_or_default(),
                to.as_deref()
                    .map(|v| format!(" to {v}"))
                    .unwrap_or_default(),
            ),
            Outcome::Current { id, version } => format!(
                "plugin: {id} is up to date{}",
                version
                    .as_deref()
                    .map(|v| format!(" ({v})"))
                    .unwrap_or_default()
            ),
            Outcome::Skipped(why) => format!("plugin: not updated, {why}"),
            Outcome::NoCli { id } => format!(
                "plugin: the claude command is not on PATH, so the plugin was not updated; run: {}",
                command(id)
            ),
            Outcome::Failed { id, why } => {
                format!("plugin: the update failed ({why}); run: {}", command(id))
            }
        })
    }
}

/// The command that updates plugin `id`.
pub fn command(id: &str) -> String {
    format!("claude plugin update {id}")
}

/// Whether the plugin update is on for `home`: `update.plugin` is not
/// false. An unreadable value counts as on.
pub fn enabled(home: &Path) -> bool {
    crate::config::resolve(CONFIG_KEY, home, None)
        .ok()
        .and_then(|(v, _)| v.as_bool())
        .unwrap_or(true)
}

/// The id Claude Code lists for the playbook plugin in
/// `installed_plugins.json`: `playbook@pragmatic-engineer` when listed,
/// else the one `playbook@<marketplace>` entry. `None` when the plugin is
/// not installed, or more than one other marketplace lists it.
pub fn plugin_id(claude_home: &Path) -> Option<String> {
    use crate::common::plugin_state::{installed_entry_at, PLUGIN_KEY};
    if installed_entry_at(claude_home, PLUGIN_KEY).is_some() {
        return Some(PLUGIN_KEY.to_string());
    }
    let text = std::fs::read_to_string(claude_home.join("plugins/installed_plugins.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let map = value.get("plugins").unwrap_or(&value).as_object()?;
    let mut ids = map.keys().filter(|k| k.starts_with("playbook@"));
    let id = ids.next()?.clone();
    ids.next().is_none().then_some(id)
}

/// The version Claude Code lists for plugin `id`, from its `installPath`.
fn installed_version(claude_home: &Path, id: &str) -> Option<String> {
    let entry = crate::common::plugin_state::installed_entry_at(claude_home, id)?;
    let first = match entry {
        serde_json::Value::Array(list) => list.into_iter().next()?,
        other => other,
    };
    let path = first.get("installPath")?.as_str()?;
    crate::plugin_root::plugin_version(Path::new(path))
        .filter(|v| crate::plugin_root::is_plain_version(v))
}

/// What the update reads, passed in so tests use a scratch home and a fake
/// `claude`.
#[derive(Debug, Clone)]
pub struct Probe {
    pub home: PathBuf,
    /// The `claude` executable, or `None` when it is not on PATH.
    pub claude: Option<PathBuf>,
}

impl Probe {
    /// The probe for this process: `claude` from PATH.
    pub fn for_home(home: &Path) -> Probe {
        let path = std::env::var("PATH").unwrap_or_default();
        Probe {
            home: home.to_path_buf(),
            claude: crate::statusline::gh::find_in_path("claude", &path),
        }
    }
}

/// Updates the playbook plugin, unless `update.plugin` is false, the plugin
/// is not installed, or it is off in Claude Code.
pub fn run(probe: &Probe) -> Outcome {
    let claude_home = probe.home.join(".claude");
    if !enabled(&probe.home) {
        return Outcome::Skipped(format!(
            "{CONFIG_KEY} is false; to turn it on, run: playbook config set {CONFIG_KEY} true"
        ));
    }
    let Some(id) = plugin_id(&claude_home) else {
        return Outcome::NotInstalled;
    };
    if crate::common::plugin_state::is_disabled_in(&probe.home) {
        return Outcome::Skipped(format!(
            "{id} is turned off in Claude Code; turn it on there, then run: {}",
            command(&id)
        ));
    }
    let Some(claude) = &probe.claude else {
        return Outcome::NoCli { id };
    };
    let before = installed_version(&claude_home, &id);
    let mut cmd = std::process::Command::new(claude);
    cmd.args(["plugin", "update", &id])
        .env("HOME", &probe.home)
        .stdin(std::process::Stdio::null());
    let Some(out) = crate::common::proc::run_with_timeout(&mut cmd, TIMEOUT) else {
        return Outcome::Failed {
            id,
            why: format!("it did not finish within {} s", TIMEOUT.as_secs()),
        };
    };
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        let last = text
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .map(|l| crate::ui::render::sanitize(&l.chars().take(200).collect::<String>()))
            .unwrap_or_default();
        let why = if last.is_empty() {
            format!("exit {}", out.status.code().unwrap_or(-1))
        } else {
            last
        };
        return Outcome::Failed { id, why };
    }
    let after = installed_version(&claude_home, &id);
    if after.is_some() && after != before {
        Outcome::Updated {
            id,
            from: before,
            to: after,
        }
    } else {
        Outcome::Current {
            id,
            version: after.or(before),
        }
    }
}

/// [`run`] for a command, unless `--no-plugin` was passed or the caller set
/// [`SKIP_ENV`]. `None` when it was not asked to run.
pub fn run_for_command(home: &Path, no_plugin: bool) -> Option<Outcome> {
    let skip = no_plugin || std::env::var_os(SKIP_ENV).is_some_and(|v| v == "1");
    (!skip).then(|| run(&Probe::for_home(home)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;

    fn plugin(home: &Path, version: &str) -> PathBuf {
        let dir = home.join(format!(
            ".claude/plugins/cache/pragmatic-engineer/playbook/{version}"
        ));
        fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        fs::create_dir_all(dir.join("skills")).unwrap();
        fs::write(
            dir.join(".claude-plugin/plugin.json"),
            format!(r#"{{"name":"playbook","version":"{version}"}}"#),
        )
        .unwrap();
        dir
    }

    fn install(home: &Path, key: &str, dir: &Path) {
        fs::write(
            home.join(".claude/plugins/installed_plugins.json"),
            format!(
                r#"{{"version":2,"plugins":{{"{key}":[{{"scope":"user","installPath":"{}"}}],"other@x":[{{"scope":"user","installPath":"/nowhere"}}]}}}}"#,
                dir.display()
            ),
        )
        .unwrap();
    }

    /// A fake `claude` that logs its arguments and, for `plugin update`,
    /// points installed_plugins.json at `to` when given.
    fn fake_claude(home: &Path, to: Option<&Path>, exit: i32) -> PathBuf {
        let bin = home.join("bin/claude");
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        let swap = to.map_or(String::new(), |dir| {
            format!(
                "printf '%s' '{{\"version\":2,\"plugins\":{{\"playbook@pragmatic-engineer\":[{{\"scope\":\"user\",\"installPath\":\"{}\"}}]}}}}' > \"$HOME/.claude/plugins/installed_plugins.json\"\n",
                dir.display()
            )
        });
        fs::write(
            &bin,
            format!(
                "#!/bin/sh\necho \"$*\" >> \"$HOME/claude-calls.log\"\n{swap}echo 'update failed: network' >&2\nexit {exit}\n"
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    fn calls(home: &Path) -> String {
        fs::read_to_string(home.join("claude-calls.log")).unwrap_or_default()
    }

    #[test]
    fn it_updates_only_the_playbook_plugin_and_says_to_restart() {
        // Arrange
        let home = scratch_dir("plugin-update-ok");
        let old = plugin(&home, "0.36.0");
        let new = plugin(&home, "0.37.0");
        install(&home, "playbook@pragmatic-engineer", &old);
        let claude = fake_claude(&home, Some(&new), 0);

        // Act
        let out = run(&Probe {
            home: home.clone(),
            claude: Some(claude),
        });

        // Assert
        assert_eq!(calls(&home), "plugin update playbook@pragmatic-engineer\n");
        assert_eq!(
            out.line().unwrap(),
            "plugin: updated playbook@pragmatic-engineer from 0.36.0 to 0.37.0. Restart Claude Code to load it"
        );
    }

    #[test]
    fn a_missing_cli_or_a_failed_update_names_the_command_and_does_not_fail() {
        let home = scratch_dir("plugin-update-fail");
        let dir = plugin(&home, "0.36.0");
        install(&home, "playbook@pragmatic-engineer", &dir);
        let missing = run(&Probe {
            home: home.clone(),
            claude: None,
        });
        let failed = run(&Probe {
            home: home.clone(),
            claude: Some(fake_claude(&home, None, 1)),
        });
        assert!(missing
            .line()
            .unwrap()
            .ends_with("run: claude plugin update playbook@pragmatic-engineer"));
        assert_eq!(
            failed.line().unwrap(),
            "plugin: the update failed (update failed: network); run: claude plugin update playbook@pragmatic-engineer"
        );
    }

    #[test]
    fn a_plugin_that_is_off_or_a_false_config_key_runs_nothing() {
        // Arrange
        let home = scratch_dir("plugin-update-off");
        let dir = plugin(&home, "0.36.0");
        install(&home, "playbook@pragmatic-engineer", &dir);
        let claude = fake_claude(&home, None, 0);
        fs::write(
            home.join(".claude/settings.json"),
            r#"{"enabledPlugins":{"playbook@pragmatic-engineer":false}}"#,
        )
        .unwrap();

        // Act
        let off = run(&Probe {
            home: home.clone(),
            claude: Some(claude.clone()),
        });
        fs::write(home.join(".claude/settings.json"), "{}").unwrap();
        let root = crate::common::paths::playbook_root_from(&home);
        fs::create_dir_all(&root).unwrap();
        crate::config::store::put(
            &root,
            crate::config::write::Tier::Global,
            None,
            CONFIG_KEY,
            &serde_json::json!(false),
        )
        .unwrap();
        let config_off = run(&Probe {
            home: home.clone(),
            claude: Some(claude),
        });

        // Assert
        let (off, config_off) = (off.line().unwrap(), config_off.line().unwrap());
        assert!(off.contains("turned off in Claude Code"), "{off}");
        assert!(
            config_off.contains("update.plugin is false"),
            "{config_off}"
        );
        assert_eq!(calls(&home), "");
    }

    #[test]
    fn the_id_comes_from_installed_plugins_and_nothing_runs_without_the_plugin() {
        let home = scratch_dir("plugin-update-id");
        fs::create_dir_all(home.join(".claude/plugins")).unwrap();
        assert_eq!(plugin_id(&home.join(".claude")), None);
        assert_eq!(
            run(&Probe {
                home: home.clone(),
                claude: None
            }),
            Outcome::NotInstalled
        );
        let dir = plugin(&home, "0.37.0");
        install(&home, "playbook@my-fork", &dir);
        assert_eq!(
            plugin_id(&home.join(".claude")).as_deref(),
            Some("playbook@my-fork")
        );
        install(&home, "playbook@pragmatic-engineer", &dir);
        assert_eq!(
            plugin_id(&home.join(".claude")).as_deref(),
            Some("playbook@pragmatic-engineer")
        );
    }
}
