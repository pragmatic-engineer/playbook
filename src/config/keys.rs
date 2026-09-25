// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Known playbook config keys, their default values, and their allowed enum
//! values where one exists. Used by the resolution and write modules to
//! reject an unknown key or an out-of-range value.

use serde_json::Value;

/// Every config key playbook resolves, in v1.
pub const KNOWN_KEYS: &[&str] = &["autoReview.enabled", "autoReview.type"];

/// The default value for a known key, or `None` if `key` is not in
/// `KNOWN_KEYS`.
pub fn default_value(key: &str) -> Option<Value> {
    match key {
        "autoReview.enabled" => Some(Value::Bool(true)),
        "autoReview.type" => Some(Value::String("deep".to_string())),
        _ => None,
    }
}

/// The fixed set of string values `key` accepts, or `None` if `key` has no
/// enum constraint (including an unknown key).
pub fn allowed_enum_values(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "autoReview.type" => Some(&["quick", "deep"]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_value_for_auto_review_enabled_is_true() {
        // Arrange
        let key = "autoReview.enabled";

        // Act
        let result = default_value(key);

        // Assert
        assert_eq!(result, Some(Value::Bool(true)));
    }

    #[test]
    fn default_value_for_auto_review_type_is_deep() {
        // Arrange
        let key = "autoReview.type";

        // Act
        let result = default_value(key);

        // Assert
        assert_eq!(result, Some(Value::String("deep".to_string())));
    }

    #[test]
    fn default_value_for_unknown_key_is_none() {
        // Arrange
        let key = "notAKnownKey";

        // Act
        let result = default_value(key);

        // Assert
        assert_eq!(result, None);
    }

    #[test]
    fn allowed_enum_values_for_auto_review_type_is_quick_or_deep() {
        // Arrange
        let key = "autoReview.type";

        // Act
        let result = allowed_enum_values(key);

        // Assert
        assert_eq!(result, Some(["quick", "deep"].as_slice()));
    }

    #[test]
    fn allowed_enum_values_for_auto_review_enabled_is_none() {
        // Arrange
        let key = "autoReview.enabled";

        // Act
        let result = allowed_enum_values(key);

        // Assert
        assert_eq!(result, None);
    }

    #[test]
    fn allowed_enum_values_for_unknown_key_is_none() {
        // Arrange
        let key = "notAKnownKey";

        // Act
        let result = allowed_enum_values(key);

        // Assert
        assert_eq!(result, None);
    }
}
