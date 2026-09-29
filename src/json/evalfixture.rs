// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Backs the `jq` call sites in `shell/eval-review-triage.sh`: validating
//! and counting the fixture array, reading one fixture's `.id`/`.pr`/
//! `.lenses` fields, re-serializing a classifier reply compactly, and
//! reading a per-lens ground-truth fact or classifier tier. Self-contained:
//! duplicates `array_length` and `string_field`'s shape from
//! `src/json/ghjson.rs` rather than importing them, one sibling file per
//! script this module backs.

use serde_json::Value;

/// True for any syntactically valid JSON, matching `jq empty`'s exit-status
/// check (line 89).
pub fn is_valid_json(json: &str) -> bool {
    serde_json::from_str::<Value>(json).is_ok()
}

/// Length of a top-level JSON array, or 0 for anything else: malformed
/// JSON or a non-array top-level value, matching `jq 'length'`'s permissive
/// behavior (line 102).
pub fn array_length(json: &str) -> usize {
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Array(elements)) => elements.len(),
        _ => 0,
    }
}

/// Compact JSON text of the element at `index` in a top-level array, or
/// `None` if the index is out of bounds or the top-level value is not an
/// array, matching `jq -c ".[$i]"` (line 128).
pub fn indexed_element(json: &str, index: usize) -> Option<String> {
    let Ok(Value::Array(elements)) = serde_json::from_str::<Value>(json) else {
        return None;
    };
    let element = elements.get(index)?;
    serde_json::to_string(element).ok()
}

