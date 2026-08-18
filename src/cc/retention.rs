// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports `shell/shared/retention.sh`: bounds disk use by keeping only the
//! newest transcripts for the current project.
//!
//! Deletes three things per pruned session: the transcript, its tool-result
//! sidecar directory, and its runtime state.

use super::{claude_dir, project_slug};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const DEFAULT_KEEP: usize = 5;

/// Never keep fewer than two, whatever `CCD_KEEP` says. Fork and clean resume
/// both read the second-newest transcript as their parent, so a keep of 1 would
/// delete the session the next launch is about to resume from.
const KEEP_FLOOR: usize = 2;

pub fn prune(cwd: &str) {
    let keep = match resolve_keep() {
        // Zero disables retention entirely, which is the documented escape
        // hatch for anyone who wants their full history.
        None => return,
        Some(k) => k,
    };

    let project_dir = claude_dir().join("projects").join(project_slug(cwd));
    if !project_dir.is_dir() {
        return;
    }

    let ranked = transcripts_newest_first(&project_dir);
    if ranked.len() <= keep {
        return;
    }

    for path in &ranked[keep..] {
        let Some(sid) = path.file_stem().map(|s| s.to_string_lossy().to_string()) else {
            continue;
        };
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir_all(project_dir.join(&sid));
        let _ = fs::remove_dir_all(claude_dir().join("runtime").join(&sid));
    }
}

/// `None` means retention is off. Values below the floor are raised to it.
fn resolve_keep() -> Option<usize> {
    let raw = std::env::var("CCD_KEEP").unwrap_or_default();
    let keep = if raw.is_empty() {
        DEFAULT_KEEP
    } else {
        // A negative or unparseable value disables, matching the shell's
        // `[ "$keep" -le 0 ] && return 0` on an arithmetic comparison.
        match raw.parse::<i64>() {
            Ok(v) if v <= 0 => return None,
            Ok(v) => v as usize,
            Err(_) => return None,
        }
    };
    Some(keep.max(KEEP_FLOOR))
}

/// Transcripts sorted newest first by mtime. An unreadable mtime sorts oldest,
/// matching the shell's fallback of 0 when both `stat` forms fail.
fn transcripts_newest_first(project_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(project_dir) else {
        return Vec::new();
    };
    let mut ranked: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "jsonl"))
        .map(|p| {
            let mtime = fs::metadata(&p)
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (mtime, p)
        })
        .collect();
    ranked.sort_by_key(|(mtime, _)| std::cmp::Reverse(*mtime));
    ranked.into_iter().map(|(_, p)| p).collect()
}
