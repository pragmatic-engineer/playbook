// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Hook `.command` strings scoped to one `settings.json`-shaped `.hooks`
//! event, replacing `jq '.hooks.<event>[]?.hooks[]?.command'` sites. Unlike
//! `doctor::field::hook_commands`, which flattens every event, this walks
//! only the one named event key: some call sites need a regex match counted
//! within a single event (PreToolUse guards), not across the whole file.

use serde_json::Value;

/// Every hook `.command` string nested under `json`'s `.hooks.<event>`,
/// across every matcher group for that one event. Empty on any failure:
/// invalid JSON, `.hooks` missing or not an object, or `event` absent from
/// it.
pub fn hook_commands_for_event(json: &str, event: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(groups) = value
        .get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for group in groups {
        let Some(group_hooks) = group.get("hooks").and_then(Value::as_array) else {
            continue;
        };
        for hook in group_hooks {
            if let Some(command) = hook.get("command").and_then(Value::as_str) {
                out.push(command.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_commands_for_event_collects_across_matcher_groups_for_one_event() {
        // Arrange
        let json = r#"{
            "hooks": {
                "PreToolUse": [
                    {"matcher": "Write", "hooks": [{"command": "rm-workspace-guard"}]},
                    {"matcher": "Bash", "hooks": [{"command": "bg-await-guard"}]}
                ],
                "Stop": [
                    {"hooks": [{"command": "playbook hook session-init"}]}
                ]
            }
        }"#;

        // Act
        let got = hook_commands_for_event(json, "PreToolUse");

        // Assert
        assert_eq!(got, vec!["rm-workspace-guard", "bg-await-guard"]);
    }

    #[test]
    fn hook_commands_for_event_is_empty_when_event_is_absent() {
        // Arrange
        let json = r#"{"hooks": {"Stop": []}}"#;

        // Act
        let got = hook_commands_for_event(json, "PreToolUse");

        // Assert
        assert!(got.is_empty());
    }

    #[test]
    fn hook_commands_for_event_is_empty_on_invalid_json() {
        // Arrange
        let json = "not json";

        // Act
        let got = hook_commands_for_event(json, "PreToolUse");

        // Assert
        assert!(got.is_empty());
    }
}
