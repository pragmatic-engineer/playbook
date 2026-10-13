// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Where a repo's plans, implementation notes, worktrees and handoffs live:
//! `~/.config/playbook/repos/<owner>/<repo>/<kind>/`, the same from every
//! checkout and every worktree. `<owner>/<repo>` comes from the `origin`
//! remote, so a person or an agent with no binary can work the path out with
//! `git remote get-url origin` alone.
//!
//! Older versions scoped these folders to one worktree
//! (`repos/<owner>/<repo>/<worktree-id>/<kind>`). The first call that needs a
//! folder copies what the old folders hold into the new one. Nothing is
//! deleted or overwritten: an old file stays where it was, and a `MOVED.txt`
//! note beside it says where the copy went and doubles as the "done" marker.

use crate::common::paths::{repo_scoped_dir, RepoScope};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The note left in an old folder once its files were copied.
const POINTER: &str = "MOVED.txt";

/// A storage folder under a repo's directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Plans,
    Implement,
    Worktrees,
    Handoff,
}

impl Kind {
    pub fn dir_name(self) -> &'static str {
        match self {
            Kind::Plans => "plans",
            Kind::Implement => "implement",
            Kind::Worktrees => "worktrees",
            Kind::Handoff => "handoff",
        }
    }

    /// Whether files in an old worktree-scoped folder are copied across. Not
    /// `worktrees`: it holds live git checkouts that point back at their
    /// repo, and a copy would be a second, broken checkout. Not `handoff`
    /// either: handoffs came from the runtime folder, see [`migrate_handoffs`].
    fn copies_from_worktree_dirs(self) -> bool {
        matches!(self, Kind::Plans | Kind::Implement)
    }
}

/// `repos/<owner>/<repo>/`, or `None` without a usable `origin` remote.
pub fn repo_dir() -> Option<PathBuf> {
    repo_scoped_dir(RepoScope::Repo)
}

/// The folder for `kind`, after copying anything older versions left in a
/// worktree-scoped folder. `None` without a usable `origin` remote.
pub fn kind_dir(kind: Kind) -> Option<PathBuf> {
    let base = repo_dir()?;
    if kind.copies_from_worktree_dirs() {
        if let Err(err) = migrate_worktree_dirs(&base, kind) {
            eprintln!(
                "playbook: could not copy the old {} folders: {err}",
                kind.dir_name()
            );
        }
    }
    Some(base.join(kind.dir_name()))
}

/// Old worktree folders sit next to the new ones, named by a slug of the
/// worktree's absolute path, which always starts with a dash.
fn worktree_dirs(base: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(base) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with('-'))
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// Copy `repos/<owner>/<repo>/<worktree-id>/<kind>/` from every old worktree
/// folder into `repos/<owner>/<repo>/<kind>/`, once per old folder.
pub fn migrate_worktree_dirs(base: &Path, kind: Kind) -> io::Result<()> {
    if !kind.copies_from_worktree_dirs() {
        return Ok(());
    }
    let new = base.join(kind.dir_name());
    for old in worktree_dirs(base)
        .into_iter()
        .map(|d| d.join(kind.dir_name()))
        .filter(|d| d.is_dir() && !d.join(POINTER).exists())
    {
        fs::create_dir_all(&new)?;
        let stats = copy_missing(&old, &new)?;
        write_pointer(&old, &new, &format!("{} files copied, {} kept apart because the new folder already had a different file with the same name", stats.copied, stats.clashed))?;
        if stats.copied > 0 || stats.clashed > 0 {
            eprintln!(
                "playbook: copied {} file(s) from {} to {}; the originals stay where they are",
                stats.copied,
                old.display(),
                new.display()
            );
        }
    }
    Ok(())
}

#[derive(Default)]
struct Copied {
    copied: usize,
    clashed: usize,
}

