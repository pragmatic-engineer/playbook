// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Reads a JSONL Claude Code session transcript, one JSON object per line.
//! Both functions here operate line by line so a change to one line never
//! touches the bytes of any other.

use regex::Regex;
use serde_json::Value;

/// Drops every line whose `type` is `"system"` and whose `content` (empty
/// when the field is missing) matches `pattern`, passing every other line
/// through byte-for-byte unchanged. A line that fails to parse as JSON is
/// dropped rather than passed through: unlike `rewrite_session_id`, this
/// function's whole purpose is deciding which lines survive, so a line whose
/// shape cannot even be inspected cannot be trusted to keep. An invalid
/// `pattern` regex is treated as matching nothing, so the input passes
/// through unchanged rather than being silently emptied.
pub fn filter_system_lines(jsonl: &str, pattern: &str) -> String {
    let Ok(re) = Regex::new(pattern) else {
        return jsonl.to_string();
    };

    jsonl
        .lines()
        .filter(|line| {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                return false;
            };
            if value.get("type").and_then(Value::as_str) != Some("system") {
                return true;
            }
            let content = value.get("content").and_then(Value::as_str).unwrap_or("");
            !re.is_match(content)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rewrites every line's string `sessionId` field to `new_sid`, preserving
/// each object's original key order. A line with no `sessionId` field, a
/// non-string `sessionId` (`null`, `false`, and so on), or one that fails to
/// parse as JSON passes through byte-for-byte unchanged: this function only
/// ever changes a value already known to be a session id, never a line's
/// shape.
pub fn rewrite_session_id(jsonl: &str, new_sid: &str) -> String {
    jsonl
        .lines()
        .map(|line| rewrite_line(line, new_sid))
        .collect::<Vec<_>>()
        .join("\n")
}

fn rewrite_line(line: &str, new_sid: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(line) else {
        return line.to_string();
    };
    let Some(obj) = value.as_object_mut() else {
        return line.to_string();
    };
    match obj.get("sessionId") {
        Some(Value::String(_)) => {
            obj.insert("sessionId".to_string(), Value::String(new_sid.to_string()));
            value.to_string()
        }
        _ => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_system_lines_drops_a_matching_system_line_keeps_others() {
        // Arrange
        let matching_system = r#"{"type":"system","content":"contains unwanted text"}"#;
        let non_matching_system = r#"{"type":"system","content":"safe content"}"#;
        let non_system = r#"{"type":"user","content":"unwanted stuff too"}"#;
        let input = format!("{matching_system}\n{non_matching_system}\n{non_system}");

        // Act
        let got = filter_system_lines(&input, "unwanted");

        // Assert
        let expected = format!("{non_matching_system}\n{non_system}");
        assert_eq!(got, expected);
    }

    #[test]
    fn filter_system_lines_keeps_all_non_system_lines_unchanged() {
        // Arrange
        let line1 = r#"{"type":"user","content":"hello"}"#;
        let line2 = r#"{"type":"assistant","content":"world"}"#;
        let input = format!("{line1}\n{line2}");

        // Act
        let got = filter_system_lines(&input, "unwanted");

        // Assert
        assert_eq!(got, input);
    }

    #[test]
    fn filter_system_lines_treats_missing_content_as_empty_and_keeps_the_line() {
        // Arrange
        let line = r#"{"type":"system"}"#;

        // Act
        let got = filter_system_lines(line, "unwanted");

        // Assert
        assert_eq!(got, line);
    }

    #[test]
    fn filter_system_lines_skips_a_malformed_json_line_without_panicking() {
        // Arrange
        let line1 = r#"{"type":"user","content":"fine"}"#;
        let malformed = "{not valid json";
        let line3 = r#"{"type":"system","content":"also fine"}"#;
        let input = format!("{line1}\n{malformed}\n{line3}");

        // Act
        let got = filter_system_lines(&input, "unwanted");

        // Assert
        let expected = format!("{line1}\n{line3}");
        assert_eq!(got, expected);
    }

    #[test]
    fn rewrite_session_id_rewrites_an_existing_session_id() {
        // Arrange
        let line = r#"{"sessionId":"abc123","type":"user"}"#;

        // Act
        let got = rewrite_session_id(line, "new-session-999");

        // Assert
        let expected = r#"{"sessionId":"new-session-999","type":"user"}"#;
        assert_eq!(got, expected);
    }

    #[test]
    fn rewrite_session_id_passes_through_a_line_with_no_session_id_field() {
        // Arrange
        let line = r#"{"type":"user","content":"hi"}"#;

        // Act
        let got = rewrite_session_id(line, "new-session-999");

        // Assert
        assert_eq!(got, line);
    }

    #[test]
    fn rewrite_session_id_does_not_rewrite_a_null_session_id() {
        // Arrange
        let line = r#"{"sessionId":null,"type":"user"}"#;

        // Act
        let got = rewrite_session_id(line, "new-session-999");

        // Assert
        assert_eq!(got, line);
    }

    #[test]
    fn rewrite_session_id_does_not_rewrite_a_false_session_id() {
        // Arrange
        let line = r#"{"sessionId":false,"type":"user"}"#;

        // Act
        let got = rewrite_session_id(line, "new-session-999");

        // Assert
        assert_eq!(got, line);
    }
}
