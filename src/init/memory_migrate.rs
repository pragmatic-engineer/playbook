// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Migrates the whole memory store as a single reported step named `memory`:
//! renames `~/.claude/memory/graph.json` to `memory.graph.json` if the old
//! filename is still present, then moves the whole `~/.claude/memory` tree to
//! `$HOME/.config/playbook/memory`, then self-heals a stray `graph.json` left
//! behind at that new location. The rename is an internal precondition the
//! move needs (it copies whatever filenames are present, so a move before
//! the rename would strand the old filename at the new location), not a
//! separate migration a caller should reason about. The new-root self-heal
//! is likewise folded in here rather than left to each caller: this is the
//! one place every entry point (`playbook init`, SessionStart, Stop) gets
//! the full set of legacy-filename fixes, not just the ones a particular
//! caller happened to add.

use crate::common::atomic::with_dir_lock;
use crate::common::paths::playbook_root_from;
use crate::init::run::{StepReport, StepStatus};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

const STEP_NAME: &str = "memory";
const OLD_FILE_NAME: &str = "graph.json";
const NEW_FILE_NAME: &str = "memory.graph.json";
const OLD_LOCK_NAME: &str = "graph.json.lock";
const NEW_LOCK_NAME: &str = "memory.graph.json.lock";
/// Its presence is the only "migration complete" signal: a destination
/// left partially populated by an interrupted prior run has no sentinel.
const SENTINEL_NAME: &str = ".migration-complete";

/// Migrates the whole memory store in one step: renames the legacy filename
/// first (a fast no-op once already renamed), then moves the tree to its
/// final location (a fast no-op once the sentinel is present), then
/// self-heals a stray legacy filename left at the new root. Safe to call
/// repeatedly and from any entry point (`playbook init`, SessionStart, or
/// Stop): every call after the first is a cheap check, not a re-copy.
///
/// A failed rename does not abort the move: the move copies whatever
/// filename is present either way, so a legacy `graph.json` that could not
/// be renamed (a read-only source, say) still reaches the new location,
/// just under its old name. The new-root self-heal runs regardless of the
/// move's own outcome, including a failed one: it repairs whatever stray
/// old name it finds at the new root, however it got there, independent of
/// whether this particular call is the one that put it there. Aborting the
/// whole step on a failed move would instead leave every reader looking at
/// a store the move never even attempted to bring over.
pub fn migrate_memory(home: &Path, claude_home: &Path) -> StepReport {
    let rename = rename_legacy_graph_file(claude_home);
    let moved = move_memory_root(home, claude_home);
    rename_legacy_graph_file_at_new_root(home);

    if moved.status == StepStatus::Failed {
        return StepReport::failed(STEP_NAME, moved.detail);
    }

    let detail = match rename.status {
        StepStatus::Wired => format!(
            "renamed legacy graph.json to memory.graph.json; {}",
            moved.detail
        ),
        StepStatus::Failed => format!(
            "could not rename legacy graph.json ({}); {}",
            rename.detail, moved.detail
        ),
        _ => moved.detail,
    };
    match combined_status(rename.status, moved.status) {
        StepStatus::Wired => StepReport::wired(STEP_NAME, detail),
        StepStatus::AlreadyCorrect => StepReport::already_correct(STEP_NAME, detail),
        StepStatus::Skipped => StepReport::skipped(STEP_NAME, detail),
        StepStatus::Failed => {
            unreachable!("moved.status == Failed already returned above")
        }
    }
}

/// The overall status once `moved.status` is known not to be `Failed`
/// (`migrate_memory` returns early on that case). `Wired` beats
/// `AlreadyCorrect` beats `Skipped`: if either sub-step actually did
/// something, the step as a whole did something. A failed rename alone
/// never wins here (see `migrate_memory`'s doc comment), it only ever
/// changes the detail string.
fn combined_status(rename: StepStatus, moved: StepStatus) -> StepStatus {
    match (rename, moved) {
        (StepStatus::Wired, _) | (_, StepStatus::Wired) => StepStatus::Wired,
        (StepStatus::AlreadyCorrect, _) | (_, StepStatus::AlreadyCorrect) => {
            StepStatus::AlreadyCorrect
        }
        _ => StepStatus::Skipped,
    }
}

