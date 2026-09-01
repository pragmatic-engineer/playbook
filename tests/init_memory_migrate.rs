// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Unit tests for `playbook::init::memory_migrate::migrate_memory_store`.

use playbook::init::memory_migrate::migrate_memory_store;
use playbook::init::run::StepStatus;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory standing in for `$HOME`, unique per call.
fn scratch_home(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let home = env::temp_dir().join(format!(
        "playbook-init-memory-migrate-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&home).expect("scratch home should be creatable");
    home
}

fn claude_home_of(home: &Path) -> PathBuf {
    home.join(".claude")
}

fn mem_dir_of(claude_home: &Path) -> PathBuf {
    claude_home.join("memory")
}

fn old_graph_path(claude_home: &Path) -> PathBuf {
    mem_dir_of(claude_home).join("graph.json")
}

fn new_graph_path(claude_home: &Path) -> PathBuf {
    mem_dir_of(claude_home).join("memory.graph.json")
}

fn old_lock_path(claude_home: &Path) -> PathBuf {
    mem_dir_of(claude_home).join("graph.json.lock")
}

fn new_lock_path(claude_home: &Path) -> PathBuf {
    mem_dir_of(claude_home).join("memory.graph.json.lock")
}

const GRAPH_CONTENT: &str = r#"{"nodes":[],"edges":[]}"#;

#[test]
fn only_old_file_present_wires_and_preserves_lock_sibling() {
    // Arrange: the old filename, plus its advisory lock sibling directory.
    let home = scratch_home("wired");
    let claude_home = claude_home_of(&home);
    fs::create_dir_all(mem_dir_of(&claude_home)).unwrap();
    fs::write(old_graph_path(&claude_home), GRAPH_CONTENT).unwrap();
    fs::create_dir(old_lock_path(&claude_home)).unwrap();

    // Act
    let report = migrate_memory_store(&claude_home);

    // Assert
    assert_eq!(report.status, StepStatus::Wired, "{}", report.detail);
    assert!(
        !old_graph_path(&claude_home).exists(),
        "the old filename should be renamed away, not left behind"
    );
    assert_eq!(
        fs::read_to_string(new_graph_path(&claude_home))
            .expect("memory.graph.json should now exist"),
        GRAPH_CONTENT,
        "the migration must preserve the old file's content exactly"
    );
    assert!(
        new_lock_path(&claude_home).is_dir(),
        "the lock sibling should be renamed alongside the graph file"
    );
    assert!(
        !old_lock_path(&claude_home).exists(),
        "the old lock filename should not be left behind"
    );
}

#[test]
fn neither_file_present_reports_skipped() {
    // Arrange: nothing written yet.
    let home = scratch_home("skipped");
    let claude_home = claude_home_of(&home);

    // Act
    let report = migrate_memory_store(&claude_home);

    // Assert
    assert_eq!(report.status, StepStatus::Skipped, "{}", report.detail);
    assert!(!new_graph_path(&claude_home).exists());
}

#[test]
fn new_file_present_without_old_file_reports_already_correct() {
    // Arrange: a store already migrated on a prior run.
    let home = scratch_home("already-correct");
    let claude_home = claude_home_of(&home);
    fs::create_dir_all(mem_dir_of(&claude_home)).unwrap();
    fs::write(new_graph_path(&claude_home), GRAPH_CONTENT).unwrap();

    // Act
    let report = migrate_memory_store(&claude_home);

    // Assert
    assert_eq!(report.status, StepStatus::AlreadyCorrect, "{}", report.detail);
    assert!(!old_graph_path(&claude_home).exists());
}

#[test]
fn new_file_present_with_old_file_also_present_leaves_old_untouched() {
    // Arrange: both filenames present at once.
    let home = scratch_home("already-correct-with-old");
    let claude_home = claude_home_of(&home);
    fs::create_dir_all(mem_dir_of(&claude_home)).unwrap();
    fs::write(new_graph_path(&claude_home), "new content").unwrap();
    fs::write(old_graph_path(&claude_home), "old content, must stay untouched").unwrap();

    // Act
    let report = migrate_memory_store(&claude_home);

    // Assert
    assert_eq!(report.status, StepStatus::AlreadyCorrect, "{}", report.detail);
    assert_eq!(
        fs::read_to_string(old_graph_path(&claude_home)).unwrap(),
        "old content, must stay untouched",
        "an old file left alongside an already-migrated store must not be touched"
    );
    assert_eq!(
        fs::read_to_string(new_graph_path(&claude_home)).unwrap(),
        "new content"
    );
}

#[test]
fn migrate_memory_store_is_idempotent_on_repeated_calls() {
    // Arrange
    let home = scratch_home("idempotent");
    let claude_home = claude_home_of(&home);
    fs::create_dir_all(mem_dir_of(&claude_home)).unwrap();
    fs::write(old_graph_path(&claude_home), GRAPH_CONTENT).unwrap();

    // Act: migrate once, then again.
    let first = migrate_memory_store(&claude_home);
    let second = migrate_memory_store(&claude_home);

    // Assert
    assert_eq!(first.status, StepStatus::Wired, "{}", first.detail);
    assert_eq!(second.status, StepStatus::AlreadyCorrect, "{}", second.detail);
    assert_eq!(
        fs::read_to_string(new_graph_path(&claude_home)).unwrap(),
        GRAPH_CONTENT,
        "a repeated migration must not alter the already-migrated content"
    );
}
