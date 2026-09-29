// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Prints a settings-style JSON document with a top-level key change
//! applied, either named keys removed or one boolean-true marker key added,
//! replacing `jq 'del(.key)'` and `jq '. + {"key": true}'` call sites that
//! build a fixture file rather than compare two documents (see
//! `json::jsoncmp` for the comparison case).

use crate::json::jsoncmp::drop_top_level_keys;

/// Parses `json`, drops `keys` via `drop_top_level_keys`, and re-serializes
/// pretty-printed, matching `serde_json::to_string_pretty`'s established use
/// for settings-shaped output elsewhere in this repo (`settings::gen`).
/// Returns malformed input back out verbatim: a fixture-building call site
/// has no differing-keys report to produce, so the input itself is the most
/// useful diagnostic on parse failure.
pub fn remove_keys_print(json: &str, keys: &[&str]) -> String {
    let Ok(mut value) = serde_json::from_str(json) else {
        return json.to_string();
    };
    drop_top_level_keys(&mut value, keys);
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| json.to_string())
}

/// Parses `json`, sets `key` to boolean `true` at the top level (overwriting
/// any existing value under that name), and re-serializes pretty-printed.
/// Narrow by design for building a drift fixture with one marker key added,
/// not a general JSON-merge primitive; a caller needing a different value or
/// multiple keys gets its own function. Same malformed-input fallback as
/// [`remove_keys_print`].
pub fn add_marker_key_print(json: &str, key: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(json) else {
        return json.to_string();
    };
    if let Some(map) = value.as_object_mut() {
        map.insert(key.to_string(), serde_json::Value::Bool(true));
    }
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| json.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn remove_keys_print_removes_a_present_key() {
        // Arrange
        let input = r#"{"a": 1, "editorMode": "vim", "b": 2}"#;

        // Act
        let got = super::remove_keys_print(input, &["editorMode"]);

        // Assert
        let parsed: serde_json::Value = serde_json::from_str(&got).expect("output is valid JSON");
        assert_eq!(parsed, json!({"a": 1, "b": 2}));
    }

    #[test]
    fn remove_keys_print_is_a_noop_when_key_is_absent() {
        // Arrange
        let input = r#"{"a": 1, "b": 2}"#;

        // Act
        let got = super::remove_keys_print(input, &["missing"]);

        // Assert
        let parsed: serde_json::Value = serde_json::from_str(&got).expect("output is valid JSON");
        assert_eq!(parsed, json!({"a": 1, "b": 2}));
    }

    #[test]
    fn remove_keys_print_preserves_remaining_keys_and_structure() {
        // Arrange
        let input = r#"{"a": 1, "nested": {"x": true}, "list": [1, 2, 3], "drop": "gone"}"#;

        // Act
        let got = super::remove_keys_print(input, &["drop"]);

        // Assert
        let parsed: serde_json::Value = serde_json::from_str(&got).expect("output is valid JSON");
        assert_eq!(
            parsed,
            json!({"a": 1, "nested": {"x": true}, "list": [1, 2, 3]})
        );
    }

    #[test]
    fn add_marker_key_print_adds_a_boolean_true_key() {
        // Arrange
        let input = r#"{"a": 1}"#;

        // Act
        let got = super::add_marker_key_print(input, "zzExtraKey");

        // Assert
        let parsed: serde_json::Value = serde_json::from_str(&got).expect("output is valid JSON");
        assert_eq!(parsed, json!({"a": 1, "zzExtraKey": true}));
    }

    #[test]
    fn add_marker_key_print_overwrites_an_existing_key_with_true() {
        // Arrange
        let input = r#"{"a": 1, "zzExtraKey": "existing"}"#;

        // Act
        let got = super::add_marker_key_print(input, "zzExtraKey");

        // Assert
        let parsed: serde_json::Value = serde_json::from_str(&got).expect("output is valid JSON");
        assert_eq!(parsed, json!({"a": 1, "zzExtraKey": true}));
    }

    #[test]
    fn add_marker_key_print_returns_input_unchanged_on_malformed_json() {
        // Arrange
        let input = "not json";

        // Act
        let got = super::add_marker_key_print(input, "zzExtraKey");

        // Assert
        assert_eq!(got, input);
    }
}
