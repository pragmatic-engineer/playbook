// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Where worktrees and per-agent cargo target folders live (ADR 0047
//! decision 7). An agent building in worktree `<name>` uses
//! `<worktrees root>/cargo-<name>` as its `CARGO_TARGET_DIR`.

use std::path::{Path, PathBuf};

/// The name prefix of a per-agent cargo target folder.
pub const TARGET_PREFIX: &str = "cargo-";

/// The folder that holds this repo's worktree folder and the `cargo-*`
/// target folders: `<main worktree parent>/.worktrees`, or
/// `WORKTREE_BASE_DIR` as the worktree hook reads it. `repo_root` may be any
/// worktree of the repo. The main worktree is the first entry of `git
/// worktree list`, which also holds with a separate git folder.
pub fn worktrees_root(repo_root: &Path) -> Option<PathBuf> {
    let main = crate::worktree::list_worktrees(repo_root)
        .ok()?
        .into_iter()
        .next()?;
    root_for_main(&main.path)
}

/// [`worktrees_root`] for a known main worktree.
pub fn root_for_main(main_root: &Path) -> Option<PathBuf> {
    crate::worktree::launcher_base(main_root)?
        .parent()
        .map(Path::to_path_buf)
}

/// The `cargo-*` folders directly under `root`, sorted. Symlinks are left
/// out, so a link never leads a cleanup outside the root.
pub fn target_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with(TARGET_PREFIX))
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs
}

/// The target folder that belongs to the worktree at `worktree`.
pub fn target_for(root: &Path, worktree: &Path) -> Option<PathBuf> {
    let name = worktree.file_name()?.to_string_lossy();
    Some(root.join(format!("{TARGET_PREFIX}{name}")))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lists_only_cargo_folders_and_skips_links() {
        let root =
            std::env::temp_dir().join(format!("pb-disk-targets-{}", crate::testing::run_id()));
        fs::create_dir_all(root.join("cargo-a")).unwrap();
        fs::create_dir_all(root.join("cargo-b")).unwrap();
        fs::create_dir_all(root.join("playbook")).unwrap();
        fs::write(root.join("cargo-file"), "x").unwrap();
        std::os::unix::fs::symlink("/", root.join("cargo-link")).unwrap();
        assert_eq!(
            target_dirs(&root),
            vec![root.join("cargo-a"), root.join("cargo-b")]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_worktree_maps_to_its_cargo_folder() {
        let root = Path::new("/w/.worktrees");
        assert_eq!(
            target_for(root, Path::new("/w/.worktrees/playbook/fix-x")),
            Some(root.join("cargo-fix-x"))
        );
    }

    #[test]
    fn the_root_sits_next_to_the_main_worktree() {
        if std::env::var_os("WORKTREE_BASE_DIR").is_some() {
            return;
        }
        assert_eq!(
            root_for_main(Path::new("/code/acme/widgets")),
            Some(PathBuf::from("/code/acme/.worktrees"))
        );
    }

    #[test]
    fn a_linked_worktree_finds_the_same_root() {
        if std::env::var_os("WORKTREE_BASE_DIR").is_some() {
            return;
        }
        let base = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("pb-disk-root-{}", crate::testing::run_id()));
        let repo = base.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let git = |dir: &Path, args: &[&str]| {
            assert!(std::process::Command::new("git")
                .current_dir(dir)
                .args(args)
                .output()
                .unwrap()
                .status
                .success());
        };
        git(&repo, &["init", "-q"]);
        git(
            &repo,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "x",
            ],
        );
        let linked = base.join(".worktrees/repo/feature");
        git(&repo, &["worktree", "add", "-q", linked.to_str().unwrap()]);
        assert_eq!(worktrees_root(&linked), Some(base.join(".worktrees")));
        assert_eq!(worktrees_root(&repo), Some(base.join(".worktrees")));
        fs::remove_dir_all(&base).unwrap();
    }
}
