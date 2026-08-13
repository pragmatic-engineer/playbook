// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Stop / SessionEnd hook (ports hooks/session-clean-exit.py): marks a
//! session as having ended cleanly, so the NEXT session's session-init hook
//! can detect an orphaned, crashed session (no clean-exit marker) versus a
//! graceful one.
//!
//! Wired to both `Stop` (fires after every assistant turn) and `SessionEnd`
//! (fires once, when the session truly ends) from the same script. `Stop`
//! refreshes `last-clean-ts` on every turn, so a stale-session check only
//! fires when a session is genuinely abandoned. Only `SessionEnd` carries a
//! `.reason`, and only when that reason is present and not `"other"` does
//! this hook write the `clean-exit` marker and consider queuing the
//! auto-learn nudge; that presence-and-not-other check is how the two
//! events are told apart from inside one script.

use crate::common::{session_dir, session_id, Payload};
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_AUTO_LEARN_MIN_EDITS: i64 = 5;

/// Run the session-clean-exit hook. Never panics; every failure along the
/// way is swallowed, matching hooks/session-clean-exit.py's fail-soft
/// contract.
pub fn run(payload: &Payload) {
    let dir = session_dir(payload);
    if dir.is_empty() {
        return;
    }

    refresh_last_clean_ts(&dir);

    let reason = payload.field(".reason");
    if reason.is_empty() || reason == "other" {
        return;
    }

    write_clean_exit_marker(&dir, &reason);
    queue_auto_learn(payload, &dir);
}

/// Stamp `last-clean-ts` with the current time. Fires on every `Stop`, not
/// only at session end, so a crash check downstream only trips when a
/// session is genuinely abandoned. Matches
/// hooks/session-clean-exit.py:27-31.
fn refresh_last_clean_ts(dir: &str) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = fs::write(Path::new(dir).join("last-clean-ts"), now.to_string());
}

/// Write the `clean-exit` marker holding the session-end reason. Matches
/// hooks/session-clean-exit.py:43-47.
fn write_clean_exit_marker(dir: &str, reason: &str) {
    let _ = fs::write(Path::new(dir).join("clean-exit"), format!("{reason}\n"));
}

/// Queue an auto-learn flag for this repo if the session made enough edits.
/// Matches hooks/session-clean-exit.py:49-85.
fn queue_auto_learn(payload: &Payload, dir: &str) {
    if std::env::var("AUTO_LEARN_NUDGE").unwrap_or_else(|_| "1".to_string()) == "0" {
        return;
    }
    let root = git_toplevel();
    if root.is_empty() {
        return;
    }

    let edits = read_int(&Path::new(dir).join("edit-count"));
    let threshold = std::env::var("AUTO_LEARN_MIN_EDITS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(DEFAULT_AUTO_LEARN_MIN_EDITS);
    if edits < threshold {
        return;
    }

    let home = std::env::var("HOME").unwrap_or_default();
    let qdir = Path::new(&home)
        .join(".claude")
        .join("runtime")
        .join("to-learn");
    if fs::create_dir_all(&qdir).is_err() {
        return;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let flag = AutoLearnFlag {
        repo_root: &root,
        edits,
        session_id: &session_id(payload),
        ts: now,
    };
    let Ok(rendered) = serde_json::to_string(&flag) else {
        return;
    };

    let slug = slugify(&root);
    let dest = qdir.join(format!("{slug}.json"));
    let tmp = qdir.join(format!("{slug}.json.tmp"));
    if fs::write(&tmp, rendered).is_ok() {
        let _ = fs::rename(&tmp, &dest);
    } else {
        let _ = fs::remove_file(&tmp);
    }
}

#[derive(Serialize)]
struct AutoLearnFlag<'a> {
    repo_root: &'a str,
    edits: i64,
    session_id: &'a str,
    ts: u64,
}

/// `git rev-parse --show-toplevel`, trimmed. Empty outside a repo or on any
/// failure. Never panics.
fn git_toplevel() -> String {
    match Command::new("git")
        .args(["--no-optional-locks", "rev-parse", "--show-toplevel"])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    }
}

/// Parse the digits in `path`'s contents as an integer, ignoring any other
/// characters. `0` on any read or parse failure. Matches
/// hooks/session-clean-exit.py:99-105.
fn read_int(path: &Path) -> i64 {
    let Ok(contents) = fs::read_to_string(path) else {
        return 0;
    };
    let digits: String = contents.chars().filter(char::is_ascii_digit).collect();
    digits.parse::<i64>().unwrap_or(0)
}

/// Replace every character outside `[A-Za-z0-9_.-]` with `_`. Matches the
/// slug regex hooks/session-clean-exit.py:71 applies inline.
fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}