/// Renames the pre-rename `graph.json` (and its mkdir-based `.lock`
/// sibling, if present) to `memory.graph.json`. An old file left alongside
/// an already-migrated store is never touched or deleted.
fn rename_legacy_graph_file(claude_home: &Path) -> StepReport {
    let mem_dir = claude_home.join("memory");
    let old_path = mem_dir.join(OLD_FILE_NAME);
    let new_path = mem_dir.join(NEW_FILE_NAME);

    if new_path.exists() {
        return StepReport::already_correct(
            STEP_NAME,
            format!("already at {}", new_path.display()),
        );
    }

    if !old_path.exists() {
        return StepReport::skipped(STEP_NAME, "no legacy graph.json to migrate");
    }

    if let Err(err) = fs::rename(&old_path, &new_path) {
        return StepReport::failed(
            STEP_NAME,
            format!(
                "could not rename {} to {}: {err}",
                old_path.display(),
                new_path.display()
            ),
        );
    }

    let old_lock = mem_dir.join(OLD_LOCK_NAME);
    if old_lock.exists() {
        let _ = fs::rename(&old_lock, mem_dir.join(NEW_LOCK_NAME));
    }

    StepReport::wired(
        STEP_NAME,
        format!("renamed {} to {}", old_path.display(), new_path.display()),
    )
}

/// Best-effort self-heal for a stray `graph.json` found at the NEW root
/// (`<home>/.config/playbook/memory`) after the move above, distinct from
/// `rename_legacy_graph_file`'s own OLD-root check. A stray new-root file
/// can arrive by paths the move itself never touches (an interrupted older
/// build, a manual copy), so this is not redundant with the move's own
/// copy step. It is silent and ignores every failure, since it is a
/// defensive fixup, not a step whose outcome callers need to react to.
///
/// Deliberately does not rename a `graph.json.lock` sibling the way
/// `rename_legacy_graph_file` does for the old root: nothing ever reads or
/// takes that lock under its legacy name, only `memory.graph.json.lock`
/// (`rebuild_memory_graph.rs`), so a stray old-named lock directory left
/// behind here is inert, not a bug.
///
/// Not lock-protected against a concurrent `rebuild_memory_graph` write
/// under `memory.graph.json.lock`: a rebuild landing in the narrow window
/// between the `exists()` check and the rename below could be overwritten
/// by this stale file. That race predates this function (the fallback it
/// replaces had it too); it needs a stray legacy file to be present at all,
/// and the graph rebuilds from the fact files regardless, so it's accepted
/// here rather than added to for what is meant to stay a cheap check.
fn rename_legacy_graph_file_at_new_root(home: &Path) {
    let mem_dir = playbook_root_from(home).join("memory");
    let old_path = mem_dir.join(OLD_FILE_NAME);
    let new_path = mem_dir.join(NEW_FILE_NAME);

    if new_path.exists() || !old_path.is_file() {
        return;
    }

    let _ = fs::rename(&old_path, &new_path);
}

/// Moves the whole `<claude_home>/memory` tree to
/// `<home>/.config/playbook/memory`, verifying a fallback copy before the
/// sentinel is written and the source deleted.
fn move_memory_root(home: &Path, claude_home: &Path) -> StepReport {
    let old_root = claude_home.join("memory");
    let new_root = playbook_root_from(home).join("memory");
    let sentinel = new_root.join(SENTINEL_NAME);

    if sentinel.is_file() {
        return StepReport::already_correct(
            STEP_NAME,
            format!("already migrated to {}", new_root.display()),
        );
    }

    if !old_root.exists() {
        return finish_when_source_absent(&new_root, &sentinel);
    }

    if !new_root.exists() {
        if let Some(report) = try_rename(&old_root, &new_root, &sentinel) {
            return report;
        }
    }

    // The lock lives under new_root, so new_root must exist before it is
    // acquired: acquiring first would burn the whole retry budget on
    // `NotFound` on a fresh cross-device migration, where new_root doesn't
    // exist yet at this point.
    if !new_root.exists() {
        if let Err(err) = fs::create_dir_all(&new_root) {
            return StepReport::failed(
                STEP_NAME,
                format!("could not create {}: {err}", new_root.display()),
            );
        }
    }

    let lock_path = new_root.join(NEW_LOCK_NAME);
    let (acquired, report) = with_dir_lock(&lock_path, 50, Duration::from_millis(10), || {
        copy_verify_and_finish(&old_root, &new_root, &sentinel)
    });
    if acquired {
        let _ = fs::remove_dir(&lock_path);
    }
    report
}

