// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Validity check for a JSON document, replacing `jq empty`/`jq -e .` sites
//! that only ever care whether the input parses.

use serde_json::Value;

/// Reports whether `json` parses as JSON. Mirrors `jq empty`/`jq -e .`,
/// which both exit non-zero on malformed input.
pub fn is_valid_json(json: &str) -> bool {
    serde_json::from_str::<Value>(json).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_valid_json_is_true_for_well_formed_json() {
        // Arrange
        let json = r#"{"a": 1}"#;

        // Act
        let got = is_valid_json(json);

        // Assert
        assert!(got);
    }

    #[test]
    fn is_valid_json_is_false_for_malformed_json() {
        // Arrange
        let json = "not json";

        // Act
        let got = is_valid_json(json);

        // Assert
        assert!(!got);
    }

    #[test]
    fn is_valid_json_is_false_for_an_empty_document() {
        // Arrange
        let json = "";

        // Act
        let got = is_valid_json(json);

        // Assert
        assert!(!got);
    }
}
