// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Behavioural tests for `staleness::check_staleness`, exercised directly
//! in-process, mirroring `tests/hooks_signals.rs`'s pattern.

use playbook::hooks::staleness::check_staleness;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// --- Test infrastructure ---------------------------------------------------

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory, unique per call.
fn scratch_dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("playbook-wu4-{}-{tag}-{n}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes a minimal `memory.graph.json` into `mem_dir`, returning its mtime
/// as Unix epoch seconds: the fact-touched reference.
fn seed_graph(mem_dir: &Path) -> i64 {
    let path = mem_dir.join("memory.graph.json");
    fs::write(&path, r#"{"nodes":[],"edges":[]}"#).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    modified.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

/// A fake repo: a `.git` marker directory plus an anchor file.
fn seed_repo_with_anchor(repo_root: &Path, content: &str) -> PathBuf {
    fs::create_dir_all(repo_root.join(".git")).unwrap();
    let anchor_path = repo_root.join("src").join("anchor.rs");
    fs::create_dir_all(anchor_path.parent().unwrap()).unwrap();
    fs::write(&anchor_path, content).unwrap();
    anchor_path
}

/// A call-counting fake git lookup returning a fixed date every call.
fn counting_lookup(fixed_date: Option<i64>) -> (impl Fn(&Path) -> Option<i64>, Rc<Cell<u32>>) {
    let calls = Rc::new(Cell::new(0u32));
    let calls_inner = calls.clone();
    let lookup = move |_: &Path| -> Option<i64> {
        calls_inner.set(calls_inner.get() + 1);
        fixed_date
    };
    (lookup, calls)
}

fn signals_mtime(mem_dir: &Path) -> Option<SystemTime> {
    fs::metadata(mem_dir.join("memory.signals.json"))
        .ok()
        .and_then(|m| m.modified().ok())
}

// --- Scenario 1: same-repo anchor committed after the fact -----------------

#[test]
fn a_same_repo_anchor_committed_after_the_fact_is_stale_on_first_check() {
    // Arrange
    let mem_dir = scratch_dir("scenario1-mem");
    let repo_root = scratch_dir("scenario1-repo");
    let fact_touched_at = seed_graph(&mem_dir);
    let anchor_path = seed_repo_with_anchor(&repo_root, "original");
    let (git_lookup, calls) = counting_lookup(Some(fact_touched_at + 1000));

    // Act
    let result = check_staleness(&mem_dir, "global/example-fact", &anchor_path, &git_lookup);

    // Assert
    assert!(
        result.stale,
        "an anchor committed after the fact was last touched must be marked stale"
    );
    assert_eq!(
        calls.get(),
        1,
        "a cache miss must call the git lookup exactly once"
    );

    let _ = fs::remove_dir_all(&mem_dir);
    let _ = fs::remove_dir_all(&repo_root);
}

// --- Scenario 2: repeat check reuses the cached verdict ---------------------

#[test]
fn a_repeat_check_right_after_reads_the_cache_without_a_second_git_call() {
    // Arrange
    let mem_dir = scratch_dir("scenario2-mem");
    let repo_root = scratch_dir("scenario2-repo");
    let fact_touched_at = seed_graph(&mem_dir);
    let anchor_path = seed_repo_with_anchor(&repo_root, "original");
    let (git_lookup, calls) = counting_lookup(Some(fact_touched_at + 1000));

    // Act
    let first = check_staleness(&mem_dir, "global/example-fact", &anchor_path, &git_lookup);
    let second = check_staleness(&mem_dir, "global/example-fact", &anchor_path, &git_lookup);

    // Assert
    assert_eq!(
        calls.get(),
        1,
        "the second check must not call the git lookup again"
    );
    assert_eq!(
        second.stale, first.stale,
        "the second check must return the same cached verdict as the first"
    );

    let _ = fs::remove_dir_all(&mem_dir);
    let _ = fs::remove_dir_all(&repo_root);
}

// --- Scenario 3: same commit point is not stale -----------------------------

#[test]
fn a_same_repo_anchor_committed_at_the_same_point_as_the_fact_is_not_stale() {
    // Arrange
    let mem_dir = scratch_dir("scenario3-mem");
    let repo_root = scratch_dir("scenario3-repo");
    let fact_touched_at = seed_graph(&mem_dir);
    let anchor_path = seed_repo_with_anchor(&repo_root, "original");
    let (git_lookup, _calls) = counting_lookup(Some(fact_touched_at));

    // Act
    let result = check_staleness(&mem_dir, "global/example-fact", &anchor_path, &git_lookup);

    // Assert
    assert!(
        !result.stale,
        "an anchor committed at the same point as the fact must not be marked stale"
    );

    let _ = fs::remove_dir_all(&mem_dir);
    let _ = fs::remove_dir_all(&repo_root);
}

// --- Scenario 4: non-repo anchor falls back to a content hash --------------

#[test]
fn a_non_repo_anchor_falls_back_to_a_cached_content_hash() {
    // Arrange
    let mem_dir = scratch_dir("scenario4-mem");
    let repo_root = scratch_dir("scenario4-repo");
    let anchor_path = repo_root.join("notes.txt");
    fs::write(&anchor_path, "original content").unwrap();
    let (git_lookup, calls) = counting_lookup(Some(0));

    // Act
    let first = check_staleness(&mem_dir, "global/example-fact", &anchor_path, &git_lookup);
    let mtime_after_first = signals_mtime(&mem_dir);
    fs::write(&anchor_path, "changed content").unwrap();
    let second = check_staleness(&mem_dir, "global/example-fact", &anchor_path, &git_lookup);
    let mtime_after_second = signals_mtime(&mem_dir);

    // Assert
    assert_eq!(
        calls.get(),
        0,
        "a non-repo anchor must never call the git lookup"
    );
    assert_eq!(
        second.stale, first.stale,
        "a repeat check on a non-repo anchor must return the cached verdict, not re-hash"
    );
    assert_eq!(
        mtime_after_first, mtime_after_second,
        "the repeat check must be a pure read of memory.signals.json, not a fresh write"
    );

    let _ = fs::remove_dir_all(&mem_dir);
    let _ = fs::remove_dir_all(&repo_root);
}

// --- Scenario 5: untracked/uncommitted anchor shows no marker --------------

#[test]
fn an_unverifiable_same_repo_anchor_shows_no_marker_not_a_false_positive() {
    // Arrange
    let mem_dir = scratch_dir("scenario5-mem");
    let repo_root = scratch_dir("scenario5-repo");
    let anchor_path = seed_repo_with_anchor(&repo_root, "untracked");
    let (git_lookup, calls) = counting_lookup(None);

    // Act
    let result = check_staleness(&mem_dir, "global/example-fact", &anchor_path, &git_lookup);

    // Assert
    assert!(
        !result.stale,
        "a git lookup returning None must never be treated as stale"
    );
    assert_eq!(
        calls.get(),
        1,
        "a same-repo anchor must still attempt the git lookup once"
    );

    let _ = fs::remove_dir_all(&mem_dir);
    let _ = fs::remove_dir_all(&repo_root);
}