/// A bare destination means a fresh install, nothing to do; a populated
/// one with no sentinel is a prior rename that finished but never marked.
fn finish_when_source_absent(new_root: &Path, sentinel: &Path) -> StepReport {
    if !new_root.exists() {
        return StepReport::skipped(STEP_NAME, "no legacy ~/.claude/memory to migrate");
    }
    match write_sentinel(sentinel) {
        Ok(()) => StepReport::wired(
            STEP_NAME,
            format!(
                "{} already complete from a prior run; marked so",
                new_root.display()
            ),
        ),
        Err(err) => StepReport::failed(
            STEP_NAME,
            format!(
                "could not write completion marker {}: {err}",
                sentinel.display()
            ),
        ),
    }
}

/// `Some` is a final report (rename succeeded or failed outright); `None`
/// means fall through to the verified copy after a cross-device error.
fn try_rename(old_root: &Path, new_root: &Path, sentinel: &Path) -> Option<StepReport> {
    if let Some(parent) = new_root.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            return Some(StepReport::failed(
                STEP_NAME,
                format!("could not create {}: {err}", parent.display()),
            ));
        }
    }
    match fs::rename(old_root, new_root) {
        Ok(()) => Some(match write_sentinel(sentinel) {
            Ok(()) => StepReport::wired(
                STEP_NAME,
                format!("renamed {} to {}", old_root.display(), new_root.display()),
            ),
            Err(err) => StepReport::failed(
                STEP_NAME,
                format!(
                    "moved to {} but could not write completion marker: {err}",
                    new_root.display()
                ),
            ),
        }),
        Err(err) if err.kind() == io::ErrorKind::CrossesDevices => None,
        Err(err) => Some(StepReport::failed(
            STEP_NAME,
            format!(
                "could not rename {} to {}: {err}",
                old_root.display(),
                new_root.display()
            ),
        )),
    }
}

/// The resume path for a destination a prior interrupted run already
/// touched, and the cross-device fallback for a fresh migration.
fn copy_verify_and_finish(old_root: &Path, new_root: &Path, sentinel: &Path) -> StepReport {
    if let Err(err) = fs::create_dir_all(new_root) {
        return StepReport::failed(
            STEP_NAME,
            format!("could not create {}: {err}", new_root.display()),
        );
    }

    let (files, skipped_symlinks) = match relative_files(old_root) {
        Ok(result) => result,
        Err(err) => {
            return StepReport::failed(
                STEP_NAME,
                format!("could not enumerate {}: {err}", old_root.display()),
            );
        }
    };

    if let Err(err) = copy_all(old_root, new_root, &files) {
        return StepReport::failed(
            STEP_NAME,
            with_skipped_symlinks(
                format!(
                    "copy to {} failed, the original is untouched: {err}",
                    new_root.display()
                ),
                &skipped_symlinks,
            ),
        );
    }

    if let Err(relpath) = all_copied_and_verified(old_root, new_root, &files) {
        return StepReport::failed(
            STEP_NAME,
            with_skipped_symlinks(
                format!(
                    "verification failed after copying to {}, the original is untouched; \
                     {} differs and was not overwritten",
                    new_root.display(),
                    relpath.display()
                ),
                &skipped_symlinks,
            ),
        );
    }

    if let Err(err) = write_sentinel(sentinel) {
        return StepReport::failed(
            STEP_NAME,
            with_skipped_symlinks(
                format!("copy verified but could not write completion marker: {err}"),
                &skipped_symlinks,
            ),
        );
    }

    // Only after the destination is verified and marked complete is the
    // source removed; a leftover file after this point is harmless.
    let _ = fs::remove_dir_all(old_root);

    StepReport::wired(
        STEP_NAME,
        with_skipped_symlinks(
            format!(
                "copied {} file(s) to {} and removed the original",
                files.len(),
                new_root.display()
            ),
            &skipped_symlinks,
        ),
    )
}

