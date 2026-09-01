// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Soft, non-blocking staleness marker, cached in `memory.signals.json` so
//! a repeat check costs only one read.

use crate::common::atomic::with_dir_lock;
use serde_json::{Map, Value};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    if let Some(stale) = read_cached_stale(mem_dir, node_id) {
        return Staleness { stale };
    }

    let (stale, signature) = compute_verdict(mem_dir, anchor_path, git_lookup);
    write_cached_verdict(mem_dir, node_id, stale, signature);
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

/// Reads `node_id`'s cached verdict, or `None` on a cache miss.
fn read_cached_stale(mem_dir: &Path, node_id: &str) -> Option<bool> {
    let content = fs::read_to_string(mem_dir.join("memory.signals.json")).ok()?;
    let root: Value = serde_json::from_str(&content).ok()?;
    root.get("nodes")?.get(node_id)?.get("stale")?.as_bool()
}

/// Writes `node_id`'s fresh verdict back, other nodes and fields untouched.
fn write_cached_verdict(mem_dir: &Path, node_id: &str, stale: bool, signature: Option<String>) {
    let lock_path = mem_dir.join("memory.signals.json.lock");
    let (acquired, ()) = with_dir_lock(&lock_path, 50, Duration::from_millis(10), || {
        let mut root = read_store(mem_dir);
        let Some(root_obj) = root.as_object_mut() else {
            return;
        };
        let nodes_val = root_obj
            .entry("nodes")
            .or_insert_with(|| Value::Object(Map::new()));
        let Some(nodes_obj) = nodes_val.as_object_mut() else {
            return;
        };
        let node_val = nodes_obj
            .entry(node_id)
            .or_insert_with(|| Value::Object(Map::new()));
        let Some(node_obj) = node_val.as_object_mut() else {
            return;
        };
        node_obj.insert("stale".to_string(), Value::Bool(stale));
        node_obj.insert(
            "verified_at".to_string(),
            Value::String(current_epoch_secs().to_string()),
        );
        if let Some(hash) = signature {
            node_obj.insert("verified_hash".to_string(), Value::String(hash));
        }
        write_store_atomically(mem_dir, &root);
    });
    if acquired {
        let _ = fs::remove_dir(&lock_path);
    }
}

fn current_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The whole store, read generically instead of via `memory_signals.rs`'s
/// private typed shape.
fn read_store(mem_dir: &Path) -> Value {
    fs::read_to_string(mem_dir.join("memory.signals.json"))
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_else(|| Value::Object(Map::new()))
}

/// Writes `root` via a temp file plus a rename; its own prefix avoids
/// colliding with `memory_signals.rs`'s writes to the same file.
fn write_store_atomically(mem_dir: &Path, root: &Value) {
    let Ok(rendered) = serde_json::to_string_pretty(root) else {
        return;
    };
    let tmp_path = mem_dir.join(format!(
        ".signals-staleness-{}-{:?}.json.tmp",
        std::process::id(),
        std::thread::current().id()
    ));
    if fs::write(&tmp_path, rendered).is_err() {
        let _ = fs::remove_file(&tmp_path);
        return;
    }
    if fs::rename(&tmp_path, mem_dir.join("memory.signals.json")).is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
}
