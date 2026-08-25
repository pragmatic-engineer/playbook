// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports hooks/precompact-warn.py: a PreCompact hook that logs the
//! compaction event and warns the user, since PreCompact has no
//! `additionalContext` channel to speak to Claude directly.
//!
//! Two divergences from the python source, both non-observable:
//! - `hooks/lib/common.py` exposes a module-private `RUNTIME_ROOT`
//!   (`$HOME/.claude/runtime`), and `src/common/session.rs` computes the
//!   same path but keeps it private to that module (SEGMENT-B-RULES.md
//!   forbids editing `src/common/**`), so this file re-derives the same two
//!   path segments locally rather than reusing a shared constant.
//! - The python hook appends to the log with a bare `open(log, "a")`,
//!   skipping its own `atomic_append` helper (which the hook itself never
//!   imports for this call). This port uses `common::atomic_append`
//!   instead, since `src/common/mod.rs` documents it as the one
//!   implementation every hook should share; the on-disk line format is
//!   identical either way.
//!
//! Local system time (via the `date` command, mirroring how
//! `common::repo_slug` already shells out to `git`) stands in for python's
//! `time.strftime`, which also renders in the local timezone. `std` alone
//! has no timezone database to do this without shelling out.

use crate::common::atomic::with_dir_lock;
use crate::common::payload::Payload;
use crate::common::{emit_system_message, home_dir, run_with_timeout, session_id};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const LOG_LINE_CAP: usize = 500;

/// How long to wait for the `date` command before giving up. Matches the
/// same `timeout=5` used for every other shelled-out call in
/// hooks/lib/common.py.
const DATE_TIMEOUT: Duration = Duration::from_secs(5);

/// PreCompact entry point. Never panics: a failed log write or a failed
/// timestamp lookup still emits the user-facing warning.
pub fn run(payload: &Payload) {
    let trigger = payload.field(".trigger");
    let sid = session_id(payload);
    let ts = current_timestamp();
    let log_trigger = if trigger.is_empty() {
        "unknown"
    } else {
        trigger.as_str()
    };
    let line = format!("{ts}\tsession={sid}\ttrigger={log_trigger}");

    let log_path = runtime_root().join("compactions.log");
    if let Some(log_path_str) = log_path.to_str() {
        // The append and the trim must serialize together, not just each
        // against itself: `common::atomic_append` releases its own lock the
        // moment it returns, so calling it and then `cap_lines` separately
        // leaves a window where another session's append lands in between,
        // and this session's trim (reading the file before that append,
        // writing after) silently drops the line that just landed. Sharing
        // one `with_dir_lock` acquisition across both closes that window.
        // This inlines the raw append instead of calling `atomic_append`,
        // since nesting two acquisitions of the same lock path from one
        // process would make the inner one retry its full budget pointlessly
        // before failing open, the lock isn't reentrant. `atomic_append`
        // also created the log's parent directory before writing; do the
        // same here, since a fresh runtime dir has nothing under it yet.
        if let Some(parent) = log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let lock_path = PathBuf::from(format!("{log_path_str}.lock"));
        let (acquired, ()) = with_dir_lock(&lock_path, 50, Duration::from_millis(10), || {
            append_line_unlocked(&log_path, &line);
            cap_lines(&log_path, LOG_LINE_CAP);
        });
        if acquired {
            let _ = fs::remove_dir(&lock_path);
        }
    }

    let message_trigger = if trigger.is_empty() {
        "auto"
    } else {
        trigger.as_str()
    };
    let user_msg = format!(
        "\u{26a0} Context compaction triggered ({message_trigger}). After this point, every turn \
replays a lossy summary instead of the original transcript, so the cache \
savings are gone. Strongly consider: finish the current step, ask me to \
wrap up (a session handoff), then /clear for a fresh session."
    );
    emit_system_message(&user_msg);
}

/// `$HOME/.claude/runtime`, matching `RUNTIME_ROOT` in hooks/lib/common.py
/// and the private `runtime_root` in `src/common/session.rs`.
fn runtime_root() -> PathBuf {
    home_dir().join(".claude").join("runtime")
}

/// Current local time as `%Y-%m-%d %H:%M:%S`. Empty on any failure; never
/// panics.
fn current_timestamp() -> String {
    let mut command = Command::new("date");
    command.arg("+%Y-%m-%d %H:%M:%S");
    match run_with_timeout(&mut command, DATE_TIMEOUT) {
        Some(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => String::new(),
    }
}

/// Append `line` plus a trailing newline to `path`, with no locking of its
/// own: the caller holds a `with_dir_lock` around this and `cap_lines`
/// together (see the call site's comment for why). Never panics; a failure
/// is swallowed, matching every other hook write in this codebase.
fn append_line_unlocked(path: &Path, line: &str) {
    if let Ok(mut opened) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(opened, "{line}");
    }
}

/// Trim `path` down to its last `limit` lines in place, via a temp file
/// plus rename so a reader never observes a partial write. Ports
/// `_cap_lines` (hooks/precompact-warn.py:48). Never panics; any failure
/// leaves the file as it was.
fn cap_lines(path: &Path, limit: usize) {
    let Ok(contents) = fs::read_to_string(path) else {
        return;
    };
    let lines: Vec<&str> = contents.split_inclusive('\n').collect();
    if lines.len() <= limit {
        return;
    }
    let kept = &lines[lines.len() - limit..];
    let tmp_path = PathBuf::from(format!("{}.tmp.{}", path.display(), std::process::id()));
    let Ok(mut tmp_file) = fs::File::create(&tmp_path) else {
        return;
    };
    for line in kept {
        if tmp_file.write_all(line.as_bytes()).is_err() {
            let _ = fs::remove_file(&tmp_path);
            return;
        }
    }
    let _ = fs::rename(&tmp_path, path);
}
