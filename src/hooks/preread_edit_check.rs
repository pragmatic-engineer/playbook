// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports hooks/preread-edit-check.py: a PreToolUse hook on Read that injects
//! a "do not re-read" reminder when the target was edited by this session
//! via Edit/Write within the last `WINDOW_SECS` seconds. Emits
//! `additionalContext` only; never blocks.

use crate::common::payload::Payload;
use crate::common::{abspath, emit_pre_context, session_dir};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Matches WINDOW in hooks/preread-edit-check.py:18 (30 minutes).
const WINDOW_SECS: i64 = 1800;

pub fn run(payload: &Payload) {
    let dir = session_dir(payload);
    if dir.is_empty() {
        return;
    }

    let edits_path = Path::new(&dir).join("edits.jsonl");
    match fs::metadata(&edits_path) {
        Ok(meta) if meta.len() > 0 => {}
        _ => return,
    }

    let path = payload.field(".tool_input.file_path");
    if path.is_empty() {
        return;
    }
    let abs_path = abspath(&path);
    let now = now_secs();

    let Some(match_ts) = most_recent_match(&edits_path, &abs_path, now) else {
        return;
    };

    let ago = format_ago(now - match_ts);
    let msg = format!(
        "You edited this file {ago} via Edit/Write. Your context already reflects the \
         post-edit state. Re-reading it now is wasted tokens unless you suspect \
         external modifications. Skip the Read and proceed."
    );
    emit_pre_context("PreToolUse", &msg);
}

/// Scan `edits.jsonl` for the most recent record matching `abs_path` within
/// the window, returning its timestamp. Records are `{"path":..,"ts":..}`
/// JSON lines, one per Edit/Write.
///
/// Oddity preserved from hooks/preread-edit-check.py:42-54: a line that
/// parses as valid JSON but is not an object (`rec.get(...)` on a python
/// list or scalar raises, escaping the inner try and hitting the outer
/// `except Exception: return 0`) aborts the whole scan silently, discarding
/// any match already found on an earlier line. A malformed (non-JSON) line
/// is skipped instead, since python's inner try/except catches only the
/// `json.loads` call.
fn most_recent_match(edits_path: &Path, abs_path: &str, now: i64) -> Option<i64> {
    let contents = fs::read_to_string(edits_path).ok()?;
    let mut match_ts: Option<i64> = None;

    for raw in contents.lines() {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let record = value.as_object()?;
        let record_path = record.get("path").and_then(|v| v.as_str());
        let record_ts = record.get("ts").and_then(|v| v.as_i64()).unwrap_or(0);
        if record_path == Some(abs_path) && (now - record_ts) < WINDOW_SECS {
            match_ts = Some(record_ts);
        }
    }

    match_ts
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Matches the age-string thresholds in hooks/preread-edit-check.py:60-66.
fn format_ago(delta: i64) -> String {
    if delta < 60 {
        format!("{delta}s ago")
    } else if delta < 3600 {
        format!("{}m ago", delta / 60)
    } else {
        format!("{}h ago", delta / 3600)
    }
}