/// Copy regular files from `from` into `to`, recursively. A name that already
/// exists in `to` is left alone, so a copy never overwrites newer work.
fn copy_missing(from: &Path, to: &Path) -> io::Result<Copied> {
    let mut stats = Copied::default();
    for entry in fs::read_dir(from)?.flatten() {
        let name = entry.file_name();
        if name == POINTER {
            continue;
        }
        let kind = entry.file_type()?;
        let target = to.join(&name);
        if kind.is_dir() {
            fs::create_dir_all(&target)?;
            let inner = copy_missing(&entry.path(), &target)?;
            stats.copied += inner.copied;
            stats.clashed += inner.clashed;
        } else if kind.is_file() {
            if copy_new(&entry.path(), &target)? {
                stats.copied += 1;
            } else if !same_bytes(&entry.path(), &target) {
                stats.clashed += 1;
            }
        }
    }
    Ok(stats)
}

/// Copy `from` to `to` unless `to` exists. The bytes land in a temporary file
/// first and a hard link puts them in place, so a reader never sees half a
/// file and two processes cannot both win.
fn copy_new(from: &Path, to: &Path) -> io::Result<bool> {
    if to.exists() {
        return Ok(false);
    }
    let tmp = to.with_extension(format!("copying-{}", std::process::id()));
    fs::copy(from, &tmp)?;
    let linked = fs::hard_link(&tmp, to);
    let _ = fs::remove_file(&tmp);
    match linked {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(err) => Err(err),
    }
}

fn same_bytes(a: &Path, b: &Path) -> bool {
    matches!((fs::read(a), fs::read(b)), (Ok(x), Ok(y)) if x == y)
}

fn write_pointer(old: &Path, new: &Path, summary: &str) -> io::Result<()> {
    let mut file = fs::File::create(old.join(POINTER))?;
    writeln!(
        file,
        "playbook keeps this folder at {} now, the same from every worktree of the repo.\n\
         Files here were copied there, not moved or deleted: {summary}.\n\
         This note marks the copy as done. It is safe to delete the folder when you no longer need it.",
        new.display()
    )
}

/// The old, global handoff folder: one folder for every directory, keyed by
/// the directory's slug in each file name.
pub fn legacy_handoff_root() -> PathBuf {
    crate::common::paths::runtime_root().join("handoff")
}

/// The handoff folder for the current repo, or the old global one when the
/// current directory has no usable `origin` remote. Copies this directory's
/// handoffs across the first time.
pub fn handoff_root(slug: &str) -> PathBuf {
    let legacy = legacy_handoff_root();
    let Some(base) = repo_dir() else {
        return legacy;
    };
    let new = base.join(Kind::Handoff.dir_name());
    if let Err(err) = migrate_handoffs(&legacy, &new, slug) {
        eprintln!("playbook: could not copy the old handoffs: {err}");
    }
    new
}

