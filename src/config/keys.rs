// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Known playbook config keys, their default values, and their allowed enum
//! values where one exists. Used by the resolution and write modules to
//! reject an unknown key or an out-of-range value.

use serde_json::Value;

/// Every config key playbook resolves, in v1.
pub const KNOWN_KEYS: &[&str] = &[
    "autoReview.enabled",
    "autoReview.type",
    "autoReview.fix",
    "autoMerge.enabled",
    "commit.signOff",
    "pr.draft",
    "worktreeCleanup.enabled",
    "worktreeCleanup.staleAfterDays",
    "worktreeCleanup.conflictGracePeriodDays",
    "mode",
    "maxEffortLevel",
    "auto.budgetUsd",
    "auto.warnPct",
    "fix.maxFiles",
    "fix.maxLines",
];

/// The default value for a known key, or `None` if `key` is not in
/// `KNOWN_KEYS`.
pub fn default_value(key: &str) -> Option<Value> {
    match key {
        "autoReview.enabled" => Some(Value::Bool(true)),
        "autoReview.type" => Some(Value::String("auto".to_string())),
        "autoReview.fix" => Some(Value::Bool(false)),
        "autoMerge.enabled" => Some(Value::Bool(false)),
        "commit.signOff" => Some(Value::Bool(true)),
        "pr.draft" => Some(Value::Bool(true)),
        "worktreeCleanup.enabled" => Some(Value::Bool(true)),
        "worktreeCleanup.staleAfterDays" => Some(Value::Number(30.into())),
        "worktreeCleanup.conflictGracePeriodDays" => Some(Value::Number(90.into())),
        "mode" => Some(Value::String("ask".to_string())),
        "maxEffortLevel" => Some(Value::String("auto".to_string())),
        "auto.budgetUsd" => Some(Value::Number(5.into())),
        "auto.warnPct" => Some(Value::Number(70.into())),
        "fix.maxFiles" => Some(Value::Number(3.into())),
        "fix.maxLines" => Some(Value::Number(500.into())),
        _ => None,
    }
}

/// The fixed set of string values `key` accepts, or `None` if `key` has no
/// enum constraint (including an unknown key).
pub fn allowed_enum_values(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "autoReview.type" => Some(&["quick", "deep", "auto"]),
        "mode" => Some(&["ask", "auto"]),
        "maxEffortLevel" => Some(&["auto", "low", "medium", "high", "xhigh", "max"]),
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
    fn default_value_for_auto_review_type_is_auto() {
        // Arrange
        let key = "autoReview.type";

        // Act
        let result = default_value(key);

        // Assert
        assert_eq!(result, Some(Value::String("auto".to_string())));
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
    fn allowed_enum_values_for_auto_review_type_is_quick_deep_or_auto() {
        // Arrange
        let key = "autoReview.type";

        // Act
        let result = allowed_enum_values(key);

        // Assert
        assert_eq!(result, Some(["quick", "deep", "auto"].as_slice()));
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

    #[test]
    fn pr_draft_defaults_to_true_with_no_enum_constraint() {
        // Arrange
        let key = "pr.draft";

        // Act
        let default = default_value(key);
        let allowed = allowed_enum_values(key);

        // Assert
        assert_eq!(default, Some(Value::Bool(true)));
        assert_eq!(allowed, None);
    }

    #[test]
    fn default_values_for_the_pr_and_commit_settings() {
        // Arrange
        let cases = [
            ("autoReview.fix", false),
            ("autoMerge.enabled", false),
            ("commit.signOff", true),
            ("pr.draft", true),
        ];

        for (key, expected) in cases {
            // Act
            let result = default_value(key);

            // Assert
            assert_eq!(result, Some(Value::Bool(expected)), "{key}");
        }
    }

    #[test]
    fn the_pr_and_commit_settings_are_known_keys_with_no_enum_constraint() {
        // Arrange
        let keys = [
            "autoReview.fix",
            "autoMerge.enabled",
            "commit.signOff",
            "pr.draft",
        ];

        for key in keys {
            // Act
            let known = KNOWN_KEYS.contains(&key);
            let allowed = allowed_enum_values(key);

            // Assert
            assert!(known, "{key} must be listed in KNOWN_KEYS");
            assert_eq!(allowed, None, "{key}");
        }
    }
}
