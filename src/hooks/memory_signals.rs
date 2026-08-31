// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `~/.claude/memory/memory.signals.json` data layer: hit counters, staleness
//! verification stamps, and a consolidation cursor, one entry per memory
//! node. Owns the whole read/modify/write cycle under its own mkdir-based
//! advisory lock, mirroring `rebuild_memory_graph.rs`'s
//! `write_graph_atomically`/`rebuild` pattern. No hook dispatches into this
//! module yet: it is a standalone data layer for a later consumer to build
//! hit-counter promotion, staleness caching, and consolidation on top of.

use crate::common::atomic::with_dir_lock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

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
/// node has been promoted, and staleness verification stamps. Only `hits` is
/// read or written by this module today; the rest exist so a later consumer
/// has somewhere to store them without a schema change.
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

/// Increment `node_id`'s hit count by one, creating its entry if absent.
pub fn bump_hit(mem_dir: &Path, node_id: &str) {
    modify_locked(mem_dir, |store| {
        store.nodes.entry(node_id.to_string()).or_default().hits += 1;
    });
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
