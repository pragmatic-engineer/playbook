// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Re-read guard, part of the `preread-size-check` hook. A full Read of a
//! file this session already read in full, with the same modification time
//! and size, and with no compaction since, only repeats text that is still
//! in context. It is denied with a short note naming the way to read a part.
//!
//! Per-session state is `read-state.jsonl` in the session runtime dir, one
//! `{"path","mtime_ns","size","lines"}` record per full Read, newest last.
//! `session-init` zeroes the file on every SessionStart, which includes the
//! `compact` source, so a compaction always starts from a clean slate and the
//! next Read is allowed.
//!
//! Conservative by construction, a Read is never denied when it carries
//! `offset`, `limit` or `pages`, when the target is an image, PDF or
//! notebook, when it comes from a subagent (its context is not the parent's),
//! when the file changed, or when the earlier read was partial (more than
//! `READ_LINE_CAP` lines, so the harness truncated it, or over the size
//! guard limits). Only files of at least `MIN_LINES` lines are denied, since
//! a short file costs less than the retry.

use super::preread_size_check::{is_allowlisted, BYTE_LIMIT, LINE_LIMIT};
use crate::common::payload::Payload;
use crate::common::{abspath, atomic_append, emit_pre_deny, session_dir};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Lines the Read tool returns without an explicit limit. A longer file was
/// truncated, so reading it again with a range can reveal new content.
const READ_LINE_CAP: u64 = 2000;
/// Smallest file worth denying; below this the retry costs about as much.
const MIN_LINES: u64 = 150;
const STATE_FILE: &str = "read-state.jsonl";
/// Extensions the Read tool renders or pages itself, never deduplicated.
const EXEMPT_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "pdf", "svg", "ipynb"];

#[derive(Serialize, Deserialize, PartialEq)]
struct ReadRecord {
    path: String,
    mtime_ns: u128,
    size: u64,
    lines: u64,
}

/// Returns true when the Read was denied (the deny was already emitted).
/// Records the read otherwise. Never panics; any failure allows the Read.
pub(crate) fn guard(payload: &Payload, file_path: &Path) -> bool {
    if !payload.field(".agent_id").is_empty()
        || !payload.field(".tool_input.offset").is_empty()
        || !payload.field(".tool_input.limit").is_empty()
        || !payload.field(".tool_input.pages").is_empty()
        || is_exempt(file_path)
    {
        return false;
    }
    let dir = session_dir(payload);
    if dir.is_empty() {
        return false;
    }
    let Some(current) = snapshot(file_path) else {
        return false;
    };
    let state = format!("{dir}/{STATE_FILE}");
    match last_record(&state, &current.path) {
        Some(prev) if prev == current => {
            if current.lines < MIN_LINES {
                return false;
            }
            emit_pre_deny(&format!(
                "Already read in full this session and unchanged ({} lines). The text is still in \
                 context, so use Grep to find a part, or Read with offset and limit for a range. \
                 To force a full re-read, call Read again with offset:1 and limit:{}.",
                current.lines, current.lines
            ));
            true
        }
        _ => {
            if let Ok(line) = serde_json::to_string(&current) {
                atomic_append(&state, &line);
            }
            false
        }
    }
}

fn is_exempt(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            EXEMPT_EXTENSIONS
                .iter()
                .any(|e| ext.eq_ignore_ascii_case(e))
        })
}

/// The file's identity, or None when it is not a plain full-readable text
/// file the harness would return whole (so it is never recorded or denied).
fn snapshot(file_path: &Path) -> Option<ReadRecord> {
    let meta = fs::metadata(file_path).ok()?;
    let size = meta.len();
    if size > BYTE_LIMIT {
        return None;
    }
    let mtime_ns = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let bytes = fs::read(file_path).ok()?;
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count() as u64;
    let unterminated = u64::from(bytes.last().is_some_and(|&b| b != b'\n'));
    let lines = newlines + unterminated;
    let base = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if lines > READ_LINE_CAP || (lines > LINE_LIMIT && !is_allowlisted(base)) {
        return None;
    }
    Some(ReadRecord {
        path: abspath(&file_path.to_string_lossy()),
        mtime_ns,
        size,
        lines,
    })
}

fn last_record(state: &str, path: &str) -> Option<ReadRecord> {
    let contents = fs::read_to_string(state).ok()?;
    contents
        .lines()
        .filter_map(|line| serde_json::from_str::<ReadRecord>(line).ok())
        .rfind(|rec| rec.path == path)
}