/// Copy the handoffs for directory `slug` (and the directories under it) from
/// the old global folder into `new`, once per `slug`. The old files stay; a
/// marker in `new` stops a handoff that a session already consumed from
/// coming back.
pub fn migrate_handoffs(legacy: &Path, new: &Path, slug: &str) -> io::Result<()> {
    if slug.is_empty() || !legacy.is_dir() {
        return Ok(());
    }
    let marker = new.join(".copied").join(slug);
    if marker.exists() {
        return Ok(());
    }
    fs::create_dir_all(new)?;
    let prefix = format!("{slug}-");
    let mut copied = 0;
    for entry in fs::read_dir(legacy)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".md")
            && name.starts_with(&prefix)
            && entry.file_type()?.is_file()
            && copy_new(&entry.path(), &new.join(&name))?
        {
            copied += 1;
        }
    }
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&marker, "")?;
    if copied > 0 {
        let pointer = legacy.join(POINTER);
        if !pointer.exists() {
            write_pointer(
                legacy,
                new,
                "handoffs for a repo's directories are copied there on first use",
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn plans_and_implement_files_are_copied_from_every_old_worktree_folder() {
        let base = scratch_dir("storage-copy");
        write(&base.join("-a-wt1/plans/one.md"), "one");
        write(&base.join("-a-wt2/plans/two.md"), "two");
        write(&base.join("-a-wt2/plans/sub/three.md"), "three");

        migrate_worktree_dirs(&base, Kind::Plans).unwrap();

        assert_eq!(
            fs::read_to_string(base.join("plans/one.md")).unwrap(),
            "one"
        );
        assert_eq!(
            fs::read_to_string(base.join("plans/two.md")).unwrap(),
            "two"
        );
        assert_eq!(
            fs::read_to_string(base.join("plans/sub/three.md")).unwrap(),
            "three"
        );
    }

    #[test]
    fn the_old_files_stay_and_a_pointer_marks_the_copy_done() {
        let base = scratch_dir("storage-pointer");
        write(&base.join("-a-wt1/plans/one.md"), "one");

        migrate_worktree_dirs(&base, Kind::Plans).unwrap();

        assert!(base.join("-a-wt1/plans/one.md").is_file());
        let note = fs::read_to_string(base.join("-a-wt1/plans/MOVED.txt")).unwrap();
        assert!(
            note.contains(&base.join("plans").display().to_string()),
            "{note}"
        );
        assert!(!base.join("plans/MOVED.txt").exists());
    }

    #[test]
    fn a_second_run_does_not_bring_back_a_file_that_was_removed_from_the_new_folder() {
        let base = scratch_dir("storage-once");
        write(&base.join("-a-wt1/plans/one.md"), "one");
        migrate_worktree_dirs(&base, Kind::Plans).unwrap();
        fs::remove_file(base.join("plans/one.md")).unwrap();

        migrate_worktree_dirs(&base, Kind::Plans).unwrap();

        assert!(!base.join("plans/one.md").exists());
    }

    #[test]
    fn an_existing_file_is_never_overwritten_and_a_clash_is_counted() {
        let base = scratch_dir("storage-clash");
        write(&base.join("plans/one.md"), "new work");
        write(&base.join("-a-wt1/plans/one.md"), "old work");

        migrate_worktree_dirs(&base, Kind::Plans).unwrap();

        assert_eq!(
            fs::read_to_string(base.join("plans/one.md")).unwrap(),
            "new work"
        );
        assert_eq!(
            fs::read_to_string(base.join("-a-wt1/plans/one.md")).unwrap(),
            "old work"
        );
        let note = fs::read_to_string(base.join("-a-wt1/plans/MOVED.txt")).unwrap();
        assert!(note.contains("1 kept apart"), "{note}");
    }

    #[test]
    fn worktree_checkouts_are_never_copied() {
        let base = scratch_dir("storage-worktrees");
        write(&base.join("-a-wt1/worktrees/branch/file.rs"), "code");

        migrate_worktree_dirs(&base, Kind::Worktrees).unwrap();

        assert!(!base.join("worktrees/branch").exists());
        assert!(!base.join("-a-wt1/worktrees/MOVED.txt").exists());
    }

    #[test]
    fn handoffs_for_this_directory_and_its_children_are_copied_once() {
        let root = scratch_dir("storage-handoff");
        let legacy = root.join("legacy");
        let new = root.join("new");
        write(&legacy.join("-w-repo-1-2.md"), "here");
        write(&legacy.join("-w-repo-sub-1-2.md"), "child");
        write(&legacy.join("-w-other-1-2.md"), "elsewhere");

        migrate_handoffs(&legacy, &new, "-w-repo").unwrap();
        fs::remove_file(new.join("-w-repo-1-2.md")).unwrap();
        migrate_handoffs(&legacy, &new, "-w-repo").unwrap();

        assert!(
            !new.join("-w-repo-1-2.md").exists(),
            "a consumed handoff came back"
        );
        assert!(new.join("-w-repo-sub-1-2.md").is_file());
        assert!(!new.join("-w-other-1-2.md").exists());
        assert!(legacy.join("-w-repo-1-2.md").is_file());
        assert!(legacy.join(POINTER).is_file());
    }
}
