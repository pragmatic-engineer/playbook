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
    // The old-root rename failed, so the copy carried the file to the new
    // root under its old name, exactly as before this test's follow-up
    // fix. But the new-root self-heal folded into `migrate_memory` now
    // catches this immediately: it fires right after the move, in the same
    // call, so the stray old name never lingers at the new root even when
    // the earlier old-root rename couldn't run.
    let new_root = new_root_of(&home);
    assert!(
        !new_root.join("graph.json").exists(),
        "the new-root self-heal folded into migrate_memory should catch \
         the stray old name the failed rename left behind"
    );
    assert_eq!(
        fs::read_to_string(new_root.join("memory.graph.json")).unwrap(),
        GRAPH_CONTENT,
        "the new-root self-heal must rename the file in place, preserving content"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn stray_file_at_new_root_is_renamed_with_content_preserved() {
    // Arrange: the tree is already fully migrated (sentinel present), but a
    // stray old-named `graph.json` also sits at the new root, e.g. left
    // behind by an older build or a manual copy: a case `migrate_memory`'s
    // own rename step never covers on its own, since that step only ever
    // looks at the OLD root.
    let home = scratch_home("new-root-stray-renamed");
    let claude_home = claude_home_of(&home);
    let new_root = new_root_of(&home);
    fs::create_dir_all(&new_root).unwrap();
    fs::write(new_root.join("graph.json"), GRAPH_CONTENT).unwrap();
    fs::write(new_root.join(".migration-complete"), "migrated\n").unwrap();

    // Act
    let report = migrate_memory(&home, &claude_home);

    // Assert: the stray file is renamed in place, content preserved, and
    // the call reports its usual (already-migrated) status; the new-root
    // self-heal is a silent, best-effort side effect, not a status change.
    assert_eq!(
        report.status,
        StepStatus::AlreadyCorrect,
        "{}",
        report.detail
    );
    assert!(
        !new_root.join("graph.json").exists(),
        "the stray new-root file should be renamed away, not left behind"
    );
    assert_eq!(
        fs::read_to_string(new_root.join("memory.graph.json")).unwrap(),
        GRAPH_CONTENT,
        "the rename must preserve the stray file's content exactly"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn new_root_file_already_correct_is_left_untouched_when_a_stray_file_is_also_present() {
    // Arrange: the new root already has BOTH the correct filename and a
    // stray old-named file sitting beside it. The correct file must win:
    // it's never overwritten or deleted by the stray one.
    let home = scratch_home("new-root-both-present");
    let claude_home = claude_home_of(&home);
    let new_root = new_root_of(&home);
    fs::create_dir_all(&new_root).unwrap();
    fs::write(new_root.join("memory.graph.json"), "correct content").unwrap();
    fs::write(
        new_root.join("graph.json"),
        "stray content, must not overwrite",
    )
    .unwrap();
    fs::write(new_root.join(".migration-complete"), "migrated\n").unwrap();

    // Act
    let report = migrate_memory(&home, &claude_home);

    // Assert: neither file is touched.
    assert_eq!(
        report.status,
        StepStatus::AlreadyCorrect,
        "{}",
        report.detail
    );
    assert_eq!(
        fs::read_to_string(new_root.join("memory.graph.json")).unwrap(),
        "correct content",
        "an already-correct file must never be overwritten by a stray sibling"
    );
    assert_eq!(
        fs::read_to_string(new_root.join("graph.json")).unwrap(),
        "stray content, must not overwrite",
        "a stray file next to an already-correct one is left alone, never deleted"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn new_root_self_heal_is_idempotent_on_a_second_call() {
    // Arrange: a stray file at the new root, not yet renamed.
    let home = scratch_home("new-root-idempotent");
    let claude_home = claude_home_of(&home);
    let new_root = new_root_of(&home);
    fs::create_dir_all(&new_root).unwrap();
    fs::write(new_root.join("graph.json"), GRAPH_CONTENT).unwrap();
    fs::write(new_root.join(".migration-complete"), "migrated\n").unwrap();

    // Act: call once (the rename happens here), then again (must be a no-op).
    let first = migrate_memory(&home, &claude_home);
    let second = migrate_memory(&home, &claude_home);

    // Assert: the first call performs the rename; the second is a genuine
    // no-op, not a repeat rename or a crash on an already-absent source.
    assert_eq!(first.status, StepStatus::AlreadyCorrect, "{}", first.detail);
    assert_eq!(
        second.status,
        StepStatus::AlreadyCorrect,
        "{}",
        second.detail
    );
    assert!(
        !new_root.join("graph.json").exists(),
        "the stray file must be gone after the first call"
    );
    assert_eq!(
        fs::read_to_string(new_root.join("memory.graph.json")).unwrap(),
        GRAPH_CONTENT,
        "content must survive both calls unchanged"
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn new_root_self_heal_leaves_a_stray_lock_directory_under_its_legacy_name() {
    // Arrange: a stray `graph.json` at the new root, with its own mkdir-based
    // lock sibling under the OLD name too, unlike the old-root rename (which
    // does carry `graph.json.lock` across), the new-root self-heal
    // deliberately leaves the lock alone: nothing ever reads or takes a lock
    // under the legacy name, only `memory.graph.json.lock`.
    let home = scratch_home("new-root-lock-left-alone");
    let claude_home = claude_home_of(&home);
    let new_root = new_root_of(&home);
    fs::create_dir_all(&new_root).unwrap();
    fs::write(new_root.join("graph.json"), GRAPH_CONTENT).unwrap();
    fs::create_dir(new_root.join("graph.json.lock")).unwrap();
    fs::write(new_root.join(".migration-complete"), "migrated\n").unwrap();

    // Act
    let report = migrate_memory(&home, &claude_home);

    // Assert: the graph file is renamed, but the lock directory is left
    // exactly where it was, under its legacy name, not renamed or removed.
    assert_eq!(
        report.status,
        StepStatus::AlreadyCorrect,
        "{}",
        report.detail
    );
    assert_eq!(
        fs::read_to_string(new_root.join("memory.graph.json")).unwrap(),
        GRAPH_CONTENT
    );
    assert!(
        new_root.join("graph.json.lock").is_dir(),
        "the new-root self-heal deliberately does not rename the lock sibling"
    );
    assert!(!new_root.join("memory.graph.json.lock").exists());

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn new_root_self_heal_never_renames_a_directory_named_graph_json() {
    // Arrange: `graph.json` at the new root is a directory, not a file (an
    // unlikely but possible on-disk state). The `is_file()` guard must
    // refuse it, unlike a looser `exists()` check, which would rename a
    // directory onto the path every reader expects to be the graph file.
    let home = scratch_home("new-root-graph-json-is-a-directory");
    let claude_home = claude_home_of(&home);
    let new_root = new_root_of(&home);
    fs::create_dir_all(new_root.join("graph.json")).unwrap();
    fs::write(new_root.join(".migration-complete"), "migrated\n").unwrap();

    // Act
    let report = migrate_memory(&home, &claude_home);

    // Assert: the directory is left exactly where it was; no rename attempted.
    assert_eq!(
        report.status,
        StepStatus::AlreadyCorrect,
        "{}",
        report.detail
    );
    assert!(
        new_root.join("graph.json").is_dir(),
        "a directory named graph.json must never be renamed by the self-heal"
    );
    assert!(!new_root.join("memory.graph.json").exists());

    let _ = fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn new_root_self_heal_fires_even_when_the_old_root_rename_fails_and_a_distinct_stray_file_already_sits_at_the_new_root(
) {
    // Arrange: the exact composition this fold-in's design has to get right:
    // an OLD-root rename failure (read-only source directory) happening at
    // the same time as an INDEPENDENT, pre-existing
    // stray `graph.json` already sitting at the new root, seeded with its
    // own distinct content before the call, not arriving via this call's
    // copy. Both self-heals must still do their job without interfering.
    use std::os::unix::fs::PermissionsExt;

    let home = scratch_home("new-root-stray-plus-failed-old-rename");
    let claude_home = claude_home_of(&home);
    let mem_dir = mem_dir_of(&claude_home);
    fs::create_dir_all(&mem_dir).unwrap();
    fs::write(old_graph_path(&claude_home), GRAPH_CONTENT).unwrap();

    let new_root = new_root_of(&home);
    fs::create_dir_all(&new_root).unwrap();
    fs::write(
        new_root.join("graph.json"),
        "independent new-root stray, seeded before the call",
    )
    .unwrap();

    fs::set_permissions(&mem_dir, fs::Permissions::from_mode(0o555)).unwrap();
    let probe = mem_dir.join(".write-probe");
    let permissions_are_enforced = fs::write(&probe, "x").is_err();
    let _ = fs::set_permissions(&mem_dir, fs::Permissions::from_mode(0o755));
    let _ = fs::remove_file(&probe);
    if !permissions_are_enforced {
        eprintln!(
            "skipping new_root_self_heal_fires_even_when_the_old_root_rename_fails_and_a_distinct_stray_file_already_sits_at_the_new_root: \
             running as a user that bypasses directory permissions"
        );
        let _ = fs::remove_dir_all(&home);
        return;
    }
    fs::set_permissions(&mem_dir, fs::Permissions::from_mode(0o555)).unwrap();

    // Act
    let report = migrate_memory(&home, &claude_home);

    // Assert: the move still ran (the new root already existed, so it takes
    // the copy path), and the self-heal still fires afterward. This scenario
    // only asserts existence/absence at the new root, not whose content
    // survives: `copy_all` overwrites the
    // new root's same-named `graph.json` with the old root's un-renamed
    // copy before the self-heal ever runs, so the surviving content is
    // whatever the old root held, not the independently-seeded value.
    fs::set_permissions(&mem_dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        !new_root.join("graph.json").exists(),
        "no stray graph.json must remain at the new root, regardless of which \
         origin it came from"
    );
    assert!(
        new_root.join("memory.graph.json").is_file(),
        "the new-root self-heal must still fire: {}",
        report.detail
    );

    let _ = fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn new_root_self_heal_fires_even_when_the_copy_step_itself_fails() {
    // Arrange: unlike the scenario above (where the move actually succeeds
    // via the copy path), this one forces `move_memory_root` to return
    // `Failed` outright: one source file is made unreadable, so `copy_all`
    // fails partway through and the whole step fails, while the new root
    // itself stays fully writable so the self-heal's own rename isn't
    // blocked by the same permission problem. An independent, pre-existing
    // stray `graph.json` sits at the new root before the call, seeded with
    // its own distinct content, exactly like the read-only-old-root scenario
    // above. The self-heal must still run and repair it: `migrate_memory`
    // calls `rename_legacy_graph_file_at_new_root` unconditionally, above
    // the `if moved.status == StepStatus::Failed { return ... }` early
    // return, not after it.
    use std::os::unix::fs::PermissionsExt;

    let home = scratch_home("new-root-stray-plus-failed-copy");
    let claude_home = claude_home_of(&home);
    let mem_dir = mem_dir_of(&claude_home);
    fs::create_dir_all(&mem_dir).unwrap();
    fs::write(old_graph_path(&claude_home), GRAPH_CONTENT).unwrap();
    // A second file the copy step must also carry over, made unreadable so
    // `fs::copy` fails on the read side. `graph.json` above stays readable:
    // if it were the only file, the copy step could still succeed and the
    // move would not actually fail.
    let unreadable = mem_dir.join("a-fact.md");
    fs::write(&unreadable, "some fact content").unwrap();
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
    let permissions_are_enforced = fs::read(&unreadable).is_err();
    if !permissions_are_enforced {
        eprintln!(
            "skipping new_root_self_heal_fires_even_when_the_copy_step_itself_fails: \
             running as a user that bypasses file permissions"
        );
        let _ = fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644));
        let _ = fs::remove_dir_all(&home);
        return;
    }

    let new_root = new_root_of(&home);
    fs::create_dir_all(&new_root).unwrap();
    fs::write(
        new_root.join("graph.json"),
        "independent new-root stray, seeded before the call",
    )
    .unwrap();

    // Act
    let report = migrate_memory(&home, &claude_home);

    // Assert: the move itself genuinely failed.
    let _ = fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644));
    assert_eq!(
        report.status,
        StepStatus::Failed,
        "expected the copy step to fail on the unreadable source file: {}",
        report.detail
    );
    // ...but the self-heal still fires afterward and repairs the
    // independently-seeded stray, proving it runs unconditionally rather
    // than only on a non-failed move.
    assert!(
        !new_root.join("graph.json").exists(),
        "no stray graph.json must remain at the new root even after a failed move"
    );
    assert!(
        new_root.join("memory.graph.json").is_file(),
        "the new-root self-heal must still fire after a failed move: {}",
        report.detail
    );

    let _ = fs::remove_dir_all(&home);
}
