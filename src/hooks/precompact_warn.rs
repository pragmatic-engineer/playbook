// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Ports the retired shell original: a PreCompact hook that logs the
//! compaction event and warns the user, since PreCompact has no
//! `additionalContext` channel to speak to Claude directly.
//!
//! One divergence from the python source, non-observable: this port uses
//! `common::atomic_append` where the python hook appends with a bare `open`.
//!
//! Local system time via `localtime_r` stands in for python's
//! `time.strftime`, which also renders in the local timezone. `std` alone
//! has no timezone database, and spawning `date` cost a process per compaction.

use crate::common::atomic::with_dir_lock;
use crate::common::payload::Payload;
use crate::common::{emit_system_message, session_id};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const LOG_LINE_CAP: usize = 500;

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

    let log_path = crate::common::paths::runtime_root().join("compactions.log");
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

/// Current local time as `%Y-%m-%d %H:%M:%S`, formatted in process with
/// `localtime_r`. Empty on any failure; never panics.
#[cfg(unix)]
pub fn current_timestamp() -> String {
    // SAFETY: `time` and `localtime_r` write only to the locals passed in, and
    // `strftime` stays within the buffer length it is given.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return String::new();
        }
        let mut buf = [0 as libc::c_char; 32];
        let n = libc::strftime(
            buf.as_mut_ptr(),
            buf.len(),
            c"%Y-%m-%d %H:%M:%S".as_ptr(),
            &tm,
        );
        let bytes: Vec<u8> = buf[..n].iter().map(|&c| c as u8).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// No `localtime_r` off unix, so the log line carries an empty timestamp.
#[cfg(not(unix))]
pub fn current_timestamp() -> String {
    String::new()
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
/// `_cap_lines` (the retired shell original). Never panics; any failure
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn timestamp_has_the_format_the_date_command_produced() {
        // Arrange, Act
        let before = current_timestamp();
        let out = Command::new("date")
            .arg("+%Y-%m-%d %H:%M:%S")
            .output()
            .expect("date runs");
        let after = current_timestamp();
        let date = String::from_utf8_lossy(&out.stdout).trim().to_string();

        // Assert: `date` ran between the two reads, so it equals one of them.
        assert_eq!(before.len(), 19, "got {before:?}");
        assert!(
            date == before || date == after,
            "{date} vs {before}/{after}"
        );
    }
}
