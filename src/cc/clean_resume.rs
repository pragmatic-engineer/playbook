// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports the transcript half of `shell/shared/clean-resume.sh`: clone a
//! transcript, drop the config-override entries, and give the clone a fresh
//! session id so the harness sees a coherent history.
//!
//! The conversation survives while runtime config resets to `settings.json`
//! defaults, and the original transcript is never modified.
//!
//! **Launching is not here.** The shell function ends by exec'ing `claude
//! --resume`, which is the launcher's job and lands with the shim in WU-18.
//! This module prepares the clone and reports what it did, so the behaviour
//! that needs testing is testable without spawning a session.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Slash commands whose transcript entries reset runtime config. Dropping them
/// is the entire point: replaying them would re-apply the overrides this is
/// trying to clear.
///
/// The shell matched these with a jq regex; the tags are fixed strings, so a
/// substring test is equivalent and needs no regex crate.
const OVERRIDE_COMMANDS: [&str; 5] = ["model", "effort", "config", "output-style", "style"];

/// Entries for other slash commands, their stdout, and permission-mode changes
/// are deliberately KEPT. Permission grants especially: silently replaying a
/// cleaned transcript should not restore access the user has to re-grant.
fn is_override_entry(line: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    if value.get("type").and_then(|t| t.as_str()) != Some("system") {
        return false;
    }
    let content = value.get("content").and_then(|c| c.as_str()).unwrap_or("");
    OVERRIDE_COMMANDS
        .iter()
        .any(|cmd| content.contains(&format!("<command-name>/{cmd}</command-name>")))
}

#[derive(Debug)]
pub struct Prepared {
    pub new_sid: String,
    pub stripped: usize,
    pub kept: usize,
}

#[derive(Debug, PartialEq)]
pub enum CleanError {
    TranscriptMissing(PathBuf),
    Io(String),
}

/// Clones `old_sid`'s transcript into a new session, dropping override entries
/// and rewriting every `sessionId` to the new one.
pub fn prepare(project_dir: &Path, old_sid: &str) -> Result<Prepared, CleanError> {
    let old_jsonl = project_dir.join(format!("{old_sid}.jsonl"));
    if !old_jsonl.is_file() {
        return Err(CleanError::TranscriptMissing(old_jsonl));
    }
    let body = fs::read_to_string(&old_jsonl).map_err(|e| CleanError::Io(e.to_string()))?;

    let new_sid = new_session_id();
    let mut kept_lines = Vec::new();
    let mut stripped = 0usize;

    for line in body.lines().filter(|l| !l.trim().is_empty()) {
        if is_override_entry(line) {
            stripped += 1;
            continue;
        }
        kept_lines.push(rewrite_session_id(line, &new_sid));
    }

    let new_jsonl = project_dir.join(format!("{new_sid}.jsonl"));
    let mut out = kept_lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    fs::write(&new_jsonl, out).map_err(|e| CleanError::Io(e.to_string()))?;

    copy_sidecar(project_dir, old_sid, &new_sid);

    Ok(Prepared {
        new_sid,
        stripped,
        kept: kept_lines.len(),
    })
}

/// Only the `sessionId` field changes, and only when already present. A line
/// that is not an object, or has no `sessionId`, is passed through untouched.
fn rewrite_session_id(line: &str, new_sid: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(line) else {
        return line.to_string();
    };
    if let Some(obj) = value.as_object_mut() {
        if obj.contains_key("sessionId") {
            obj.insert(
                "sessionId".to_string(),
                serde_json::Value::String(new_sid.to_string()),
            );
        }
    }
    serde_json::to_string(&value).unwrap_or_else(|_| line.to_string())
}

/// Copied rather than symlinked: the harness writes into the new session's
/// sidecar, and a symlink would leak those writes back into the original,
/// breaking the promise that the original is untouched.
fn copy_sidecar(project_dir: &Path, old_sid: &str, new_sid: &str) {
    let src = project_dir.join(old_sid);
    if !src.is_dir() {
        return;
    }
    let dst = project_dir.join(new_sid);
    if fs::create_dir_all(&dst).is_err() {
        return;
    }
    let Ok(entries) = fs::read_dir(&src) else {
        return;
    };
    for entry in entries.flatten() {
        let target = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            let _ = fs::copy(entry.path(), target);
        }
    }
}

fn copy_tree(src: &Path, dst: &Path) {
    if fs::create_dir_all(dst).is_err() {
        return;
    }
    let Ok(entries) = fs::read_dir(src) else {
        return;
    };
    for entry in entries.flatten() {
        let target = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            let _ = fs::copy(entry.path(), target);
        }
    }
}

/// A lowercase v4-shaped UUID from the system CSPRNG.
///
/// The shell called `uuidgen | tr 'A-Z' 'a-z'`. Reading `/dev/urandom` keeps
/// the crate graph at clap plus serde rather than adding a uuid dependency for
/// sixteen bytes.
fn new_session_id() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut bytes);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 4122 variant
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}