/// Folds a non-empty `skipped_symlinks` list into `detail`, so a symlink
/// skipped during the walk stays visible in the returned `StepReport`
/// regardless of which of `copy_verify_and_finish`'s return paths produced it.
fn with_skipped_symlinks(detail: String, skipped_symlinks: &[PathBuf]) -> String {
    if skipped_symlinks.is_empty() {
        return detail;
    }
    let paths = skipped_symlinks
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{detail}; skipped {} symlink(s): {paths}",
        skipped_symlinks.len()
    )
}

/// Every regular file under `root`, recursively, as paths relative to
/// `root`, plus the relative paths of any symlinks the walk skipped rather
/// than followed. `Err` names the subdirectory that could not be read.
fn relative_files(root: &Path) -> io::Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let mut out = Vec::new();
    let mut skipped_symlinks = Vec::new();
    relative_files_into(root, root, &mut out, &mut skipped_symlinks)?;
    Ok((out, skipped_symlinks))
}

fn relative_files_into(
    root: &Path,
    dir: &Path,
    out: &mut Vec<PathBuf>,
    skipped_symlinks: &mut Vec<PathBuf>,
) -> io::Result<()> {
    // A bare `io::Error` carries no path, so wrapping it here is what lets
    // the caller name the specific failing subdirectory.
    let entries = fs::read_dir(dir)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", dir.display())))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = entry.file_type()?;
        // `file_type()` reports the entry itself, unlike `path.is_dir()`,
        // which follows a symlink to check its target instead.
        if file_type.is_symlink() {
            if let Ok(rel) = path.strip_prefix(root) {
                skipped_symlinks.push(rel.to_path_buf());
            }
        } else if file_type.is_dir() {
            relative_files_into(root, &path, out, skipped_symlinks)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_path_buf());
        }
    }
    Ok(())
}

