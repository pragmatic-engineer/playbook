// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration tests for `playbook::init::memory_migrate::migrate_memory`,
//! focused on the legacy filename rename it performs before moving the tree.
//! The whole-tree move itself (byte-identical copy, resume after an
//! interrupted run, permission failures) is covered by `memory_migrate.rs`'s
//! own internal `mod tests`, which calls the same public `migrate_memory`
//! entry point this file does but has direct access to the private
//! `SENTINEL_NAME` const to assert on the completion marker directly.

use playbook::init::memory_migrate::migrate_memory;
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

fn new_root_of(home: &Path) -> PathBuf {
    home.join(".config").join("playbook").join("memory")
}

fn old_graph_path(claude_home: &Path) -> PathBuf {
    mem_dir_of(claude_home).join("graph.json")
}

fn old_lock_path(claude_home: &Path) -> PathBuf {
    mem_dir_of(claude_home).join("graph.json.lock")
}

const GRAPH_CONTENT: &str = r#"{"nodes":[],"edges":[]}"#;

#[test]
fn only_old_file_present_wires_and_preserves_lock_sibling_at_the_new_root() {
    // Arrange: the old filename, plus its advisory lock sibling directory,
    // under the pre-migration location.
    let home = scratch_home("wired");
    let claude_home = claude_home_of(&home);
    fs::create_dir_all(mem_dir_of(&claude_home)).unwrap();
    fs::write(old_graph_path(&claude_home), GRAPH_CONTENT).unwrap();
    fs::create_dir(old_lock_path(&claude_home)).unwrap();

    // Act: one call renames, then moves the whole tree.
    let report = migrate_memory(&home, &claude_home);

    // Assert
    assert_eq!(report.status, StepStatus::Wired, "{}", report.detail);
    assert_eq!(report.name, "memory");
    let new_root = new_root_of(&home);
    assert_eq!(
        fs::read_to_string(new_root.join("memory.graph.json"))
            .expect("memory.graph.json should exist at the new root"),
        GRAPH_CONTENT,
        "the migration must preserve the old file's content exactly"
    );
    assert!(
        new_root.join("memory.graph.json.lock").is_dir(),
        "the lock sibling should be renamed and moved alongside the graph file"
    );
    assert!(
        !mem_dir_of(&claude_home).exists(),
        "the whole pre-migration tree should be gone once the move completes"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn neither_file_present_reports_skipped() {
    // Arrange: nothing written yet, not even the memory directory.
    let home = scratch_home("skipped");
    let claude_home = claude_home_of(&home);

    // Act
    let report = migrate_memory(&home, &claude_home);

    // Assert
    assert_eq!(report.status, StepStatus::Skipped, "{}", report.detail);
    assert!(!new_root_of(&home).exists());

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn correctly_named_file_with_no_lock_still_moves_the_tree() {
    // Arrange: the filename is already correct (a store migrated on some
    // prior run of just the rename, before this repo unified the two steps),
    // but the tree itself has never moved to the new root.
    let home = scratch_home("already-renamed-not-moved");
    let claude_home = claude_home_of(&home);
    fs::create_dir_all(mem_dir_of(&claude_home)).unwrap();
    fs::write(
        mem_dir_of(&claude_home).join("memory.graph.json"),
        GRAPH_CONTENT,
    )
    .unwrap();

    // Act
    let report = migrate_memory(&home, &claude_home);

    // Assert: the rename sub-step has nothing to do, but the combined step
    // still reports Wired, because the tree still had to move.
    assert_eq!(report.status, StepStatus::Wired, "{}", report.detail);
    assert_eq!(
        fs::read_to_string(new_root_of(&home).join("memory.graph.json")).unwrap(),
        GRAPH_CONTENT
    );
    assert!(!mem_dir_of(&claude_home).exists());

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn new_file_present_with_old_file_also_present_carries_both_to_the_new_root_untouched() {
    // Arrange: both filenames present at once under the old location.
    let home = scratch_home("both-present");
    let claude_home = claude_home_of(&home);
    fs::create_dir_all(mem_dir_of(&claude_home)).unwrap();
    fs::write(
        mem_dir_of(&claude_home).join("memory.graph.json"),
        "new content",
    )
    .unwrap();
    fs::write(
        old_graph_path(&claude_home),
        "old content, must stay untouched",
    )
    .unwrap();

    // Act
    let report = migrate_memory(&home, &claude_home);

    // Assert: the rename step leaves the stray old file alone (an old file
    // beside an already-migrated name is never touched or deleted), and the
    // tree move carries it along as-is, since it copies whatever is present
    // rather than filtering by filename.
    assert_eq!(report.status, StepStatus::Wired, "{}", report.detail);
    let new_root = new_root_of(&home);
    assert_eq!(
        fs::read_to_string(new_root.join("memory.graph.json")).unwrap(),
        "new content"
    );
    assert_eq!(
        fs::read_to_string(new_root.join("graph.json")).unwrap(),
        "old content, must stay untouched",
        "a stray old-named file must survive the migration untouched, not be dropped"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn migrate_memory_is_idempotent_on_repeated_calls() {
    // Arrange
    let home = scratch_home("idempotent");
    let claude_home = claude_home_of(&home);
    fs::create_dir_all(mem_dir_of(&claude_home)).unwrap();
    fs::write(old_graph_path(&claude_home), GRAPH_CONTENT).unwrap();

    // Act: migrate once, then again.
    let first = migrate_memory(&home, &claude_home);
    let second = migrate_memory(&home, &claude_home);

    // Assert
    assert_eq!(first.status, StepStatus::Wired, "{}", first.detail);
    assert_eq!(
        second.status,
        StepStatus::AlreadyCorrect,
        "{}",
        second.detail
    );
    assert_eq!(
        fs::read_to_string(new_root_of(&home).join("memory.graph.json")).unwrap(),
        GRAPH_CONTENT,
        "a repeated migration must not alter the already-migrated content"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn migrate_memory_composes_the_rename_and_move_into_one_detail_string() {
    // The whole point of unifying the rename and the tree move: one call,
    // one reported step, whose detail records that both halves ran, not
    // just that a step named "memory" exists.
    let home = scratch_home("composed-detail");
    let claude_home = claude_home_of(&home);
    fs::create_dir_all(mem_dir_of(&claude_home)).unwrap();
    fs::write(old_graph_path(&claude_home), GRAPH_CONTENT).unwrap();

    let report = migrate_memory(&home, &claude_home);

    assert_eq!(report.name, "memory");
    assert!(
        report.detail.contains("renamed legacy graph.json"),
        "detail should record the rename half: {}",
        report.detail
    );
    assert!(
        report
            .detail
            .contains(&new_root_of(&home).display().to_string()),
        "detail should record the move half: {}",
        report.detail
    );

    let _ = fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn a_failed_rename_does_not_abort_the_tree_move() {
    // Arrange: a legacy graph.json under a read-only memory directory, and
    // the new root pre-created (empty, no sentinel) so the move takes the
    // per-file copy path rather than a whole-directory rename. A plain
    // `fs::rename` of the whole directory needs write access to the source
    // directory itself on this filesystem (confirmed empirically: with the
    // new root absent, restricting mem_dir fails BOTH the in-place rename
    // and the directory move together), but `fs::copy`-ing individual files
    // out of it only needs read+execute on the source, which 0o555 still
    // grants, so the copy path isolates a rename failure from a move
    // failure the way this test needs.
    use std::os::unix::fs::PermissionsExt;

    let home = scratch_home("rename-fails-move-succeeds");
    let claude_home = claude_home_of(&home);
    let mem_dir = mem_dir_of(&claude_home);
    fs::create_dir_all(&mem_dir).unwrap();
    fs::write(old_graph_path(&claude_home), GRAPH_CONTENT).unwrap();
    fs::create_dir_all(new_root_of(&home)).unwrap();

    fs::set_permissions(&mem_dir, fs::Permissions::from_mode(0o555)).unwrap();
    // A test running as root bypasses Unix permission checks, which would
    // make the rename unexpectedly succeed; guard against that.
    let probe = mem_dir.join(".write-probe");
    let permissions_are_enforced = fs::write(&probe, "x").is_err();
    let _ = fs::set_permissions(&mem_dir, fs::Permissions::from_mode(0o755));
    let _ = fs::remove_file(&probe);
    if !permissions_are_enforced {
        eprintln!(
            "skipping a_failed_rename_does_not_abort_the_tree_move: \
             running as a user that bypasses directory permissions"
        );
        let _ = fs::remove_dir_all(&home);
        return;
    }
    fs::set_permissions(&mem_dir, fs::Permissions::from_mode(0o555)).unwrap();

    // Act
    let report = migrate_memory(&home, &claude_home);

    // Assert: the move still ran and won the combined status, and the
    // detail says the rename failed rather than staying silent about it.
    fs::set_permissions(&mem_dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        report.status,
        StepStatus::Wired,
        "a failed rename must not fail the whole step when the move still succeeds: {}",
        report.detail
    );
    assert!(
        report.detail.contains("could not rename legacy graph.json"),
        "the rename failure must not be silently swallowed: {}",
        report.detail
    );
    assert_eq!(
        fs::read_to_string(new_root_of(&home).join("graph.json")).unwrap(),
        GRAPH_CONTENT,
        "the old-named file must still reach the new root under its old name"
    );

    let _ = fs::remove_dir_all(&home);
}
