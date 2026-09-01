// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `~/.claude/memory/memory.signals.json` data layer: hit counters, staleness
//! verification stamps, and a consolidation cursor, one entry per memory
//! node. Owns the whole read/modify/write cycle under its own mkdir-based
//! advisory lock, mirroring `rebuild_memory_graph.rs`'s
//! `write_graph_atomically`/`rebuild` pattern. `bump_hit` also promotes a
//! node once its hits cross a threshold within a rolling window; staleness
//! caching and consolidation have no reader or writer yet, and remain a
//! later consumer's work to build on top of this store.

use crate::common::atomic::with_dir_lock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

/// How many hits within `PROMOTION_WINDOW_SECS` a node needs before it is
/// marked `promoted`. Deliberately lower than "rediscovered five-plus
/// times", the pain this exists to catch: three hits should flag a pattern
/// before it repeats five times, not after.
const PROMOTION_HIT_THRESHOLD: u32 = 3;

/// The rolling window `PROMOTION_HIT_THRESHOLD` hits must land inside to
/// count together. Three matches inside a week is a real, active pattern;
/// three matches spread across six months is not the same signal.
const PROMOTION_WINDOW_SECS: u64 = 7 * 24 * 60 * 60;

/// The whole signals store: one node entry per memory node plus a shared
/// consolidation cursor. Round-trips through `serde_json` as-is; a missing
/// or unparsable file falls back to `SignalsStore::default()`, an empty
/// store with no nodes.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SignalsStore {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    cursor: Cursor,
    #[serde(default)]
    nodes: HashMap<String, NodeSignals>,
}

/// Where the consolidation pass last left off. A placeholder shape: only a
/// single optional timestamp exists today because nothing in this module
/// reads or writes it yet, a later consolidation pass defines what it means.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Cursor {
    #[serde(default)]
    last_run_at: Option<String>,
}

/// Per-node signals: hit count, the window it was counted over, whether the
/// node has been promoted, and staleness verification stamps. `hits`,
/// `window_start`, and `promoted` are read and written by `bump_hit` and
/// `is_promoted`; `verified_hash`/`verified_at` exist so a later staleness
/// consumer has somewhere to store them without a schema change.
#[derive(Debug, Default, Serialize, Deserialize)]
struct NodeSignals {
    #[serde(default)]
    hits: u32,
    #[serde(default)]
    window_start: Option<String>,
    #[serde(default)]
    promoted: bool,
    #[serde(default)]
    verified_hash: Option<String>,
    #[serde(default)]
    verified_at: Option<String>,
}

/// Run `f` against the current store loaded from `mem_dir`'s
/// `memory.signals.json` (an empty default if the file is missing, the
/// normal first-run case), then write the mutated store back atomically. A
/// file that exists but fails to read or parse is left untouched instead:
/// unlike `memory.graph.json`, this store accumulates state nothing else can
/// rebuild, so silently replacing a corrupt file with an empty one would be
/// permanent, unrecoverable data loss. Serializes concurrent callers with an
/// mkdir-based advisory lock at `memory.signals.json.lock`, mirroring
/// `rebuild_memory_graph.rs`'s `rebuild()`: without it, two callers racing to
/// bump different nodes can each read the same starting snapshot, and
/// whichever write finishes second silently discards the other's change.
pub fn modify_locked(mem_dir: &Path, f: impl FnOnce(&mut SignalsStore)) {
    let lock_path = mem_dir.join("memory.signals.json.lock");
    let (acquired, ()) = with_dir_lock(&lock_path, 50, Duration::from_millis(10), || {
        if let Some(mut store) = read_store(mem_dir) {
            store.version = 1;
            f(&mut store);
            write_store_atomically(mem_dir, &store);
        }
    });
    if acquired {
        let _ = fs::remove_dir(&lock_path);
    }
}

/// Increment `node_id`'s hit count by one, creating its entry if absent, and
/// mark it promoted once its hits cross `PROMOTION_HIT_THRESHOLD` within a
/// `PROMOTION_WINDOW_SECS` rolling window. A hit landing outside that window
/// (or a node with no window recorded yet) starts a fresh window at a count
/// of one, rather than accumulating with hits from long before. Promotion is
/// a one-way ratchet: once `promoted` is `true`, this never resets it back
/// to `false`.
pub fn bump_hit(mem_dir: &Path, node_id: &str) {
    let now = current_epoch_secs();
    modify_locked(mem_dir, |store| {
        let entry = store.nodes.entry(node_id.to_string()).or_default();
        let window_expired = entry
            .window_start
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok())
            .is_none_or(|start| now.saturating_sub(start) > PROMOTION_WINDOW_SECS);
        if window_expired {
            entry.window_start = Some(now.to_string());
            entry.hits = 1;
        } else {
            entry.hits += 1;
        }
        if entry.hits >= PROMOTION_HIT_THRESHOLD {
            entry.promoted = true;
        }
    });
}

fn current_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Whether `node_id` has been promoted. A best-effort, unlocked read: a
/// missing node, a missing file, or a file that fails to parse all mean
/// `false`, not an error, matching this module's fail-open philosophy.
pub fn is_promoted(mem_dir: &Path, node_id: &str) -> bool {
    read_store(mem_dir)
        .and_then(|store| store.nodes.get(node_id).map(|n| n.promoted))
        .unwrap_or(false)
}

/// Every promoted node id, in one read. A caller checking many ids (e.g.
/// every node in a graph at SessionStart) should call this once instead of
/// `is_promoted` per id, since each call to `is_promoted` re-reads and
/// re-parses the whole file. A missing or unparsable file returns an empty
/// set, matching `is_promoted`'s own fail-open contract.
pub fn promoted_ids(mem_dir: &Path) -> std::collections::HashSet<String> {
    read_store(mem_dir)
        .map(|store| {
            store
                .nodes
                .into_iter()
                .filter(|(_, signals)| signals.promoted)
                .map(|(id, _)| id)
                .collect()
        })
        .unwrap_or_default()
}

/// A missing file returns an empty default store. A file that exists but
/// fails to read or parse returns `None`, so the caller can leave it on disk
/// untouched rather than overwriting possibly-recoverable data.
fn read_store(mem_dir: &Path) -> Option<SignalsStore> {
    match fs::read_to_string(mem_dir.join("memory.signals.json")) {
        Ok(content) => serde_json::from_str(&content).ok(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(SignalsStore::default()),
        Err(_) => None,
    }
}

/// Write `store` to `memory.signals.json` inside `mem_dir` via a temp file in
/// the same directory plus a rename, so a reader never observes a partially
/// written file. Mirrors `rebuild_memory_graph.rs`'s `write_graph_atomically`.
fn write_store_atomically(mem_dir: &Path, store: &SignalsStore) {
    let Ok(rendered) = serde_json::to_string_pretty(store) else {
        return;
    };
    let tmp_path = mem_dir.join(format!(
        ".signals-{}-{:?}.json.tmp",
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
