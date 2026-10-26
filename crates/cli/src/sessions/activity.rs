// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The small per session file that the `session-activity` hook writes
//! (ADR-0024, stage 3). It holds the two facts only a hook can see: whether a
//! turn is running, and which kind of Notification Claude Code sent last.
//!
//! The file holds no prompt, no message and no tool content. The notice is one
//! of three fixed words, so a bad payload cannot put free text on disk. The file
//! is written whole and renamed into place, stays under [`MAX_BYTES`], and is
//! read back only when it is a plain file the same user owns.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::Path;

const FILE: &str = "activity.json";
/// The most bytes read back. A real file is about 60.
const MAX_BYTES: u64 = 512;
/// The Notification types that mean the session waits for a person.
const NOTICES: [&str; 3] = ["permission_prompt", "idle_prompt", "agent_needs_input"];

/// Whether a turn is running.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Turn {
    Working,
    #[default]
    Idle,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Activity {
    turn: Turn,
    /// The last waiting notice since the turn began, one of [`NOTICES`].
    notice: Option<String>,
    /// Epoch seconds of the event.
    ts: u64,
}

/// Fold one hook event into the file in `dir`. An event this does not know, and
/// a Notification outside [`NOTICES`], change nothing. Every failure is
/// swallowed: a hook must never block a session.
pub(crate) fn record(dir: &Path, event: &str, notification_type: &str, now: u64) {
    let next = match event {
        "UserPromptSubmit" => Activity {
            turn: Turn::Working,
            notice: None,
            ts: now,
        },
        "Stop" => Activity {
            turn: Turn::Idle,
            notice: None,
            ts: now,
        },
        "Notification" => {
            let Some(notice) = NOTICES.iter().find(|n| **n == notification_type) else {
                return;
            };
            Activity {
                turn: read(dir).unwrap_or_default().turn,
                notice: Some((*notice).to_string()),
                ts: now,
            }
        }
        _ => return,
    };
    if let Ok(text) = serde_json::to_string(&next) {
        let _ = crate::common::atomic::write_atomic_private(&dir.join(FILE), text);
    }
}

/// The file in `dir`, or `None` when it is missing, not a plain file, too
/// large, owned by someone else, or not valid.
fn read(dir: &Path) -> Option<Activity> {
    let path = dir.join(FILE);
    let meta = fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES || !same_owner(&meta, dir) {
        return None;
    }
    let mut text = String::new();
    fs::File::open(&path)
        .ok()?
        .take(MAX_BYTES)
        .read_to_string(&mut text)
        .ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether the file belongs to the user that owns its folder. The folder is
/// created owner only, so another user cannot have put a file there. Windows
/// has no uid, and the folder sits in the user's own profile.
#[cfg(unix)]
fn same_owner(file: &fs::Metadata, dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(dir).is_ok_and(|d| d.uid() == file.uid())
}

#[cfg(not(unix))]
fn same_owner(_file: &fs::Metadata, _dir: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = crate::common::test_support::scratch_dir(name);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_prompt_marks_the_turn_working_and_a_stop_marks_it_idle() {
        let d = dir("activity-turn");
        record(&d, "UserPromptSubmit", "", 10);
        assert_eq!(
            read(&d),
            Some(Activity {
                turn: Turn::Working,
                notice: None,
                ts: 10
            })
        );
        record(&d, "Stop", "", 20);
        assert_eq!(read(&d).map(|a| (a.turn, a.ts)), Some((Turn::Idle, 20)));
    }

    #[test]
    fn a_waiting_notice_is_kept_until_the_next_prompt_clears_it() {
        let d = dir("activity-notice");
        record(&d, "UserPromptSubmit", "", 10);
        record(&d, "Notification", "permission_prompt", 15);
        let a = read(&d).unwrap();
        assert_eq!(
            (a.turn, a.notice.as_deref(), a.ts),
            (Turn::Working, Some("permission_prompt"), 15)
        );
        record(&d, "UserPromptSubmit", "", 30);
        assert_eq!(read(&d).unwrap().notice, None);
    }

    #[test]
    fn other_notification_types_and_events_change_nothing() {
        let d = dir("activity-ignored");
        record(&d, "Notification", "auth_success", 5);
        record(&d, "Notification", "some free text from a payload", 5);
        record(&d, "SessionStart", "", 5);
        assert_eq!(read(&d), None);
    }

    #[test]
    fn the_file_holds_only_the_fixed_fields() {
        let d = dir("activity-privacy");
        record(&d, "Notification", "idle_prompt", 7);
        let text = fs::read_to_string(d.join(FILE)).unwrap();
        assert_eq!(text, r#"{"turn":"idle","notice":"idle_prompt","ts":7}"#);
        assert!(text.len() as u64 <= MAX_BYTES);
    }

    #[test]
    fn an_oversized_or_invalid_file_reads_as_missing() {
        let d = dir("activity-bad");
        fs::write(d.join(FILE), "x".repeat(MAX_BYTES as usize + 1)).unwrap();
        assert_eq!(read(&d), None);
        fs::write(d.join(FILE), "not json").unwrap();
        assert_eq!(read(&d), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_is_not_followed() {
        let d = dir("activity-link");
        let target = d.join("elsewhere.json");
        fs::write(&target, r#"{"turn":"working","notice":null,"ts":1}"#).unwrap();
        std::os::unix::fs::symlink(&target, d.join(FILE)).unwrap();
        assert_eq!(read(&d), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("activity-mode");
        record(&d, "Stop", "", 1);
        let mode = fs::metadata(d.join(FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0);
    }
}
