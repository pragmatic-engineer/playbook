// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports `shell/shared/sessions.sh`: session lookup, enumeration and listing.
//!
//! A session is a UUID-named `.jsonl` transcript under the project directory.
//! The `customTitle` embedded in its body is what `cc` resumes by, so a lookup
//! that matched on filename alone would resume the wrong session.

use super::project_slug;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How many sessions `cc list` prints, matching the shell's `head -10`.
const LIST_LIMIT: usize = 10;

/// Shown when a transcript carries no customTitle, matching the shell default.
const NO_TITLE: &str = "(no title)";

pub struct Session {
    pub mtime: SystemTime,
    pub id: String,
    pub title: String,
}

pub fn project_dir(claude_dir: &Path, cwd: &str) -> PathBuf {
    claude_dir.join("projects").join(project_slug(cwd))
}

/// The id of the newest transcript whose body carries `customTitle: name`.
///
/// Newest wins because the same title is reused across resumes of one logical
/// session, so the oldest match is usually a stale ancestor.
pub fn find_by_title(project_dir: &Path, name: &str) -> Option<String> {
    let needle = format!("\"customTitle\":\"{name}\"");
    let mut matches: Vec<(SystemTime, String)> = transcripts(project_dir)
        .into_iter()
        .filter_map(|path| {
            let body = fs::read_to_string(&path).ok()?;
            if !body.contains(&needle) {
                return None;
            }
            Some((mtime_of(&path), session_id(&path)?))
        })
        .collect();
    matches.sort_by_key(|(mtime, _)| std::cmp::Reverse(*mtime));
    matches.into_iter().next().map(|(_, id)| id)
}

/// Transcripts newest first, skipping anything not UUID-named.
///
/// **Quirk, ported deliberately:** at most one session per whole-second mtime.
/// The shell ended with `sort -rnu -k1,1`, and `-u` deduplicates on the sort
/// key, which is the timestamp. Two sessions written in the same second collapse
/// to one and the other silently disappears from `cc list`. Kept for parity;
/// which of the tied pair survives is unspecified in both implementations, so do
/// not rely on it. Worth fixing in both at some point, not during a port.
pub fn enumerate(project_dir: &Path) -> Vec<Session> {
    let mut out: Vec<Session> = transcripts(project_dir)
        .into_iter()
        .filter_map(|path| {
            let id = session_id(&path)?;
            if !is_uuid(&id) {
                return None;
            }
            Some(Session {
                mtime: mtime_of(&path),
                title: title_of(&path).unwrap_or_else(|| NO_TITLE.to_string()),
                id,
            })
        })
        .collect();
    // Whole seconds, because `stat` reports seconds and the dedup key is that
    // value. Sorting on the full SystemTime would order pairs the shell treats
    // as equal.
    out.sort_by_key(|s| std::cmp::Reverse(epoch_secs(s.mtime)));
    out.dedup_by_key(|s| epoch_secs(s.mtime));
    out
}

fn epoch_secs(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The rendered `cc list` output, returned rather than printed so tests can
/// assert on it without capturing stdout.
pub fn render_list(project_dir: &Path, cwd: &str) -> String {
    let project_name = cwd.rsplit('/').next().unwrap_or(cwd);
    if !project_dir.is_dir() {
        return format!("no sessions for {project_name}\n");
    }
    let mut out = format!("Recent sessions for {project_name}:\n");
    for session in enumerate(project_dir).into_iter().take(LIST_LIMIT) {
        let short: String = session.id.chars().take(8).collect();
        out.push_str(&format!(
            "  {}  {}...  {}\n",
            local_timestamp(session.mtime),
            short,
            session.title
        ));
    }
    out
}

/// Formats an mtime as local `%Y-%m-%d %H:%M` by calling `date`, exactly as the
/// shell did.
///
/// Local time needs the tz database, and the crate graph is deliberately clap
/// plus serde only. Reimplementing zone rules to avoid one call to coreutils
/// would be a far worse trade than the subprocess, which runs at most ten times
/// and only for a human-facing listing.
fn local_timestamp(mtime: SystemTime) -> String {
    let secs = epoch_secs(mtime);
    // BSD date first, then GNU, matching the shell's fallback order.
    for args in [
        vec!["-r".to_string(), secs.to_string()],
        vec!["-d".to_string(), format!("@{secs}")],
    ] {
        let out = std::process::Command::new("date")
            .args(&args)
            .arg("+%Y-%m-%d %H:%M")
            .output();
        if let Ok(o) = out {
            if o.status.success() {
                return String::from_utf8_lossy(&o.stdout).trim().to_string();
            }
        }
    }
    String::new()
}

/// Excludes `memory/` and other non-session directories that share the
/// project directory.
pub fn is_uuid(s: &str) -> bool {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == GROUPS.len()
        && parts.iter().zip(GROUPS).all(|(part, len)| {
            part.len() == len
                && part
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        })
}

fn transcripts(project_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(project_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "jsonl"))
        .collect()
}

fn session_id(path: &Path) -> Option<String> {
    path.file_stem().map(|s| s.to_string_lossy().to_string())
}

fn mtime_of(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// First `"customTitle":"..."` in the file, matching the shell's `grep -m1`.
fn title_of(path: &Path) -> Option<String> {
    const KEY: &str = "\"customTitle\":\"";
    let body = fs::read_to_string(path).ok()?;
    let start = body.find(KEY)? + KEY.len();
    let rest = &body[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}