/// Reads one field from a top-level JSON object as display text: a string
/// value as-is, a number in its literal form (`42`, not `"42"`), matching
/// `jq -r '.$key'` (lines 129-130). Empty on any failure: malformed JSON, a
/// non-object top-level value, a missing key, or a value of any other type,
/// matching `src/json/ghjson.rs::field`'s established convention.
pub fn string_field(json: &str, key: &str) -> String {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(json) else {
        return String::new();
    };
    match map.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Keys of `.lenses` sorted alphabetically, matching `jq`'s `keys` builtin
/// (not insertion order). Empty for a missing or empty `.lenses` object, or
/// a malformed/non-object top-level value.
pub fn lens_names(json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(lenses) = value.get("lenses").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut names: Vec<String> = lenses.keys().cloned().collect();
    names.sort();
    names
}

/// [`lens_names`] joined with `", "`, matching `jq -r '.lenses | keys |
/// join(", ")'` (lines 131, 146/183/220's sibling filter).
pub fn lens_names_joined(json: &str) -> String {
    lens_names(json).join(", ")
}

/// Reserializes valid JSON without extra whitespace, or `None` for
/// malformed input, matching `jq -c '.'` (line 173).
pub fn is_valid_json_compact(json: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(json).ok()?;
    serde_json::to_string(&value).ok()
}

/// Raw text of `.lenses[$lens].found`, matching `jq -r
/// '.lenses[$l].found'` (line 196): an ungated filter, so a missing lens
/// key or missing `found` field prints jq's `null`, not an empty string.
/// `-r` only strips the surrounding quotes off a string result; every other
/// type (including `null`) prints its own JSON text.
pub fn lens_found(json: &str, lens: &str) -> String {
    let value = serde_json::from_str::<Value>(json).unwrap_or(Value::Null);
    let found = value
        .get("lenses")
        .and_then(|lenses| lenses.get(lens))
        .and_then(|entry| entry.get("found"))
        .cloned()
        .unwrap_or(Value::Null);
    raw_text(&found)
}

/// Raw text of `.[$lens].tier`, matching `jq -r 'if type == "object" and
/// has($l) then (.[$l].tier // "") else "" end'` (line 197): unlike
/// [`lens_found`], type-checked and `// ""`-guarded, so a non-object
/// top-level value, a missing lens key, or a present-but-null/missing
/// `tier` field all fall back to an empty string rather than jq's `null`.
pub fn lens_tier(json: &str, lens: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return String::new();
    };
    let Some(top) = value.as_object() else {
        return String::new();
    };
    let Some(entry) = top.get(lens) else {
        return String::new();
    };
    match entry.get("tier") {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// `-r`'s raw-text rendering of one JSON value: a string unquoted, every
/// other type (including `null`) as its own JSON text.
fn raw_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_json_is_true_for_a_valid_object() {
        // Arrange
        let json = r#"{"id": "fx-1"}"#;

        // Act
        let got = is_valid_json(json);

        // Assert
        assert!(got);
    }

    #[test]
    fn is_valid_json_is_true_for_a_valid_array() {
        // Arrange
        let json = r#"[1, 2, 3]"#;

        // Act
        let got = is_valid_json(json);

        // Assert
        assert!(got);
    }

    #[test]
    fn is_valid_json_is_false_for_malformed_text() {
        // Arrange
        let json = "not json";

        // Act
        let got = is_valid_json(json);

        // Assert
        assert!(!got);
    }

    #[test]
    fn array_length_matches_the_element_count_directly() {
        // Arrange
        let json = r#"[{"id": "fx-1"}, {"id": "fx-2"}, {"id": "fx-3"}]"#;

        // Act
        let got = array_length(json);

        // Assert
        assert_eq!(got, 3);
    }

    #[test]
    fn array_length_is_zero_for_an_empty_array() {
        // Arrange
        let json = "[]";

        // Act
        let got = array_length(json);

        // Assert
        assert_eq!(got, 0);
    }

    #[test]
    fn array_length_is_zero_for_a_non_array_top_level_value() {
        // Arrange
        let json = r#"{"id": "fx-1"}"#;

        // Act
        let got = array_length(json);

        // Assert
        assert_eq!(got, 0);
    }

    #[test]
    fn array_length_is_zero_for_malformed_json() {
        // Arrange
        let json = "not json";

        // Act
        let got = array_length(json);

        // Assert
        assert_eq!(got, 0);
    }

    #[test]
    fn indexed_element_returns_the_compact_text_of_the_element_at_index() {
        // Arrange
        let json = r#"[{"id": "fx-1"}, {"id":   "fx-2"}]"#;

        // Act
        let got = indexed_element(json, 1);

        // Assert
        assert_eq!(got, Some(r#"{"id":"fx-2"}"#.to_string()));
    }

    #[test]
    fn indexed_element_is_none_when_index_is_out_of_bounds() {
        // Arrange
        let json = r#"[{"id": "fx-1"}]"#;

        // Act
        let got = indexed_element(json, 5);

        // Assert
        assert_eq!(got, None);
    }

    #[test]
    fn indexed_element_is_none_for_a_non_array_top_level_value() {
        // Arrange
        let json = r#"{"id": "fx-1"}"#;

        // Act
        let got = indexed_element(json, 0);

        // Assert
        assert_eq!(got, None);
    }

    #[test]
    fn string_field_reads_a_string_value() {
        // Arrange
        let json = r#"{"id": "fx-1"}"#;

        // Act
        let got = string_field(json, "id");

        // Assert
        assert_eq!(got, "fx-1");
    }

    #[test]
    fn string_field_reads_a_number_value_as_its_literal_text() {
        // Arrange
        let json = r#"{"pr": 4242}"#;

        // Act
        let got = string_field(json, "pr");

        // Assert
        assert_eq!(got, "4242");
    }

    #[test]
    fn string_field_is_empty_when_the_key_is_missing() {
        // Arrange
        let json = r#"{"id": "fx-1"}"#;

        // Act
        let got = string_field(json, "pr");

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn string_field_is_empty_for_malformed_json() {
        // Arrange
        let json = "not json";

        // Act
        let got = string_field(json, "id");

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn lens_names_joined_sorts_alphabetically_and_joins_with_comma_space() {
        // Arrange: source order is security, correctness, opposite of the
        // expected alphabetical output, to prove sorting actually happens.
        let json = r#"{"lenses": {"security": {}, "correctness": {}}}"#;

        // Act
        let got = lens_names_joined(json);

        // Assert
        assert_eq!(got, "correctness, security");
    }

    #[test]
    fn lens_names_joined_handles_a_single_key() {
        // Arrange
        let json = r#"{"lenses": {"correctness": {}}}"#;

        // Act
        let got = lens_names_joined(json);

        // Assert
        assert_eq!(got, "correctness");
    }

    #[test]
    fn lens_names_joined_is_empty_for_an_empty_lenses_object() {
        // Arrange
        let json = r#"{"lenses": {}}"#;

        // Act
        let got = lens_names_joined(json);

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn lens_names_returns_keys_sorted_alphabetically() {
        // Arrange: source order is security, correctness, opposite of the
        // expected alphabetical output, to prove sorting actually happens.
        let json = r#"{"lenses": {"security": {}, "correctness": {}}}"#;

        // Act
        let got = lens_names(json);

        // Assert
        assert_eq!(got, vec!["correctness".to_string(), "security".to_string()]);
    }

    #[test]
    fn lens_names_is_empty_for_an_empty_lenses_object() {
        // Arrange
        let json = r#"{"lenses": {}}"#;

        // Act
        let got = lens_names(json);

        // Assert
        assert!(got.is_empty());
    }

    #[test]
    fn is_valid_json_compact_reserializes_without_extra_whitespace() {
        // Arrange
        let json = r#"{  "a" : 1 , "b" : [1,  2,   3]  }"#;

        // Act
        let got = is_valid_json_compact(json);

        // Assert
        assert_eq!(got, Some(r#"{"a":1,"b":[1,2,3]}"#.to_string()));
    }

    #[test]
    fn is_valid_json_compact_is_none_for_malformed_json() {
        // Arrange
        let json = "not json";

        // Act
        let got = is_valid_json_compact(json);

        // Assert
        assert_eq!(got, None);
    }

    #[test]
    fn lens_found_reads_a_true_found_field_as_true_text() {
        // Arrange
        let json = r#"{"lenses": {"correctness": {"found": true}}}"#;

        // Act
        let got = lens_found(json, "correctness");

        // Assert
        assert_eq!(got, "true");
    }

    #[test]
    fn lens_found_reads_a_false_found_field_as_false_text() {
        // Arrange
        let json = r#"{"lenses": {"correctness": {"found": false}}}"#;

        // Act
        let got = lens_found(json, "correctness");

        // Assert
        assert_eq!(got, "false");
    }

    #[test]
    fn lens_found_is_the_literal_null_text_when_the_lens_key_is_missing() {
        // Arrange: verified against a real `jq -r` invocation, this ungated
        // filter prints jq's null as the text "null", not an empty string.
        let json = r#"{"lenses": {"security": {"found": true}}}"#;

        // Act
        let got = lens_found(json, "correctness");

        // Assert
        assert_eq!(got, "null");
    }

    #[test]
    fn lens_found_is_the_literal_null_text_when_the_found_field_is_missing() {
        // Arrange: verified against a real `jq -r` invocation, this ungated
        // filter prints jq's null as the text "null", not an empty string.
        let json = r#"{"lenses": {"correctness": {}}}"#;

        // Act
        let got = lens_found(json, "correctness");

        // Assert
        assert_eq!(got, "null");
    }

    #[test]
    fn lens_tier_reads_the_tier_field_for_a_present_lens_key() {
        // Arrange
        let json = r#"{"correctness": {"tier": "full-lens"}}"#;

        // Act
        let got = lens_tier(json, "correctness");

        // Assert
        assert_eq!(got, "full-lens");
    }

    #[test]
    fn lens_tier_is_empty_when_the_top_level_value_is_not_an_object() {
        // Arrange
        let json = r#"["correctness", "security"]"#;

        // Act
        let got = lens_tier(json, "correctness");

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn lens_tier_is_empty_when_the_named_lens_key_is_missing() {
        // Arrange
        let json = r#"{"security": {"tier": "skip"}}"#;

        // Act
        let got = lens_tier(json, "correctness");

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn lens_tier_is_empty_when_the_tier_field_is_missing() {
        // Arrange
        let json = r#"{"correctness": {}}"#;

        // Act
        let got = lens_tier(json, "correctness");

        // Assert
        assert_eq!(got, "");
    }
}
