// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Whether the user switched the playbook skills plugin off in Claude Code,
//! and whether Claude Code lists it as installed.
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

/// Whether the marketplace is listed in `known_marketplaces.json`.
pub fn marketplace_known_in(home: &Path) -> bool {
    std::fs::read_to_string(home.join(".claude/plugins/known_marketplaces.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v.get(MARKETPLACE).is_some())
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
}
