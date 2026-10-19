// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Updates the installed playbook plugin through the `claude` CLI after
//! `playbook update` and `playbook migrate` (ADR 0034, 2026-10-19
//! amendment). It runs `claude plugin update <id>` for the playbook plugin
//! only, never for another plugin. A missing CLI, a failed update, a plugin
//! that is off in Claude Code or `update.plugin` set to off never fails
//! the command: the result names the command to run by hand.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The config key that turns the plugin update off.
pub const CONFIG_KEY: &str = "update.plugin";

/// Set by `playbook update` for the new binary's `migrate`, which then
/// leaves the plugin to the update itself, so it runs once.
pub const SKIP_ENV: &str = "PLAYBOOK_SKIP_PLUGIN_UPDATE";

/// The value of [`SKIP_ENV`] that turns the update off.
const SKIP_VALUE: &str = "1";

/// The id prefix of the playbook plugin in `installed_plugins.json`.
const ID_PREFIX: &str = "playbook@";

/// The most characters of `claude`'s last error line shown to the user.
const WHY_MAX_CHARS: usize = 200;

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

/// What `update.plugin` asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Ask,
    Off,
}

const MODE_AUTO: &str = "auto";
const MODE_ASK: &str = "ask";
const MODE_OFF: &str = "off";

/// The mode a stored value names, or `None` when it names none. The old
/// boolean values still count, so a store written before the key took words
/// keeps its meaning.
fn parse_mode(value: &serde_json::Value) -> Option<Mode> {
    match value {
        serde_json::Value::String(s) if s == MODE_AUTO => Some(Mode::Auto),
        serde_json::Value::String(s) if s == MODE_ASK => Some(Mode::Ask),
        serde_json::Value::String(s) if s == MODE_OFF => Some(Mode::Off),
        serde_json::Value::Bool(true) => Some(Mode::Auto),
        serde_json::Value::Bool(false) => Some(Mode::Off),
        _ => None,
    }
}

/// The mode for a stored value; a missing or unknown one is `Auto`, the
/// default.
fn mode_from(value: Option<&serde_json::Value>) -> Mode {
    value.and_then(parse_mode).unwrap_or(Mode::Auto)
}

/// The `update.plugin` mode for `home`. The raw stored value is read, not
/// the enum-checked one, because a `true` or `false` from before the key
/// took words is still valid here.
pub fn mode(home: &Path) -> Mode {
    let Ok((value, _)) = crate::config::resolve(CONFIG_KEY, home, None) else {
        return Mode::Auto;
    };
    if parse_mode(&value).is_none() {
        eprintln!(
            "warning: ignoring invalid value {value} for config key {CONFIG_KEY}, valid values are: {MODE_AUTO}, {MODE_ASK}, {MODE_OFF}"
        );
    }
    mode_from(Some(&value))
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
    let mut ids = map.keys().filter(|k| k.starts_with(ID_PREFIX));
    let id = ids.next()?.clone();
    ids.next().is_none().then_some(id)
}

