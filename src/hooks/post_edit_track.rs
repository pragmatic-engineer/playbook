// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Ports the retired shell original: a PostToolUse hook on Edit/Write/
//! NotebookEdit that records the edited absolute path plus a timestamp to
//! the per-session edits.jsonl file. Consumed by preread-edit-check.rs and
//! the statusline. Emits nothing on stdout; that silence is the contract.

use crate::common::payload::Payload;
use crate::common::{abspath, atomic_append, incr_counter, session_dir};
use serde::Serialize;

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
        ts: crate::common::time::now_secs(),
    };
    if let Ok(line) = serde_json::to_string(&record) {
        atomic_append(&format!("{dir}/edits.jsonl"), &line);
    }

    // Bump human-readable edit count (used by statusline).
    incr_counter(&format!("{dir}/edit-count"));
}
