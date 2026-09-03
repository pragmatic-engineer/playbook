// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Migrates `~/.claude/memory/graph.json` to `memory.graph.json`, and
//! migrates the whole `~/.claude/memory` tree to `$HOME/.config/playbook/memory`.

use crate::init::run::StepReport;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const STEP_NAME: &str = "memory-migrate";
const OLD_FILE_NAME: &str = "graph.json";
const NEW_FILE_NAME: &str = "memory.graph.json";
const OLD_LOCK_NAME: &str = "graph.json.lock";
const NEW_LOCK_NAME: &str = "memory.graph.json.lock";

/// Renames the pre-rename `graph.json` (and its mkdir-based `.lock`
/// sibling, if present) to `memory.graph.json`. An old file left alongside an already-migrated store is never touched or deleted.
pub fn migrate_memory_store(claude_home: &Path) -> StepReport {
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

const ROOT_STEP_NAME: &str = "memory-root-migrate";
/// Its presence is the only "migration complete" signal: a destination
/// left partially populated by an interrupted prior run has no sentinel.
const SENTINEL_NAME: &str = ".migration-complete";

/// Moves the whole `<claude_home>/memory` tree to `<home>/.config/playbook/memory`,
/// verifying a fallback copy before the sentinel is written and the source deleted.
pub fn migrate_memory_root(home: &Path, claude_home: &Path) -> StepReport {
    let old_root = claude_home.join("memory");
    let new_root = home.join(".config").join("playbook").join("memory");
    let sentinel = new_root.join(SENTINEL_NAME);

    if sentinel.is_file() {
        return StepReport::already_correct(
            ROOT_STEP_NAME,
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

    copy_verify_and_finish(&old_root, &new_root, &sentinel)
}

/// A bare destination means a fresh install, nothing to do; a populated
/// one with no sentinel is a prior rename that finished but never marked.
fn finish_when_source_absent(new_root: &Path, sentinel: &Path) -> StepReport {
    if !new_root.exists() {
        return StepReport::skipped(ROOT_STEP_NAME, "no legacy ~/.claude/memory to migrate");
    }
    match write_sentinel(sentinel) {
        Ok(()) => StepReport::wired(
            ROOT_STEP_NAME,
            format!(
                "{} already complete from a prior run; marked so",
                new_root.display()
            ),
        ),
        Err(err) => StepReport::failed(
            ROOT_STEP_NAME,
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
                ROOT_STEP_NAME,
                format!("could not create {}: {err}", parent.display()),
            ));
        }
    }
    match fs::rename(old_root, new_root) {
        Ok(()) => Some(match write_sentinel(sentinel) {
            Ok(()) => StepReport::wired(
                ROOT_STEP_NAME,
                format!("renamed {} to {}", old_root.display(), new_root.display()),
            ),
            Err(err) => StepReport::failed(
                ROOT_STEP_NAME,
                format!(
                    "moved to {} but could not write completion marker: {err}",
                    new_root.display()
                ),
            ),
        }),
        Err(err) if err.kind() == io::ErrorKind::CrossesDevices => None,
        Err(err) => Some(StepReport::failed(
            ROOT_STEP_NAME,
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
            ROOT_STEP_NAME,
            format!("could not create {}: {err}", new_root.display()),
        );
    }

    let files = relative_files(old_root);

    if let Err(err) = copy_all(old_root, new_root, &files) {
        return StepReport::failed(
            ROOT_STEP_NAME,
            format!(
                "copy to {} failed, the original is untouched: {err}",
                new_root.display()
            ),
        );
    }

    if !all_copied_and_verified(old_root, new_root, &files) {
        return StepReport::failed(
            ROOT_STEP_NAME,
            format!(
                "verification failed after copying to {}, the original is untouched",
                new_root.display()
            ),
        );
    }

    if let Err(err) = write_sentinel(sentinel) {
        return StepReport::failed(
            ROOT_STEP_NAME,
            format!("copy verified but could not write completion marker: {err}"),
        );
    }

    // Only after the destination is verified and marked complete is the
    // source removed; a leftover file after this point is harmless.
    let _ = fs::remove_dir_all(old_root);

    StepReport::wired(
        ROOT_STEP_NAME,
        format!(
            "copied {} file(s) to {} and removed the original",
            files.len(),
            new_root.display()
        ),
    )
}

/// Every regular file under `root`, recursively, as paths relative to
/// `root`; copies everything rather than filtering, unlike the markdown-only walk `rebuild_memory_graph` does.
fn relative_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    relative_files_into(root, root, &mut out);
    out
}

fn relative_files_into(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            relative_files_into(root, &path, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_path_buf());
        }
    }
}

/// Copies each of `files` from `old_root` to `new_root`, creating
/// destination subdirectories as needed, stopping at the first failure.
fn copy_all(old_root: &Path, new_root: &Path, files: &[PathBuf]) -> io::Result<()> {
    for rel in files {
        let dest = new_root.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(old_root.join(rel), &dest)?;
    }
    Ok(())
}

/// The completion check the sentinel's presence promises: every source
/// file exists at the destination with a matching size.
fn all_copied_and_verified(old_root: &Path, new_root: &Path, files: &[PathBuf]) -> bool {
    files.iter().all(|rel| {
        let source_len = fs::metadata(old_root.join(rel)).ok().map(|m| m.len());
        let dest_len = fs::metadata(new_root.join(rel)).ok().map(|m| m.len());
        source_len.is_some() && source_len == dest_len
    })
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

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
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
        let report = migrate_memory_root(&home, &claude_home);

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
        let report = migrate_memory_root(&home, &claude_home);

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
        let report = migrate_memory_root(&home, &claude_home);

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
        let report = migrate_memory_root(&home, &claude_home);

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
        let report = migrate_memory_root(&home, &claude_home);

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
        let report = migrate_memory_root(&home, &claude_home);

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
}
