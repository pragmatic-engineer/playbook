// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports hooks/post-edit-track.py: a PostToolUse hook on Edit/Write/
//! NotebookEdit that records the edited absolute path plus a timestamp to
//! the per-session edits.jsonl file. Consumed by preread-edit-check.rs and
//! the statusline. Emits nothing on stdout; that silence is the contract.

use crate::common::payload::Payload;
use crate::common::{abspath, atomic_append, incr_counter, session_dir};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
struct EditRecord<'a> {
    path: &'a str,
    ts: i64,
}

pub fn run(payload: &Payload) {
    let dir = session_dir(payload);
    if dir.is_empty() {
        return;
    }

    let tool = payload.field(".tool_name");
    if tool != "Edit" && tool != "Write" && tool != "NotebookEdit" {
        return;
    }

    // Different tools use different field names; try both common ones.
    let mut path = payload.field(".tool_input.file_path");
    if path.is_empty() {
        path = payload.field(".tool_input.notebook_path");
    }
    if path.is_empty() {
        return;
    }

    let abs_path = abspath(&path);
    let record = EditRecord {
        path: &abs_path,
        ts: now_unix_seconds(),
    };
    if let Ok(line) = serde_json::to_string(&record) {
        atomic_append(&format!("{dir}/edits.jsonl"), &line);
    }

    // Bump human-readable edit count (used by statusline).
    incr_counter(&format!("{dir}/edit-count"));
}

/// Current unix time in whole seconds, matching python's `int(time.time())`
/// truncation. Never panics; a clock reading before the epoch falls back to
/// 0 rather than erroring.
fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
