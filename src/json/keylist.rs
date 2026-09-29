// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Lists a JSON document's top-level keys, sorted, replacing
//! `jq -r 'keys_unsorted[]' | sort` call sites that snapshot a document's
//! key set rather than its values.

use serde_json::Value;

/// Parses `json` and returns its top-level object keys, sorted
/// lexicographically. Empty when `json` fails to parse, or its top-level
/// value is not an object (no keys to list), matching how the `jq` pipeline
/// this replaces produced no lines on either failure.
pub fn top_level_keys_sorted(json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(map) = value.as_object() else {
        return Vec::new();
    };
    let mut keys: Vec<String> = map.keys().cloned().collect();
    keys.sort();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_keys_sorted_returns_keys_in_lexicographic_order() {
        // Arrange
        let input = r#"{"b": 1, "a": 2, "c": 3}"#;

        // Act
        let got = top_level_keys_sorted(input);

        // Assert
        assert_eq!(got, vec!["a", "b", "c"]);
    }

    #[test]
    fn top_level_keys_sorted_is_empty_on_invalid_json() {
        // Arrange
        let input = "not json";

        // Act
        let got = top_level_keys_sorted(input);

        // Assert
        assert!(got.is_empty());
    }

    #[test]
    fn top_level_keys_sorted_is_empty_when_top_level_value_is_not_an_object() {
        // Arrange
        let input = "[1, 2, 3]";

        // Act
        let got = top_level_keys_sorted(input);

        // Assert
        assert!(got.is_empty());
    }
}
