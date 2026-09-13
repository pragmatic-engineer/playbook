// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Reads the JSON shapes `commands/doctor.md`'s Layer 5, 6, and 7 checks
//! need: `.claude-plugin/plugin.json`'s `.version`, `settings.json`'s
//! `.statusLine.command`, and every hook `.command` string nested under
//! `settings.json`'s `.hooks`. All three used to shell out to `jq`, so a
//! missing `jq` produced the same empty result as the field itself being
//! absent or the hooks list being empty, silently misreporting a healthy
//! install. `playbook` is already required for all three layers to mean
//! anything, so this trades an optional external dependency for one that was
//! already assumed.
//!
//! Deliberately not a general JSON query command: each function names the one
//! shape it reads. A future call site needing a different field gets its own
//! function here, not a flag on a generic getter.

use serde_json::Value;
use std::path::Path;

/// Reads `.version` from a `plugin.json`-shaped file. Empty on any failure:
/// unreadable file, invalid JSON, or a missing field, matching
/// `jq -r '.version // ""'`'s null-or-missing fallback. Diverges from `jq`
/// on one edge case both real call sites never hit: a non-string value (a
/// number, say) reads as empty here, where `jq -r` would print it raw.
pub fn plugin_version(path: &Path) -> String {
    string_field(path, &["version"])
}

/// Reads `.statusLine.command` from a `settings.json`-shaped file. Same
/// empty-on-missing-or-null behavior as [`plugin_version`], including its
/// non-string divergence from `jq -r`.
pub fn statusline_command(path: &Path) -> String {
    string_field(path, &["statusLine", "command"])
}

/// Every hook `.command` string nested under `settings.json`'s `.hooks`,
/// across every event and matcher group: `.hooks | to_entries[]? |
/// .value[]? | .hooks[]?.command // empty`, the same traversal
/// `commands/doctor.md`'s Layer 7 used to run through `jq -r`. Empty on any
/// failure (unreadable file, invalid JSON, `.hooks` missing or not an
/// object), matching `jq`'s own `?` operators, which swallow a shape
/// mismatch at each step rather than erroring. Order matches file order:
/// `serde_json::Value`'s object map preserves insertion order (the `preserve_order`
/// feature this crate already depends on), the same order `jq` walks a
/// `to_entries` result in.
///
/// Diverges from `jq -r` on one edge case: a `.command` value that exists
/// but is not a string (a number or bool) is skipped here, where `jq -r`
/// would print its raw text form. Real `settings.json` hook commands are
/// always strings, so this never fires on a real install, the same
/// documented tradeoff [`plugin_version`] and [`statusline_command`] already
/// accept.
pub fn hook_commands(path: &Path) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    let Some(hooks) = value.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for event_value in hooks.values() {
        let Some(matcher_groups) = event_value.as_array() else {
            continue;
        };
        for group in matcher_groups {
            let Some(group_hooks) = group.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for hook in group_hooks {
                if let Some(command) = hook.get("command").and_then(Value::as_str) {
                    out.push(command.to_string());
                }
            }
        }
    }
    out
}

