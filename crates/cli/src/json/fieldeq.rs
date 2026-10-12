// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! String-equality assertion against one field of a JSON document, keyed by
//! a dotted path where a numeric segment indexes into an array. Replaces
//! `jq -e '<path> == "<value>"'` sites.

use serde_json::Value;

/// Reports whether the value at `path` inside `json` is the string
/// `expected`. Each `.`-separated segment of `path` is either an object key
/// or, when it parses as an unsigned integer, an array index (so
/// `"a.0.b"` reads `.a[0].b`). False on any mismatch: a missing segment, a
/// value that is not a string, or `json` itself failing to parse; mirrors
/// `jq -e`'s non-zero exit on a false, null, or absent result.
pub fn field_equals(json: &str, path: &str, expected: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return false;
    };
    let mut current = &value;
    for segment in path.split('.') {
        let next = match segment.parse::<usize>() {
            Ok(index) => current.get(index),
            Err(_) => current.get(segment),
        };
        match next {
            Some(v) => current = v,
            None => return false,
        }
    }
    current.as_str() == Some(expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_equals_is_true_for_a_matching_top_level_field() {
        // Arrange
        let json = r#"{"decision": "block"}"#;

        // Act
        let got = field_equals(json, "decision", "block");

        // Assert
        assert!(got);
    }

    #[test]
    fn field_equals_is_true_for_a_matching_nested_field() {
        // Arrange
        let json = r#"{"hookSpecificOutput": {"permissionDecision": "deny"}}"#;

        // Act
        let got = field_equals(json, "hookSpecificOutput.permissionDecision", "deny");

        // Assert
        assert!(got);
    }

    #[test]
    fn field_equals_walks_a_numeric_segment_as_an_array_index() {
        // Arrange
        let json = r#"{"hooks":{"Notification":[{"hooks":[{"command":"/opt/notify.sh"}]}]}}"#;

        // Act
        let got = field_equals(
            json,
            "hooks.Notification.0.hooks.0.command",
            "/opt/notify.sh",
        );

        // Assert
        assert!(got);
    }

    #[test]
    fn field_equals_is_false_when_the_value_differs() {
        // Arrange
        let json = r#"{"decision": "allow"}"#;

        // Act
        let got = field_equals(json, "decision", "block");

        // Assert
        assert!(!got);
    }

    #[test]
    fn field_equals_is_false_when_the_path_is_missing() {
        // Arrange
        let json = r#"{"other": "value"}"#;

        // Act
        let got = field_equals(json, "decision", "block");

        // Assert
        assert!(!got);
    }

    #[test]
    fn field_equals_is_false_on_malformed_json() {
        // Arrange
        let json = "not json";

        // Act
        let got = field_equals(json, "decision", "block");

        // Assert
        assert!(!got);
    }
}