/// Copies each of `files` from `old_root` to `new_root`, creating
/// destination subdirectories as needed and stopping at the first I/O
/// failure. Leaves an existing destination file untouched, rather than
/// overwriting it, when that destination is already at least as new as the
/// source.
fn copy_all(old_root: &Path, new_root: &Path, files: &[PathBuf]) -> io::Result<()> {
    // Test seam: lets a test observe an in-flight copy (e.g. that the
    // migration lock is held for its duration) without slowing production
    // runs, which never set this variable.
    let test_copy_delay = std::env::var("PLAYBOOK_TEST_COPY_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis);

    for rel in files {
        let dest = new_root.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        if dest.exists() {
            let source_mtime = fs::metadata(old_root.join(rel))?.modified()?;
            let dest_mtime = fs::metadata(&dest)?.modified()?;
            // A tie counts as "not older", not just a strictly newer mtime:
            // on a filesystem with coarse (1-second) mtime resolution, a
            // hook write and a migration retry can land in the same tick,
            // and a strict `>` here would silently overwrite that write.
            if dest_mtime >= source_mtime {
                continue;
            }
        }
        if let Some(delay) = test_copy_delay {
            std::thread::sleep(delay);
        }
        fs::copy(old_root.join(rel), &dest)?;
    }
    Ok(())
}

/// The completion check the sentinel's presence promises: every source file
/// exists at the destination with byte-identical content, not just a
/// matching size. `Err` names the first relative path that does not match,
/// e.g. one a skipped-copy left holding an unrelated, newer write instead of
/// the source's content.
fn all_copied_and_verified(
    old_root: &Path,
    new_root: &Path,
    files: &[PathBuf],
) -> Result<(), PathBuf> {
    for rel in files {
        let source = fs::read(old_root.join(rel));
        let dest = fs::read(new_root.join(rel));
        if !matches!((source, dest), (Ok(s), Ok(d)) if s == d) {
            return Err(rel.clone());
        }
    }
    Ok(())
}

fn write_sentinel(sentinel: &Path) -> io::Result<()> {
    if let Some(parent) = sentinel.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(sentinel, "migrated\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use crate::init::run::StepStatus;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, SystemTime};

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    /// Sets an explicit, deterministic mtime, never relying on ordering from
    /// a `sleep` (some filesystems have 1-second mtime resolution).
    fn set_mtime(path: &Path, time: SystemTime) {
        let file = fs::File::open(path).unwrap();
        file.set_modified(time).unwrap();
    }

    fn new_root_of(home: &Path) -> PathBuf {
        home.join(".config").join("playbook").join("memory")
    }

    #[test]
    fn migration_moves_global_and_project_facts_byte_identical() {
        // Arrange: one global fact and two project scopes, plus the graph file.
        let home = scratch_dir("migrate-root-byte-identical");
        let claude_home = home.join(".claude");
        let old_root = claude_home.join("memory");
        write_file(&old_root.join("global-fact.md"), "global fact content");
        write_file(
            &old_root.join("owner-repo-one").join("fact-a.md"),
            "project one fact",
        );
        write_file(
            &old_root.join("owner-repo-two").join("fact-b.md"),
            "project two fact",
        );
        write_file(&old_root.join("memory.graph.json"), r#"{"nodes":[]}"#);

        // Act
        let report = migrate_memory(&home, &claude_home);

        // Assert
        assert_eq!(report.status, StepStatus::Wired, "{}", report.detail);
        let new_root = new_root_of(&home);
        assert_eq!(
            fs::read_to_string(new_root.join("global-fact.md")).unwrap(),
            "global fact content"
        );
        assert_eq!(
            fs::read_to_string(new_root.join("owner-repo-one").join("fact-a.md")).unwrap(),
            "project one fact"
        );
        assert_eq!(
            fs::read_to_string(new_root.join("owner-repo-two").join("fact-b.md")).unwrap(),
            "project two fact"
        );
        assert_eq!(
            fs::read_to_string(new_root.join("memory.graph.json")).unwrap(),
            r#"{"nodes":[]}"#
        );
        assert!(new_root.join(SENTINEL_NAME).is_file());
        assert!(
            !old_root.exists(),
            "the original should be gone after a successful move"
        );

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn migration_is_a_noop_when_sentinel_already_present() {
        // Arrange: sentinel present, plus stray content in both trees that a no-op must not touch.
        let home = scratch_dir("migrate-root-noop-sentinel");
        let claude_home = home.join(".claude");
        let old_root = claude_home.join("memory");
        write_file(&old_root.join("leftover.md"), "leftover old content");
        let new_root = new_root_of(&home);
        write_file(
            &new_root.join("already-there.md"),
            "already migrated content",
        );
        write_file(&new_root.join(SENTINEL_NAME), "migrated\n");

        // Act
        let report = migrate_memory(&home, &claude_home);

        // Assert
        assert_eq!(
            report.status,
            StepStatus::AlreadyCorrect,
            "{}",
            report.detail
        );
        assert_eq!(
            fs::read_to_string(old_root.join("leftover.md")).unwrap(),
            "leftover old content"
        );
        assert_eq!(
            fs::read_to_string(new_root.join("already-there.md")).unwrap(),
            "already migrated content"
        );
        assert!(!new_root.join("leftover.md").exists());

        let _ = fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn migration_leaves_source_untouched_on_simulated_copy_failure() {
        // Arrange: an empty pre-existing destination, made read-only so the
        // resume/copy path fails on its first write, simulating a crash.
        let home = scratch_dir("migrate-root-copy-failure");
        let claude_home = home.join(".claude");
        let old_root = claude_home.join("memory");
        write_file(&old_root.join("fact.md"), "fact content, must survive");
        let new_root = new_root_of(&home);
        fs::create_dir_all(&new_root).unwrap();

        fs::set_permissions(&new_root, fs::Permissions::from_mode(0o555)).unwrap();
        // A test running as root bypasses Unix permission checks, which
        // would make this write unexpectedly succeed; guard against that.
        let probe = new_root.join(".write-probe");
        let permissions_are_enforced = fs::write(&probe, "x").is_err();
        let _ = fs::set_permissions(&new_root, fs::Permissions::from_mode(0o755));
        let _ = fs::remove_file(&probe);
        if !permissions_are_enforced {
            eprintln!(
                "skipping migration_leaves_source_untouched_on_simulated_copy_failure: \
                 running as a user that bypasses directory permissions"
            );
            return;
        }
        fs::set_permissions(&new_root, fs::Permissions::from_mode(0o555)).unwrap();

        // Act
        let report = migrate_memory(&home, &claude_home);

        // Assert
        fs::set_permissions(&new_root, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(report.status, StepStatus::Failed, "{}", report.detail);
        assert_eq!(
            fs::read_to_string(old_root.join("fact.md")).unwrap(),
            "fact content, must survive",
            "the original must survive a mid-copy failure untouched"
        );
        assert!(old_root.exists());
        assert!(!new_root.join(SENTINEL_NAME).exists());

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn no_migration_needed_when_old_location_never_existed() {
        // Arrange: a fresh install, no legacy memory tree at all.
        let home = scratch_dir("migrate-root-fresh-install");
        let claude_home = home.join(".claude");

        // Act
        let report = migrate_memory(&home, &claude_home);

        // Assert
        assert_eq!(report.status, StepStatus::Skipped, "{}", report.detail);
        assert!(!new_root_of(&home).exists());

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn migration_resumes_when_destination_partially_populated_and_sentinel_absent() {
        // Arrange: old fully populated, new holds only a strict subset, no sentinel.
        let home = scratch_dir("migrate-root-resume");
        let claude_home = home.join(".claude");
        let old_root = claude_home.join("memory");
        write_file(&old_root.join("global-fact.md"), "global fact content");
        write_file(
            &old_root.join("owner-repo-one").join("fact-a.md"),
            "project one fact",
        );
        write_file(
            &old_root.join("owner-repo-two").join("fact-b.md"),
            "project two fact",
        );
        let new_root = new_root_of(&home);
        write_file(&new_root.join("global-fact.md"), "global fact content");

        // Act
        let report = migrate_memory(&home, &claude_home);

        // Assert
        assert_eq!(report.status, StepStatus::Wired, "{}", report.detail);
        assert_eq!(
            fs::read_to_string(new_root.join("global-fact.md")).unwrap(),
            "global fact content"
        );
        assert_eq!(
            fs::read_to_string(new_root.join("owner-repo-one").join("fact-a.md")).unwrap(),
            "project one fact"
        );
        assert_eq!(
            fs::read_to_string(new_root.join("owner-repo-two").join("fact-b.md")).unwrap(),
            "project two fact"
        );
        assert!(new_root.join(SENTINEL_NAME).is_file());
        assert!(
            !old_root.exists(),
            "a completed resume should remove the original"
        );

        let _ = fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn migration_never_marks_complete_until_resumed_copy_is_verified() {
        // Arrange: the same partial-destination resume setup, but the
        // destination is read-only so copying the missing file fails.
        let home = scratch_dir("migrate-root-resume-never-premature");
        let claude_home = home.join(".claude");
        let old_root = claude_home.join("memory");
        write_file(&old_root.join("global-fact.md"), "global fact content");
        write_file(
            &old_root.join("owner-repo-one").join("fact-a.md"),
            "project one fact",
        );
        let new_root = new_root_of(&home);
        write_file(&new_root.join("global-fact.md"), "global fact content");

        fs::set_permissions(&new_root, fs::Permissions::from_mode(0o555)).unwrap();
        let probe = new_root.join(".write-probe");
        let permissions_are_enforced = fs::write(&probe, "x").is_err();
        let _ = fs::set_permissions(&new_root, fs::Permissions::from_mode(0o755));
        let _ = fs::remove_file(&probe);
        if !permissions_are_enforced {
            eprintln!(
                "skipping migration_never_marks_complete_until_resumed_copy_is_verified: \
                 running as a user that bypasses directory permissions"
            );
            return;
        }
        fs::set_permissions(&new_root, fs::Permissions::from_mode(0o555)).unwrap();

        // Act
        let report = migrate_memory(&home, &claude_home);

        // Assert
        fs::set_permissions(&new_root, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(report.status, StepStatus::Failed, "{}", report.detail);
        assert!(
            !new_root.join(SENTINEL_NAME).exists(),
            "the sentinel must never be written until the resumed copy verifies complete"
        );
        assert_eq!(
            fs::read_to_string(old_root.join("global-fact.md")).unwrap(),
            "global fact content"
        );
        assert_eq!(
            fs::read_to_string(old_root.join("owner-repo-one").join("fact-a.md")).unwrap(),
            "project one fact"
        );
        assert!(
            old_root.exists(),
            "the original must survive an interrupted resume"
        );

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn copy_all_preserves_a_strictly_newer_destination_file() {
        // Arrange: source and destination both hold the file, destination's
        // mtime set strictly ahead of the source's, with distinguishable
        // content on each side so a buggy overwrite is easy to spot.
        let home = scratch_dir("copy-all-newer-dest-preserved");
        let old_root = home.join("old");
        let new_root = home.join("new");
        write_file(&old_root.join("fact.md"), "source content");
        write_file(
            &new_root.join("fact.md"),
            "destination content, must survive",
        );
        let base = SystemTime::now();
        set_mtime(&old_root.join("fact.md"), base);
        set_mtime(&new_root.join("fact.md"), base + Duration::from_secs(10));

        // Act
        let result = copy_all(&old_root, &new_root, &[PathBuf::from("fact.md")]);

        // Assert
        assert!(result.is_ok(), "{:?}", result);
        assert_eq!(
            fs::read_to_string(new_root.join("fact.md")).unwrap(),
            "destination content, must survive",
            "a strictly newer destination must not be overwritten"
        );

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn copy_all_preserves_a_tied_mtime_destination_file() {
        // Arrange: destination's mtime is EXACTLY equal to the source's,
        // pinning the >= vs > boundary: a strict > check would overwrite
        // this, since a tie is not "greater than".
        let home = scratch_dir("copy-all-tied-mtime-dest-preserved");
        let old_root = home.join("old");
        let new_root = home.join("new");
        write_file(&old_root.join("fact.md"), "source content");
        write_file(
            &new_root.join("fact.md"),
            "destination content, must survive",
        );
        let tied = SystemTime::now();
        set_mtime(&old_root.join("fact.md"), tied);
        set_mtime(&new_root.join("fact.md"), tied);

        // Act
        let result = copy_all(&old_root, &new_root, &[PathBuf::from("fact.md")]);

        // Assert
        assert!(result.is_ok(), "{:?}", result);
        assert_eq!(
            fs::read_to_string(new_root.join("fact.md")).unwrap(),
            "destination content, must survive",
            "a tied mtime must not be overwritten"
        );

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn copy_all_overwrites_an_older_destination_file() {
        // Arrange: destination's mtime is strictly behind the source's, the
        // common case a migration retry needs to catch up on.
        let home = scratch_dir("copy-all-older-dest-overwritten");
        let old_root = home.join("old");
        let new_root = home.join("new");
        write_file(&old_root.join("fact.md"), "source content, must win");
        write_file(&new_root.join("fact.md"), "stale destination content");
        let base = SystemTime::now();
        set_mtime(&new_root.join("fact.md"), base);
        set_mtime(&old_root.join("fact.md"), base + Duration::from_secs(10));

        // Act
        let result = copy_all(&old_root, &new_root, &[PathBuf::from("fact.md")]);

        // Assert
        assert!(result.is_ok(), "{:?}", result);
        assert_eq!(
            fs::read_to_string(new_root.join("fact.md")).unwrap(),
            "source content, must win",
            "an older destination must still be overwritten with the source's content"
        );

        let _ = fs::remove_dir_all(&home);
    }
}
