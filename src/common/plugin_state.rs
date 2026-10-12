// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Whether the user switched the playbook plugin off in Claude Code.
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
