// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Length of one top-level field of a JSON document, replacing
//! `jq '.<field> | length'` sites. jq's `length` means an array's element
//! count or an object's key count, so one function covers both shapes
//! rather than needing a separate `keys | length` variant.

use serde_json::Value;

/// Reads `field` from the JSON object `json` and returns its length: an
/// array's element count, or an object's key count. 0 when `json` is
/// invalid, `field` is missing, or its value is neither an array nor an
/// object.
pub fn field_length(json: &str, field: &str) -> usize {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return 0;
    };
    match value.get(field) {
        Some(Value::Array(a)) => a.len(),
        Some(Value::Object(o)) => o.len(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_length_counts_array_elements() {
        // Arrange
        let json = r#"{"edges": [1, 2, 3]}"#;

        // Act
        let got = field_length(json, "edges");

        // Assert
        assert_eq!(got, 3);
    }

    #[test]
    fn field_length_counts_object_keys() {
        // Arrange
        let json = r#"{"hooks": {"SessionStart": [], "Stop": [], "PreToolUse": []}}"#;

        // Act
        let got = field_length(json, "hooks");

        // Assert
        assert_eq!(got, 3);
    }

    #[test]
    fn field_length_is_zero_when_field_is_missing() {
        // Arrange
        let json = r#"{"other": []}"#;

        // Act
        let got = field_length(json, "edges");

        // Assert
        assert_eq!(got, 0);
    }

    #[test]
    fn field_length_is_zero_on_invalid_json() {
        // Arrange
        let json = "not json";

        // Act
        let got = field_length(json, "edges");

        // Assert
        assert_eq!(got, 0);
    }
}