/// Whether `settings.json` turns plugin `id` off. Unlike the hook pause,
/// this follows the id Claude Code lists, so a fork id counts too.
fn is_off_in_claude_code(claude_home: &Path, id: &str) -> bool {
    std::fs::read_to_string(claude_home.join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.pointer("/enabledPlugins")?.get(id).cloned())
        == Some(serde_json::Value::Bool(false))
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

/// Updates the playbook plugin, unless `update.plugin` is off, the plugin
/// is not installed, or it is off in Claude Code.
pub fn run(probe: &Probe) -> Outcome {
    let claude_home = probe.home.join(".claude");
    if mode(&probe.home) == Mode::Off {
        return Outcome::Skipped(format!(
            "{CONFIG_KEY} is off; to turn it on, run: playbook config set {CONFIG_KEY} {MODE_AUTO}"
        ));
    }
    let Some(id) = plugin_id(&claude_home) else {
        return Outcome::NotInstalled;
    };
    if is_off_in_claude_code(&claude_home, &id) {
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
            .map(|l| {
                crate::ui::render::sanitize(&l.chars().take(WHY_MAX_CHARS).collect::<String>())
            })
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
    let skip = no_plugin || std::env::var_os(SKIP_ENV).is_some_and(|v| v == SKIP_VALUE);
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
            &serde_json::json!("off"),
        )
        .unwrap();
        let config_off = run(&Probe {
            home: home.clone(),
            claude: Some(claude),
        });

        // Assert
        let (off, config_off) = (off.line().unwrap(), config_off.line().unwrap());
        assert!(off.contains("turned off in Claude Code"), "{off}");
        assert!(config_off.contains("update.plugin is off"), "{config_off}");
        assert_eq!(calls(&home), "");
    }

    #[test]
    fn mode_reads_each_word() {
        // Arrange
        let words = [("auto", Mode::Auto), ("ask", Mode::Ask), ("off", Mode::Off)];

        for (word, expected) in words {
            // Act
            let mode = mode_from(Some(&serde_json::json!(word)));

            // Assert
            assert_eq!(mode, expected, "{word}");
        }
        assert_eq!(
            mode_from(None),
            Mode::Auto,
            "a missing value is the default"
        );
    }

    #[test]
    fn a_stored_true_reads_as_auto_and_false_as_off() {
        // Arrange
        let home = scratch_dir("plugin-update-bool-mode");
        let root = crate::common::paths::playbook_root_from(&home);
        fs::create_dir_all(&root).unwrap();
        let put = |value: serde_json::Value| {
            crate::config::store::put(
                &root,
                crate::config::write::Tier::Global,
                None,
                CONFIG_KEY,
                &value,
            )
            .unwrap();
        };

        // Act
        put(serde_json::json!(true));
        let on = mode(&home);
        put(serde_json::json!(false));
        let off = mode(&home);

        // Assert
        assert_eq!((on, off), (Mode::Auto, Mode::Off));
    }

    #[test]
    fn every_allowed_word_for_the_key_parses_to_a_mode() {
        // Arrange
        let words = crate::config::keys::allowed_enum_values(CONFIG_KEY).unwrap();

        for word in words {
            // Act
            let mode = parse_mode(&serde_json::json!(word));

            // Assert
            assert!(mode.is_some(), "{word}");
        }
    }

    #[test]
    fn a_stored_ask_updates_the_plugin() {
        // Arrange
        let home = scratch_dir("plugin-update-ask");
        let dir = plugin(&home, "0.36.0");
        install(&home, "playbook@pragmatic-engineer", &dir);
        let claude = fake_claude(&home, None, 0);
        let root = crate::common::paths::playbook_root_from(&home);
        fs::create_dir_all(&root).unwrap();
        crate::config::store::put(
            &root,
            crate::config::write::Tier::Global,
            None,
            CONFIG_KEY,
            &serde_json::json!(MODE_ASK),
        )
        .unwrap();

        // Act
        let out = run(&Probe {
            home: home.clone(),
            claude: Some(claude),
        });

        // Assert
        assert_eq!(calls(&home), "plugin update playbook@pragmatic-engineer\n");
        assert!(!matches!(out, Outcome::Skipped(_)), "{out:?}");
    }

    #[test]
    fn an_unknown_word_reads_as_auto() {
        // Arrange
        let unknown = serde_json::json!("sometimes");

        // Act
        let mode = mode_from(Some(&unknown));

        // Assert
        assert_eq!(mode, Mode::Auto);
        assert_eq!(parse_mode(&unknown), None);
    }

    #[test]
    fn a_fork_id_that_is_off_runs_nothing() {
        // Arrange
        let home = scratch_dir("plugin-update-fork-off");
        let dir = plugin(&home, "0.36.0");
        install(&home, "playbook@my-fork", &dir);
        let claude = fake_claude(&home, None, 0);
        fs::write(
            home.join(".claude/settings.json"),
            r#"{"enabledPlugins":{"playbook@my-fork":false}}"#,
        )
        .unwrap();

        // Act
        let out = run(&Probe {
            home: home.clone(),
            claude: Some(claude),
        });

        // Assert
        assert!(out.line().unwrap().contains("turned off"));
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
