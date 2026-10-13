// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Whether the user switched the playbook plugin off in Claude Code, and
//! which of the two plugins (`playbook`, `playbook-hooks`) are installed.
//!
//! `playbook init` writes user-level hook entries and a status line into
//! `~/.claude/settings.json`. Disabling the plugin in Claude Code sets
//! `enabledPlugins["playbook@pragmatic-engineer"]` to `false` and leaves those
//! entries in place, so every hook entry point and the status line ask here
//! first. Only an explicit `false` pauses playbook. A missing key, a missing
//! file and a file that cannot be read or parsed all count as enabled, so
//! Homebrew-only users and broken settings never lose their guards.

use std::path::Path;

/// The plugin's key in `enabledPlugins`.
pub const PLUGIN_KEY: &str = "playbook@pragmatic-engineer";

/// Set to `1` to run hooks and the status line even when the plugin is disabled.
pub const IGNORE_ENV: &str = "PLAYBOOK_IGNORE_PLUGIN_DISABLED";

/// Whether this process should do nothing because the plugin is disabled.
/// Reads `~/.claude/settings.json` once. The escape hatch wins over the file.
pub fn paused() -> bool {
    if std::env::var(IGNORE_ENV).is_ok_and(|v| v == "1") {
        return false;
    }
    is_disabled_in(&crate::common::home_dir())
}

/// Whether `home`'s `.claude/settings.json` sets the plugin to `false`.
pub fn is_disabled_in(home: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(home.join(".claude").join("settings.json")) else {
        return false;
    };
    // Most users have no entry, so skip the JSON parse unless the key is there.
    if !text.contains(PLUGIN_KEY) {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v.pointer("/enabledPlugins")
                .and_then(|e| e.get(PLUGIN_KEY).cloned())
        })
        == Some(serde_json::Value::Bool(false))
}

/// The marketplace the plugin comes from.
pub const MARKETPLACE: &str = "pragmatic-engineer";

/// The hint shown when the plugin was removed. The status line and the
/// SessionStart hook use this one text.
pub const REMOVED_HINT: &str =
    "playbook plugin was removed, run `playbook uninstall --yes` to clean up";

/// Config key that turns the removed-plugin hint off.
pub const HINT_KEY: &str = "hints.pluginRemoved";

/// Where `playbook init` records that it saw the plugin installed.
pub fn seen_marker(home: &Path) -> std::path::PathBuf {
    crate::common::paths::playbook_root_from(home).join("plugin-seen")
}

/// The hooks plugin's key. It carries the worktree hooks and the installer
/// shim, and ships separately from the `playbook` plugin.
pub const HOOKS_PLUGIN_KEY: &str = "playbook-hooks@pragmatic-engineer";

/// The hint shown to someone who has the `playbook` plugin but not the hooks
/// plugin, which is how every install looked before the two were split.
pub const HOOKS_MISSING_HINT: &str = "the playbook-hooks plugin is not installed, so the worktree hooks are off. Add it with: claude plugin marketplace update pragmatic-engineer && claude plugin install playbook-hooks@pragmatic-engineer";

fn installed_entry(home: &Path, key: &str) -> Option<Option<serde_json::Value>> {
    let path = home.join(".claude/plugins/installed_plugins.json");
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let find = |v: &serde_json::Value| v.get(key).filter(|e| !e.is_null()).cloned();
    Some(value.get("plugins").and_then(find).or_else(|| find(&value)))
}

/// Whether Claude Code lists `key` as installed, or `None` when
/// `installed_plugins.json` is missing or cannot be parsed.
pub fn listed_in(home: &Path, key: &str) -> Option<bool> {
    installed_entry(home, key).map(|entry| entry.is_some())
}

/// Whether Claude Code lists the plugin as installed, or `None` when
/// `installed_plugins.json` is missing or cannot be parsed.
pub fn installed_in(home: &Path) -> Option<bool> {
    listed_in(home, PLUGIN_KEY)
}