fn string_field(path: &Path, keys: &[&str]) -> String {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return String::new();
    };
    let mut current = &value;
    for key in keys {
        match current.get(key) {
            Some(next) => current = next,
            None => return String::new(),
        }
    }
    current.as_str().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A scratch file holding `contents`, cleaned up on drop.
    struct Fixture {
        path: std::path::PathBuf,
    }

    impl Fixture {
        fn new(tag: &str, contents: &str) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "playbook-doctor-field-{tag}-{}-{n}.json",
                std::process::id()
            ));
            fs::write(&path, contents).expect("fixture file should be writable");
            Self { path }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn plugin_version_reads_the_version_field() {
        // Arrange
        let f = Fixture::new("plugin-version-present", r#"{"version": "0.15.0"}"#);

        // Act
        let got = plugin_version(&f.path);

        // Assert
        assert_eq!(got, "0.15.0");
    }

    #[test]
    fn plugin_version_is_empty_when_field_is_missing() {
        // Arrange
        let f = Fixture::new("plugin-version-missing", r#"{"name": "playbook"}"#);

        // Act
        let got = plugin_version(&f.path);

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn plugin_version_is_empty_when_file_is_missing() {
        // Arrange
        let path = std::env::temp_dir().join("playbook-doctor-field-does-not-exist.json");

        // Act
        let got = plugin_version(&path);

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn plugin_version_is_empty_on_invalid_json() {
        // Arrange
        let f = Fixture::new("plugin-version-invalid", "not json");

        // Act
        let got = plugin_version(&f.path);

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn plugin_version_is_empty_when_value_is_not_a_string() {
        // Arrange
        let f = Fixture::new("plugin-version-non-string", r#"{"version": 15}"#);

        // Act
        let got = plugin_version(&f.path);

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn statusline_command_reads_the_nested_field() {
        // Arrange
        let f = Fixture::new(
            "statusline-present",
            r#"{"statusLine": {"command": "~/.claude/statusline.sh"}}"#,
        );

        // Act
        let got = statusline_command(&f.path);

        // Assert
        assert_eq!(got, "~/.claude/statusline.sh");
    }

    #[test]
    fn statusline_command_is_empty_when_statusline_key_is_absent() {
        // Arrange
        let f = Fixture::new("statusline-no-key", r#"{"other": true}"#);

        // Act
        let got = statusline_command(&f.path);

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn statusline_command_is_empty_when_statusline_is_not_an_object() {
        // Arrange
        let f = Fixture::new(
            "statusline-wrong-shape",
            r#"{"statusLine": "not an object"}"#,
        );

        // Act
        let got = statusline_command(&f.path);

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn hook_commands_collects_across_multiple_events_and_matcher_groups() {
        // Arrange
        let f = Fixture::new(
            "hook-commands-multi-event",
            r#"{
                "hooks": {
                    "PreToolUse": [
                        {"matcher": "Write", "hooks": [{"command": "playbook hook preread-edit-check"}]},
                        {"matcher": "Bash", "hooks": [
                            {"command": "/legacy/guard.py"},
                            {"command": "playbook hook bash-guard"}
                        ]}
                    ],
                    "Stop": [
                        {"hooks": [{"command": "playbook hook session-init"}]}
                    ]
                }
            }"#,
        );

        // Act
        let got = hook_commands(&f.path);

        // Assert
        assert_eq!(
            got,
            vec![
                "playbook hook preread-edit-check",
                "/legacy/guard.py",
                "playbook hook bash-guard",
                "playbook hook session-init",
            ]
        );
    }

    #[test]
    fn hook_commands_is_empty_when_hooks_key_is_absent() {
        // Arrange
        let f = Fixture::new("hook-commands-no-key", r#"{"other": true}"#);

        // Act
        let got = hook_commands(&f.path);

        // Assert
        assert!(got.is_empty());
    }

    #[test]
    fn hook_commands_is_empty_when_hooks_is_not_an_object() {
        // Arrange
        let f = Fixture::new("hook-commands-wrong-shape", r#"{"hooks": "not an object"}"#);

        // Act
        let got = hook_commands(&f.path);

        // Assert
        assert!(got.is_empty());
    }

    #[test]
    fn hook_commands_skips_an_event_whose_value_is_not_an_array() {
        // Arrange: a malformed settings.json shouldn't crash the check, just
        // contribute nothing for that one event.
        let f = Fixture::new(
            "hook-commands-event-not-array",
            r#"{"hooks": {"PreToolUse": "not an array"}}"#,
        );

        // Act
        let got = hook_commands(&f.path);

        // Assert
        assert!(got.is_empty());
    }

    #[test]
    fn hook_commands_skips_a_matcher_group_with_no_hooks_array() {
        // Arrange
        let f = Fixture::new(
            "hook-commands-group-no-hooks",
            r#"{"hooks": {"PreToolUse": [{"matcher": "Write"}]}}"#,
        );

        // Act
        let got = hook_commands(&f.path);

        // Assert
        assert!(got.is_empty());
    }

    #[test]
    fn hook_commands_skips_a_non_string_command_value() {
        // Arrange: divergence from `jq -r`, documented on the function; no
        // real settings.json ever has a non-string command.
        let f = Fixture::new(
            "hook-commands-non-string",
            r#"{"hooks": {"PreToolUse": [{"hooks": [{"command": 5}]}]}}"#,
        );

        // Act
        let got = hook_commands(&f.path);

        // Assert
        assert!(got.is_empty());
    }

    #[test]
    fn hook_commands_is_empty_when_file_is_missing() {
        // Arrange
        let path = std::env::temp_dir().join("playbook-doctor-field-hooks-does-not-exist.json");

        // Act
        let got = hook_commands(&path);

        // Assert
        assert!(got.is_empty());
    }

    #[test]
    fn hook_commands_is_empty_on_invalid_json() {
        // Arrange
        let f = Fixture::new("hook-commands-invalid", "not json");

        // Act
        let got = hook_commands(&f.path);

        // Assert
        assert!(got.is_empty());
    }
}
