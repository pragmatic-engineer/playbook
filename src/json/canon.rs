// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Sorted-key JSON serialization for `diff`-based semantic comparisons,
//! replacing `jq -S`/`jq -S 'del(.<key>)'` sites: two documents that agree
//! on every value but differ in key order, or in a key expected to differ
//! (a template's raw hooks wiring versus an installed file's, once that key
//! is dropped), produce byte-identical output here.

use serde_json::{Map, Value};

/// Parses `json`, drops the top-level `del_key` field first if given, then
/// serializes it back with every object's keys sorted recursively. Empty
/// string on invalid JSON, so a `diff` against it fails loudly rather than
/// passing by coincidence.
pub fn canonical_json(json: &str, del_key: Option<&str>) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(json) else {
        return String::new();
    };
    if let (Some(key), Some(obj)) = (del_key, value.as_object_mut()) {
        obj.remove(key);
    }
    serde_json::to_string_pretty(&sort_keys(value)).unwrap_or_default()
}

fn sort_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> =
                map.into_iter().map(|(k, v)| (k, sort_keys(v))).collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().collect::<Map<String, Value>>())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_keys).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_produces_identical_output_regardless_of_key_order() {
        // Arrange
        let a = r#"{"b": 2, "a": 1}"#;
        let b = r#"{"a": 1, "b": 2}"#;

        // Act
        let got_a = canonical_json(a, None);
        let got_b = canonical_json(b, None);

        // Assert
        assert_eq!(got_a, got_b);
    }

    #[test]
    fn canonical_json_drops_the_given_top_level_key() {
        // Arrange
        let json = r#"{"a": 1, "hooks": {"x": 1}}"#;

        // Act
        let got = canonical_json(json, Some("hooks"));

        // Assert
        assert!(!got.contains("hooks"));
        assert!(got.contains("\"a\""));
    }

    #[test]
    fn canonical_json_is_empty_on_invalid_json() {
        // Arrange
        let json = "not json";

        // Act
        let got = canonical_json(json, None);

        // Assert
        assert_eq!(got, "");
    }
}