/// True when the `playbook` plugin is installed and the hooks plugin is not.
/// A machine with neither (a Homebrew-only install) is not told anything: its
/// hooks come from `settings.json`. A user who switched the `playbook` plugin
/// off is not nagged either.
pub fn hooks_plugin_missing(home: &Path) -> bool {
    installed_in(home) == Some(true)
        && listed_in(home, HOOKS_PLUGIN_KEY) == Some(false)
        && !is_disabled_in(home)
}

/// `HOOKS_MISSING_HINT` when it applies and the user has not turned plugin
/// hints off with `hints.pluginRemoved`.
pub fn hooks_missing_hint(home: &Path) -> Option<&'static str> {
    (hooks_plugin_missing(home) && !hints_off(home)).then_some(HOOKS_MISSING_HINT)
}

fn hints_off(home: &Path) -> bool {
    crate::config::resolve(HINT_KEY, home, None)
        .ok()
        .and_then(|(v, _)| v.as_bool())
        == Some(false)
}

/// Whether the marketplace is listed in `known_marketplaces.json`.
pub fn marketplace_known_in(home: &Path) -> bool {
    std::fs::read_to_string(home.join(".claude/plugins/known_marketplaces.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v.get(MARKETPLACE).is_some())
}

/// Record, in playbook's own state, that the plugin is installed right now.
/// Does nothing when it is not, and never touches Claude Code's files.
pub fn record_seen(home: &Path) {
    if installed_in(home) != Some(true) {
        return;
    }
    let marker = seen_marker(home);
    if marker.is_file() {
        return;
    }
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(marker, "1\n");
}

/// Whether the plugin was seen installed and has since been removed from
/// `installed_plugins.json`. Disabled is not removed, and a user who never
/// had the plugin never matches. An unreadable list counts as not removed.
pub fn removed(home: &Path) -> bool {
    installed_in(home) == Some(false) && seen_marker(home).is_file()
}

/// The hint to show, or `None` when the plugin was not removed or the user
/// turned the hint off with `hints.pluginRemoved`.
pub fn removed_hint(home: &Path) -> Option<&'static str> {
    (removed(home) && !hints_off(home)).then_some(REMOVED_HINT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    fn home_with(settings: Option<&str>) -> std::path::PathBuf {
        let home = scratch_dir("plugin-state");
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        if let Some(text) = settings {
            std::fs::write(home.join(".claude/settings.json"), text).unwrap();
        }
        home
    }

    #[test]
    fn an_explicit_false_means_disabled() {
        let home = home_with(Some(
            r#"{"enabledPlugins":{"playbook@pragmatic-engineer":false}}"#,
        ));
        assert!(is_disabled_in(&home));
    }

    #[test]
    fn true_means_enabled() {
        let home = home_with(Some(
            r#"{"enabledPlugins":{"playbook@pragmatic-engineer":true}}"#,
        ));
        assert!(!is_disabled_in(&home));
    }

    #[test]
    fn an_absent_key_or_absent_block_means_enabled() {
        assert!(!is_disabled_in(&home_with(Some(
            r#"{"enabledPlugins":{"other@x":false}}"#
        ))));
        assert!(!is_disabled_in(&home_with(Some(r#"{"hooks":{}}"#))));
    }

    #[test]
    fn a_missing_file_means_enabled() {
        assert!(!is_disabled_in(&home_with(None)));
    }

    #[test]
    fn a_malformed_file_means_enabled() {
        let home = home_with(Some(
            r#"{"enabledPlugins":{"playbook@pragmatic-engineer":false"#,
        ));
        assert!(!is_disabled_in(&home));
    }

    #[test]
    fn a_non_boolean_value_means_enabled() {
        let home = home_with(Some(
            r#"{"enabledPlugins":{"playbook@pragmatic-engineer":"false"}}"#,
        ));
        assert!(!is_disabled_in(&home));
    }

    fn write_installed(home: &std::path::Path, body: &str) {
        let dir = home.join(".claude/plugins");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("installed_plugins.json"), body).unwrap();
    }

    const LISTED: &str =
        r#"{"version":2,"plugins":{"playbook@pragmatic-engineer":[{"scope":"user"}]}}"#;
    const EMPTY: &str = r#"{"version":2,"plugins":{}}"#;

    #[test]
    fn init_records_the_plugin_only_when_it_is_installed() {
        let absent = home_with(None);
        write_installed(&absent, EMPTY);
        record_seen(&absent);
        assert!(!seen_marker(&absent).exists());
        let present = home_with(None);
        write_installed(&present, LISTED);
        record_seen(&present);
        assert!(seen_marker(&present).is_file());
    }

    #[test]
    fn a_seen_plugin_that_left_the_list_counts_as_removed() {
        let home = home_with(None);
        write_installed(&home, LISTED);
        record_seen(&home);
        assert!(!removed(&home));
        write_installed(&home, EMPTY);
        assert!(removed(&home));
        assert_eq!(removed_hint(&home), Some(REMOVED_HINT));
    }

    #[test]
    fn a_user_who_never_had_the_plugin_is_never_told_it_was_removed() {
        let home = home_with(None);
        write_installed(&home, EMPTY);
        assert!(!removed(&home));
        assert_eq!(removed_hint(&home), None);
    }

    #[test]
    fn a_disabled_plugin_that_is_still_listed_is_not_removed() {
        let home = home_with(Some(
            r#"{"enabledPlugins":{"playbook@pragmatic-engineer":false}}"#,
        ));
        write_installed(&home, LISTED);
        record_seen(&home);
        assert!(!removed(&home));
    }

    #[test]
    fn a_missing_or_malformed_list_is_not_removed() {
        let home = home_with(None);
        write_installed(&home, LISTED);
        record_seen(&home);
        write_installed(&home, "{not json");
        assert!(!removed(&home));
        let _ = std::fs::remove_file(home.join(".claude/plugins/installed_plugins.json"));
        assert!(!removed(&home));
    }

    const BOTH: &str = r#"{"version":2,"plugins":{"playbook@pragmatic-engineer":[{"version":"0.25.0"}],"playbook-hooks@pragmatic-engineer":[{"version":"0.25.1"}]}}"#;

    #[test]
    fn the_hooks_plugin_is_missing_only_when_the_skills_plugin_is_installed() {
        let skills_only = home_with(None);
        write_installed(&skills_only, LISTED);
        assert!(hooks_plugin_missing(&skills_only));
        assert_eq!(hooks_missing_hint(&skills_only), Some(HOOKS_MISSING_HINT));
        let both = home_with(None);
        write_installed(&both, BOTH);
        assert!(!hooks_plugin_missing(&both));
        let neither = home_with(None);
        write_installed(&neither, EMPTY);
        assert!(!hooks_plugin_missing(&neither));
        assert!(!hooks_plugin_missing(&home_with(None)));
    }

    #[test]
    fn the_older_object_shape_and_a_disabled_plugin_are_handled() {
        let old_shape = home_with(None);
        write_installed(
            &old_shape,
            r#"{"version":1,"plugins":{"playbook@pragmatic-engineer":{"version":"0.25.0"}}}"#,
        );
        assert!(hooks_plugin_missing(&old_shape));
        let disabled = home_with(Some(
            r#"{"enabledPlugins":{"playbook@pragmatic-engineer":false}}"#,
        ));
        write_installed(&disabled, LISTED);
        assert!(!hooks_plugin_missing(&disabled));
    }

    #[test]
    fn the_config_key_turns_the_hooks_hint_off_too() {
        let home = home_with(None);
        write_installed(&home, LISTED);
        crate::config::write::set(
            crate::config::write::Tier::Global,
            HINT_KEY,
            serde_json::Value::Bool(false),
            &home,
            None,
        )
        .unwrap();
        assert_eq!(hooks_missing_hint(&home), None);
    }

    #[test]
    fn the_config_key_turns_the_hint_off() {
        let home = home_with(None);
        write_installed(&home, LISTED);
        record_seen(&home);
        write_installed(&home, EMPTY);
        crate::config::write::set(
            crate::config::write::Tier::Global,
            HINT_KEY,
            serde_json::Value::Bool(false),
            &home,
            None,
        )
        .unwrap();
        assert_eq!(removed_hint(&home), None);
    }
}
