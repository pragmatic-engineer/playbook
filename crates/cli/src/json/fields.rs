// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Raw string value of one top-level field of a JSON document, replacing
//! `jq -j '.<key>'`/`jq -r '.<key> // empty'` sites. No trailing newline is
//! added, matching `-j` rather than `-r`, so a caller that redirects the
//! output straight into a file (rather than capturing it in a shell
//! variable, where the newline would be stripped anyway) gets the field's
//! exact bytes.

use serde_json::Value;

/// Reads `key` from the JSON object `json`. Empty string when `json` is
/// invalid, `key` is missing or null, or its value is not a string.
pub fn string_field(json: &str, key: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return String::new();
    };
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_field_reads_the_named_field() {
        // Arrange
        let json = r#"{"name": "playbook"}"#;

        // Act
        let got = string_field(json, "name");

        // Assert
        assert_eq!(got, "playbook");
    }

    #[test]
    fn string_field_preserves_embedded_newlines() {
        // Arrange
        let json = r#"{"settings_json": "{\n  \"a\": 1\n}"}"#;

        // Act
        let got = string_field(json, "settings_json");

        // Assert
        assert_eq!(got, "{\n  \"a\": 1\n}");
    }

    #[test]
    fn string_field_is_empty_when_key_is_missing() {
        // Arrange
        let json = r#"{"other": "x"}"#;

        // Act
        let got = string_field(json, "name");

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn string_field_is_empty_on_invalid_json() {
        // Arrange
        let json = "not json";

        // Act
        let got = string_field(json, "name");

        // Assert
        assert_eq!(got, "");
    }
}
