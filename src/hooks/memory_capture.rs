// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports hooks/memory-capture.py: a Stop hook that pauses the turn once
//! statusline.sh's `capture-due` marker fires.

//! `{"decision":"block","reason":<text>}` asks the model to write down
//! durable facts before continuing.

//! The marker releases silently once a fact write is detected, otherwise
//! it retains for a bounded re-block; see `REBLOCK_CAP`.
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
use crate::common::session::memory_dir;
use crate::common::{emit_block, session_dir};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

const MAX_PATHS: usize = 5;
/// How many times a no-write invocation retains the marker and re-blocks
/// before it releases unconditionally.
const REBLOCK_CAP: i64 = 2;

const INTRO: &str = "Context usage in this session just crossed the capture threshold. This \
is a good moment to pause, not a problem: write down anything from this \
session worth remembering next time, such as a decision made, a gotcha \
found, or a convention confirmed, using the memory tools or store this \
project keeps. Then continue with the rest of the turn.";

const HANDOFF_NUDGE: &str = "\n\nAlso worth doing now: if this feels like a natural stopping \
point, run /playbook:session-handoff so the next session can pick up without re-reading \
this one.";

/// Stop entry point. A detected write releases the marker silently;
/// otherwise it re-blocks up to `REBLOCK_CAP` times, then fails open.
pub fn run(payload: &Payload) {
    let dir = session_dir(payload);
    if dir.is_empty() {
        return;
    }
    let dir_path = Path::new(&dir);
    let marker = dir_path.join("capture-due");
    let attempts_path = dir_path.join("capture-attempts");

    let marker_mtime = match fs::metadata(&marker) {
        Ok(meta) => match meta.modified() {
            Ok(t) => t,
            Err(_) => return release(&marker, &attempts_path),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(_) => return release(&marker, &attempts_path),
    };

    let graph_path = memory_dir().join("memory.graph.json");
    let write_detected = match fs::metadata(&graph_path) {
        Ok(meta) => match meta.modified() {
            Ok(graph_mtime) => graph_mtime > marker_mtime,
            Err(_) => return release(&marker, &attempts_path),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(_) => return release(&marker, &attempts_path),
    };
    if write_detected {
        return release(&marker, &attempts_path);
    }

    let attempts = match read_attempts(&attempts_path) {
        Some(n) => n,
        None => return release(&marker, &attempts_path),
    };
    if attempts >= REBLOCK_CAP {
        eprintln!("memory-capture: capture skipped, re-block cap reached");
        return release(&marker, &attempts_path);
    }

    let next_attempt = attempts + 1;
    let _ = fs::write(&attempts_path, next_attempt.to_string());

    let edits_path = dir_path.join("edits.jsonl");
    let unique = unique_paths_recent_first(&edits_path);
    emit_block(&build_block_body(&unique, next_attempt));
}

/// Best-effort remove both state files, releasing without blocking. Used by
/// every release path: a detected write, a fail-open error, and the cap.
fn release(marker: &Path, attempts_path: &Path) {
    let _ = fs::remove_file(marker);
    let _ = fs::remove_file(attempts_path);
}

/// Parse `capture-attempts`'s integer contents. Absent means no attempt yet
/// (`Some(0)`); unparseable or unreadable is `None`, the fail-open signal.
fn read_attempts(path: &Path) -> Option<i64> {
    match fs::read_to_string(path) {
        Ok(contents) => contents.trim().parse::<i64>().ok(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Some(0),
        Err(_) => None,
    }
}

/// Build a no-write re-block reason: `INTRO`, the edited-path listing,
/// `HANDOFF_NUDGE`, then a sentence naming this attempt out of `REBLOCK_CAP`.
fn build_block_body(unique: &[String], attempt: i64) -> String {
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
    body.push_str(&format!(
        "\n\nThis is re-block {attempt} of {REBLOCK_CAP} since context usage crossed the \
threshold; still no new memory fact detected."
    ));
    body
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
