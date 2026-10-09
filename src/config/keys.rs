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
    "memory.source",
    "agents.variants",
    "models.haiku",
    "models.sonnet",
    "models.opus",
    "routing.escalate",
    "auto.budgetUsd",
    "auto.warnPct",
    "fix.maxFiles",
    "fix.maxLines",
];

/// The component kinds an `effort.<kind>.<name>` key can name.
pub const EFFORT_COMPONENT_KINDS: [&str; 3] = ["agents", "commands", "skills"];

/// Whether `key` is `effort.<kind>.<name>`: a known kind and a name of
/// lowercase letters, digits and hyphens. Whether the name exists is checked
/// later against the plugin files, never here.
fn is_effort_component_key(key: &str) -> bool {
    let mut parts = key.split('.');
    let (Some("effort"), Some(kind), Some(name), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    EFFORT_COMPONENT_KINDS.contains(&kind)
        && !name.is_empty()
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The default value for a known key, or `None` if `key` is not in
/// `KNOWN_KEYS`.
pub fn default_value(key: &str) -> Option<Value> {
    if is_effort_component_key(key) {
        return Some(Value::String("auto".to_string()));
    }
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
        "memory.source" => Some(Value::String("both".to_string())),
        "agents.variants" => Some(Value::String("auto".to_string())),
        "models.haiku" | "models.sonnet" | "models.opus" => Some(Value::String(String::new())),
        "routing.escalate" => Some(Value::String("ask".to_string())),
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
    if is_effort_component_key(key) {
        return Some(&["auto", "low", "medium", "high", "xhigh", "max"]);
    }
    match key {
        "autoReview.type" => Some(&["quick", "deep", "auto"]),
        "mode" => Some(&["ask", "auto"]),
        "maxEffortLevel" => Some(&["auto", "low", "medium", "high", "xhigh", "max"]),
        "memory.source" => Some(&["both", "playbook"]),
        "agents.variants" => Some(&["auto", "all", "off"]),
        "routing.escalate" => Some(&["ask", "auto", "deny"]),
        _ => None,
    }
}

/// The problem with a `models.<tier>` value, or `None` when it is fine.
/// Fine is empty (no override) or `claude-<tier>-<major>[-<minor>]`, so a
/// `models.sonnet` value can only name a Sonnet model. `None` for other keys.
pub fn model_override_error(key: &str, value: &str) -> Option<String> {
    let tier = key.strip_prefix("models.")?;
    if !["haiku", "sonnet", "opus"].contains(&tier) || value.is_empty() {
        return None;
    }
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let valid = value
        .strip_prefix(&format!("claude-{tier}-"))
        .is_some_and(|rest| {
            let parts: Vec<&str> = rest.split('-').collect();
            (1..=2).contains(&parts.len()) && parts.iter().all(|p| digits(p))
        });
    if valid {
        return None;
    }
    Some(format!(
        "expected empty or claude-{tier}-<major>[-<minor>], for example claude-{tier}-5-5"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_overrides_must_name_a_model_of_their_own_tier() {
        assert_eq!(model_override_error("models.sonnet", ""), None);
        assert_eq!(
            model_override_error("models.sonnet", "claude-sonnet-5-5"),
            None
        );
        assert_eq!(model_override_error("models.opus", "claude-opus-5"), None);
        assert!(model_override_error("models.sonnet", "claude-opus-5").is_some());
        assert!(model_override_error("models.sonnet", "claude-sonnet-x").is_some());
        assert!(model_override_error("models.sonnet", "sonnet").is_some());
        assert!(model_override_error("models.sonnet", "claude-sonnet-5-5-1").is_some());
        assert_eq!(model_override_error("mode", "anything"), None);
    }

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
    fn effort_component_keys_are_a_validated_family() {
        assert!(is_effort_component_key("effort.agents.fact-checker"));
        assert!(is_effort_component_key("effort.commands.deep-review"));
        assert!(is_effort_component_key("effort.skills.writing-style"));
        assert!(!is_effort_component_key("effort.agent.reviewer"));
        assert!(!is_effort_component_key("effort.agents."));
        assert!(!is_effort_component_key("effort.agents.Reviewer"));
        assert!(!is_effort_component_key("effort.agents.a.b"));
        assert!(!is_effort_component_key("effort.agents.-x"));
        assert_eq!(
            default_value("effort.agents.reviewer"),
            Some(Value::String("auto".to_string()))
        );
        assert_eq!(
            allowed_enum_values("effort.skills.x").map(<[&str]>::len),
            Some(6)
        );
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
