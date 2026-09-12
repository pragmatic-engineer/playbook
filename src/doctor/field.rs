// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Reads the two single string fields `commands/doctor.md`'s Layer 5 and
//! Layer 6 checks need: `.claude-plugin/plugin.json`'s `.version` and
//! `settings.json`'s `.statusLine.command`. Both used to shell out to
//! `jq -r '<path> // ""' <file> 2>/dev/null`, so a missing `jq` produced the
//! same empty string as the field itself being absent, silently misreporting
//! a healthy install. `playbook` is already required for both layers to mean
//! anything, so this trades an optional external dependency for one that was
//! already assumed.
//!
//! Deliberately not a general JSON query command: each function names the one
//! field it reads. A future call site needing a different field gets its own
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
}
