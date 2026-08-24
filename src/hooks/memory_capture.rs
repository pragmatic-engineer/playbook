// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports hooks/memory-capture.py: a Stop hook that, when statusline.sh has
//! dropped a `capture-due` marker in the session dir, pauses the turn with
//! `{"decision":"block","reason":<text>}` asking the model to write down
//! durable facts before continuing. It fires once per threshold crossing:
//! the marker is consumed (deleted) as soon as it is seen, before the
//! reason text is even built, so a run that fails partway through still
//! leaves the marker gone rather than stuck blocking every future turn.
//!
//! Two divergences from the python source, both driven by the "never panic"
//! rule in SEGMENT-B-RULES.md rather than a behaviour choice:
//!
//! 1. python's `rec.get("path")` appends whatever JSON value sits at
//!    `"path"`, including a non-string one, and only breaks later (an
//!    uncaught `TypeError`) when the reason text is assembled with
//!    `"- " + p`. This port instead only accepts a string `"path"` field and
//!    silently skips anything else, so a malformed `edits.jsonl` line can
//!    never crash the Stop hook.
//!
//! 2. python wraps the whole per-line scan in one outer `try`, with only
//!    `json.loads` inside an inner `try`. A line that parses as valid JSON
//!    but is not an object (a bare number, string, array, bool, or null)
//!    makes `rec.get("path")` raise `AttributeError`, which only the outer
//!    `try` catches, so the function returns an EMPTY list, discarding
//!    every path already collected and skipping every later line too. This
//!    port's `serde_json::Value::get` returns `None` for a non-object value
//!    instead of raising, so it skips just that one line and keeps
//!    scanning. This is deliberate, not merely tolerated: a hook must never
//!    lose data it already has (paths gathered from earlier, valid lines)
//!    because one line further down the log happens to be malformed. Note
//!    that a corrupted `edits.jsonl` only happens if something other than
//!    this toolkit's own trusted writer touches the file.

use crate::common::payload::Payload;
use crate::common::{emit_block, session_dir};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

const MAX_PATHS: usize = 5;

const INTRO: &str = "Context usage in this session just crossed the capture threshold. This \
is a good moment to pause, not a problem: write down anything from this \
session worth remembering next time, such as a decision made, a gotcha \
found, or a convention confirmed, using the memory tools or store this \
project keeps. Then continue with the rest of the turn.";

const HANDOFF_NUDGE: &str = "\n\nAlso worth doing now: if this feels like a natural stopping \
point, run /playbook:session-handoff so the next session can pick up without re-reading \
this one.";

const OUTRO: &str = "\n\nThis prompt fires once per threshold crossing, so it will not \
interrupt the next turn unless usage climbs past the threshold again.";

/// Stop entry point. Never panics: a missing session id, a missing marker,
/// or a missing/malformed edit log all resolve to either silence or a
/// best-effort reason text.
pub fn run(payload: &Payload) {
    let dir = session_dir(payload);
    if dir.is_empty() {
        return;
    }
    let dir_path = Path::new(&dir);

    let marker = dir_path.join("capture-due");
    if !marker.is_file() {
        return;
    }
    // Consume the marker before building the reason text: a marker that
    // survives a later failure would block every turn after this one.
    let _ = fs::remove_file(&marker);

    let edits_path = dir_path.join("edits.jsonl");
    let unique = unique_paths_recent_first(&edits_path);

    let mut body = String::from(INTRO);
    if !unique.is_empty() {
        let total = unique.len();
        let listed = &unique[..total.min(MAX_PATHS)];
        let more = total.saturating_sub(MAX_PATHS);

        let path_lines: Vec<String> = listed.iter().map(|p| format!("- {p}")).collect();
        body.push_str(
            "\n\nFiles edited this session, most recent first, worth checking \
for capture worthy facts:\n",
        );
        body.push_str(&path_lines.join("\n"));
        if more > 0 {
            body.push_str(&format!("\n...and {more} more not shown."));
        }
    }
    body.push_str(HANDOFF_NUDGE);
    body.push_str(OUTRO);

    emit_block(&body);
}

/// Unique edited paths from `edits.jsonl`, most recently edited first.
/// Ports `_unique_paths_recent_first` (hooks/memory-capture.py:85): reverse
/// the append log, keep the first occurrence of each path. Never panics; a
/// missing, empty, or unreadable file yields an empty list, and any line
/// that fails to parse as JSON or lacks a string `"path"` is skipped.
fn unique_paths_recent_first(path: &Path) -> Vec<String> {
    let Ok(metadata) = fs::metadata(path) else {
        return Vec::new();
    };
    if metadata.len() == 0 {
        return Vec::new();
    }
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };

    let mut paths = Vec::new();
    for raw in contents.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if let Some(p) = record.get("path").and_then(Value::as_str) {
            paths.push(p.to_string());
        }
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for p in paths.into_iter().rev() {
        if seen.insert(p.clone()) {
            out.push(p);
        }
    }
    out
}
