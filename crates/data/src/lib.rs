// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Shared storage for the playbook crates. Every database any playbook tool
//! creates lives in one folder, `~/.config/playbook/data`:
//!
//! * [`data_dir`] and the per-repo file helpers name that folder and the files
//!   in it.
//! * [`migrate`] moves a database from its old path into the folder without
//!   ever deleting it.

pub mod migrate;

use playbook_common::atomic::ensure_private_dir;
use playbook_common::paths::{playbook_root_from, worktree_id};
use playbook_common::repo::repo_slug;
use playbook_common::session::home_dir;
use std::path::{Path, PathBuf};

/// `$HOME/.config/playbook/data`.
pub fn data_dir() -> PathBuf {
    data_dir_from(&home_dir())
}

/// Core of [`data_dir`], taking `home` explicitly for tests.
pub fn data_dir_from(home: &Path) -> PathBuf {
    playbook_root_from(home).join("data")
}

/// [`data_dir`], created owner only when missing.
pub fn ensure_data_dir() -> std::io::Result<PathBuf> {
    let dir = data_dir();
    ensure_private_dir(&dir)?;
    Ok(dir)
}

/// `<data>/<kind>/<owner>/<repo>/<worktree-id>.db`: the file of one database
/// kind (`gate`, `code`) for one worktree. Per-worktree files keep two
/// worktrees of the same repo apart, as the old `repos/...` folders did.
pub fn scoped_db_in(data: &Path, kind: &str, owner: &str, repo: &str, worktree: &str) -> PathBuf {
    data.join(kind)
        .join(owner)
        .join(repo)
        .join(format!("{worktree}.db"))
}

/// A path part that cannot climb out of its folder or merge two kinds.
fn safe_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\'])
}

/// [`scoped_db_in`] for the current repo and worktree under [`data_dir`].
/// `None` when the repo has no `origin` remote or no resolvable worktree.
pub fn scoped_db(kind: &str) -> Option<PathBuf> {
    let slug = repo_slug();
    let (owner, repo) = slug.split_once('/')?;
    let id = worktree_id();
    if id.is_empty() || ![owner, repo].iter().all(|p| safe_segment(p)) {
        return None;
    }
    Some(scoped_db_in(&data_dir(), kind, owner, repo, &id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_sits_under_the_playbook_root() {
        assert_eq!(
            data_dir_from(Path::new("/h")),
            PathBuf::from("/h/.config/playbook/data")
        );
    }

    #[test]
    fn dot_segments_are_not_safe_path_parts() {
        for bad in ["", ".", "..", "a/b", "a\\b"] {
            assert!(!safe_segment(bad), "{bad:?}");
        }
        assert!(safe_segment("pragmatic-engineer"));
    }

    #[test]
    fn scoped_files_keep_repos_and_worktrees_apart() {
        let d = Path::new("/d");
        assert_eq!(
            scoped_db_in(d, "gate", "o", "r", "wt-1"),
            PathBuf::from("/d/gate/o/r/wt-1.db")
        );
        assert_ne!(
            scoped_db_in(d, "gate", "o", "r", "a"),
            scoped_db_in(d, "gate", "o", "r", "b")
        );
        assert_ne!(
            scoped_db_in(d, "gate", "o", "r", "a"),
            scoped_db_in(d, "code", "o", "r", "a")
        );
    }
}
