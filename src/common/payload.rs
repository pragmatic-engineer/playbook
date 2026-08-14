// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Dotted-path field extraction over the hook input JSON payload. Replaces
//! `field()` (hooks/lib/common.py:54) and `hi_field` (hooks/lib/common.sh:29).
//! A missing field, a non-object payload, or malformed JSON all resolve to
//! an empty string rather than an error; hooks must never break Claude Code.

use serde_json::Value;

/// The parsed hook input JSON, cached once so every field lookup reads the
/// same value. Mirrors common.py's module-load-time `_payload` cache.
pub struct Payload(Value);

impl Payload {
    /// Parse raw hook input into a `Payload`. Empty input, invalid JSON, and
    /// JSON that is not an object all produce an empty payload. Never panics.
    pub fn parse(raw: &str) -> Self {
        if raw.is_empty() {
            return Self(Value::Object(serde_json::Map::new()));
        }
        match serde_json::from_str::<Value>(raw) {
            Ok(value) if value.is_object() => Self(value),
            _ => Self(Value::Object(serde_json::Map::new())),
        }
    }

    /// Extract a field by jq-style dotted path (e.g. `.tool_input.file_path`).
    /// Returns an empty string for a missing or null field. Strings come back
    /// as-is; booleans as `true`/`false`; whole numbers without a trailing
    /// `.0`; objects and arrays as compact JSON. Never panics.
    pub fn field(&self, path: &str) -> String {
        let stripped = path.trim_start_matches('.');
        if stripped.is_empty() {
            return String::new();
        }
        let mut current = &self.0;
        for key in stripped.split('.') {
            match current.as_object().and_then(|object| object.get(key)) {
                Some(value) => current = value,
                None => return String::new(),
            }
        }
        stringify(current)
    }
}

fn stringify(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => (if *b { "true" } else { "false" }).to_string(),
        Value::String(s) => s.clone(),
        Value::Number(n) => stringify_number(n),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn stringify_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    if let Some(f) = n.as_f64() {
        if f.is_finite() && f.fract() == 0.0 {
            return (f as i64).to_string();
        }
        return f.to_string();
    }
    n.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_string_value() {
        // Arrange
        let payload = Payload::parse(r#"{"session_id":"abc","tool_input":{"file_path":"/tmp/x"}}"#);

        // Act
        let got = payload.field(".session_id");

        // Assert
        assert_eq!(got, "abc");
    }

    #[test]
    fn field_nested_path() {
        // Arrange
        let payload = Payload::parse(r#"{"session_id":"abc","tool_input":{"file_path":"/tmp/x"}}"#);

        // Act
        let got = payload.field(".tool_input.file_path");

        // Assert
        assert_eq!(got, "/tmp/x");
    }

    #[test]
    fn field_missing_key_returns_empty_string() {
        // Arrange
        let payload = Payload::parse(r#"{"session_id":"abc"}"#);

        // Act
        let got = payload.field(".missing");

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn field_boolean_true() {
        // Arrange
        let payload = Payload::parse(r#"{"flag":true}"#);

        // Act
        let got = payload.field(".flag");

        // Assert
        assert_eq!(got, "true");
    }

    #[test]
    fn field_integer_number() {
        // Arrange
        let payload = Payload::parse(r#"{"n":42}"#);

        // Act
        let got = payload.field(".n");

        // Assert
        assert_eq!(got, "42");
    }

    #[test]
    fn field_object_returns_compact_json() {
        // Arrange
        let payload = Payload::parse(r#"{"obj":{"k":"v"}}"#);

        // Act
        let got = payload.field(".obj");

        // Assert
        assert_eq!(got, r#"{"k":"v"}"#);
    }

    #[test]
    fn parse_empty_input_yields_empty_fields() {
        // Arrange
        let payload = Payload::parse("");

        // Act
        let got = payload.field(".session_id");

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn parse_truncated_json_never_panics_and_yields_empty_fields() {
        // Arrange
        let payload = Payload::parse(r#"{"session_id":"abc","tool_input":{"#);

        // Act
        let got = payload.field(".session_id");

        // Assert
        assert_eq!(got, "");
    }

    #[test]
    fn parse_non_object_json_yields_empty_fields() {
        // Arrange
        let payload = Payload::parse(r#"["a","b"]"#);

        // Act
        let got = payload.field(".0");

        // Assert
        assert_eq!(got, "");
    }
}
