// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Soft, non-blocking staleness marker, cached in `memory.signals.json` so
//! a repeat check costs only one read.

use crate::hooks::memory_signals;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// Unix epoch seconds; no date-handling dependency exists in this crate.
pub type DateTime = i64;

/// The result of a staleness check: additive and non-blocking.
#[derive(Debug, PartialEq, Eq)]
pub struct Staleness {
    pub stale: bool,
}

/// Whether `anchor_path` drifted since the `node_id` fact was last touched.
/// A cache hit costs only one read; a miss recomputes once and caches it.
pub fn check_staleness(
    mem_dir: &Path,
    node_id: &str,
    anchor_path: &Path,
    git_lookup: &impl Fn(&Path) -> Option<DateTime>,
) -> Staleness {
    if let Some(stale) = memory_signals::cached_stale(mem_dir, node_id) {
        return Staleness { stale };
    }

    let (stale, signature) = compute_verdict(mem_dir, anchor_path, git_lookup);
    memory_signals::set_staleness(mem_dir, node_id, stale, signature);
    Staleness { stale }
}

/// Same-repo: compares the anchor's git-log date to `memory.graph.json`'s
/// mtime. Non-repo: never stale on first sight, hashed only for later use.
fn compute_verdict(
    mem_dir: &Path,
    anchor_path: &Path,
    git_lookup: &impl Fn(&Path) -> Option<DateTime>,
) -> (bool, Option<String>) {
    if !is_same_repo_anchor(anchor_path) {
        return (false, content_hash(anchor_path));
    }
    let stale = git_lookup(anchor_path)
        .zip(fact_touched_at(mem_dir))
        .is_some_and(|(anchor_date, touched_at)| anchor_date > touched_at);
    (stale, None)
}

/// Walks up from `anchor_path` looking for a `.git` directory.
fn is_same_repo_anchor(anchor_path: &Path) -> bool {
    let mut dir = anchor_path.parent();
    while let Some(current) = dir {
        if current.join(".git").exists() {
            return true;
        }
        dir = current.parent();
    }
    false
}

/// `memory.graph.json`'s own mtime: the fact's "last touched" reference.
fn fact_touched_at(mem_dir: &Path) -> Option<DateTime> {
    let modified = fs::metadata(mem_dir.join("memory.graph.json"))
        .ok()?
        .modified()
        .ok()?;
    let secs = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    DateTime::try_from(secs).ok()
}

/// A cheap, non-cryptographic content signature; no hashing crate exists
/// in this workspace, so `DefaultHasher` stands in.
fn content_hash(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Some(format!("{:x}", hasher.finish()))
}
