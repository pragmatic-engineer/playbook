// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Per-project field reads from a `~/.claude.json`-shaped document, where
//! the project path is a dynamic key (a filesystem path, so it may contain
//! dots and is never split on them), replacing
//! `jq -r --arg p "<path>" '.projects[$p].<field>'` sites.

use serde_json::Value;

/// Reads `.projects[project_path].field` from `json`, where `project_path`
/// is used verbatim as an object key. Empty string on invalid JSON,
/// matching the empty capture a failed `jq` leaves behind once its stderr
/// is redirected away. Otherwise mirrors `jq -r`: a string field prints
/// unquoted, any other JSON value (including a missing field or project
/// entry, both of which jq's null-chaining resolves to `null`) prints its
/// compact JSON text, so a missing entry reads as the literal `"null"`.
pub fn project_field(json: &str, project_path: &str, field: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return String::new();
    };
    let found = value
        .get("projects")
        .and_then(|projects| projects.get(project_path))
        .and_then(|project| project.get(field));
    match found {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_field_reads_a_boolean_field_as_its_raw_text() {
        // Arrange
        let json =
            r#"{"projects": {"/home/x/.config/playbook": {"hasTrustDialogAccepted": true}}}"#;

        // Act
        let got = project_field(json, "/home/x/.config/playbook", "hasTrustDialogAccepted");

        // Assert
        assert_eq!(got, "true");
    }

    #[test]
    fn project_field_reads_a_number_field_as_its_raw_text() {
        // Arrange
        let json = r#"{"projects": {"/home/x/.config/playbook": {"lastCost": 1.23}}}"#;

        // Act
        let got = project_field(json, "/home/x/.config/playbook", "lastCost");

        // Assert
        assert_eq!(got, "1.23");
    }

    #[test]
    fn project_field_does_not_split_a_project_path_containing_dots() {
        // Arrange
        let json =
            r#"{"projects": {"/home/x/.config/playbook": {"hasTrustDialogAccepted": true}}}"#;

        // Act
        let got = project_field(json, "/home/x/.config/other", "hasTrustDialogAccepted");

        // Assert
        assert_eq!(got, "null");
    }

    #[test]
    fn project_field_is_null_when_the_project_entry_is_absent() {
        // Arrange
        let json = r#"{"projects": {}}"#;

        // Act
        let got = project_field(json, "/home/x/.config/playbook", "hasTrustDialogAccepted");

        // Assert
        assert_eq!(got, "null");
    }

    #[test]
    fn project_field_is_empty_on_invalid_json() {
        // Arrange
        let json = "not json";

        // Act
        let got = project_field(json, "/home/x/.config/playbook", "hasTrustDialogAccepted");

        // Assert
        assert_eq!(got, "");
    }
}
