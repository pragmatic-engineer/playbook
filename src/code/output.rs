// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The JSON envelope every `playbook code` command prints with `--json`.
//! Key order is stable because `serde_json` keeps insertion order.

use serde_json::{json, Map, Value};

/// Schema version of the envelope.
pub const JSON_VERSION: u32 = 1;

/// The state of the index a command read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexState {
    Fresh,
    Stale,
    Missing,
}

impl IndexState {
    pub fn as_str(self) -> &'static str {
        match self {
            IndexState::Fresh => "fresh",
            IndexState::Stale => "stale",
            IndexState::Missing => "missing",
        }
    }
}

/// One command's result.
#[derive(Debug, Clone)]
pub struct Envelope {
    pub command: &'static str,
    pub state: IndexState,
    pub index_path: String,
    pub results: Value,
    pub truncated: bool,
    pub omitted: usize,
    pub warnings: Vec<String>,
    /// Extra counters, printed as `stats` only when present.
    pub stats: Option<Value>,
}

impl Envelope {
    pub fn new(
        command: &'static str,
        state: IndexState,
        index_path: String,
        results: Value,
    ) -> Self {
        Self {
            command,
            state,
            index_path,
            results,
            truncated: false,
            omitted: 0,
            warnings: Vec::new(),
            stats: None,
        }
    }

    /// The document as compact JSON with a trailing newline.
    pub fn to_json(&self) -> String {
        let mut doc = Map::new();
        doc.insert("version".into(), json!(JSON_VERSION));
        doc.insert("command".into(), json!(self.command));
        doc.insert(
            "index".into(),
            json!({"state": self.state.as_str(), "path": self.index_path}),
        );
        doc.insert("results".into(), self.results.clone());
        doc.insert("truncated".into(), json!(self.truncated));
        doc.insert("omitted".into(), json!(self.omitted));
        doc.insert("warnings".into(), json!(self.warnings));
        if let Some(stats) = &self.stats {
            doc.insert("stats".into(), stats.clone());
        }
        let mut text = Value::Object(doc).to_string();
        text.push('\n');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_envelope_has_a_stable_key_order_and_a_trailing_newline() {
        // Arrange
        let envelope = Envelope::new(
            "index",
            IndexState::Fresh,
            "/x/index".into(),
            json!({"indexed": 3}),
        );

        // Act
        let text = envelope.to_json();

        // Assert
        assert_eq!(
            text,
            "{\"version\":1,\"command\":\"index\",\"index\":{\"state\":\"fresh\",\"path\":\"/x/index\"},\"results\":{\"indexed\":3},\"truncated\":false,\"omitted\":0,\"warnings\":[]}\n"
        );
    }
}
