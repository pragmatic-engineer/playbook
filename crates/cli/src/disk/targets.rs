// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Where worktrees and per-agent cargo target folders live (ADR 0047
//! decision 7). An agent building in worktree `<name>` uses
//! `<worktrees root>/cargo-<name>-<hash>` as its `CARGO_TARGET_DIR`.

use std::path::{Path, PathBuf};

/// The name of the target folder cargo makes inside a worktree.
pub const IN_TREE_TARGET: &str = "target";

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

/// The target folder that belongs to the worktree at `worktree`:
/// `cargo-<folder name>-<hash>`, where the hash comes from the full path, so
/// two worktrees with the same folder name in different repos or plans
/// never share one. The parent is resolved, since the worktree itself may
/// already be gone when its folder is removed.
pub fn target_for(root: &Path, worktree: &Path) -> Option<PathBuf> {
    use sha2::{Digest, Sha256};
    let name = worktree.file_name()?;
    let full = worktree
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .map_or_else(|| worktree.to_path_buf(), |p| p.join(name));
    let digest = Sha256::digest(full.as_os_str().as_encoded_bytes());
    let hash: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    Some(root.join(format!("{TARGET_PREFIX}{}-{hash}", name.to_string_lossy())))
}

/// The warning for more than `max` cargo target folders, or `None`.
pub fn too_many_warning(count: usize, max: u64) -> Option<String> {
    (count as u64 > max).then(|| {
        format!(
            "{count} cargo target folders exist, more than disk.maxTargetDirs ({max}). `playbook disk clean` removes idle ones."
        )
    })
}

/// The warning for cargo target folders that hold more than `max_gb` GB
/// together, or `None`.
pub fn too_big_warning(bytes: u64, max_gb: u64) -> Option<String> {
    (bytes > max_gb.saturating_mul(super::probe::GIB)).then(|| {
        format!(
            "cargo target folders hold {}, more than disk.maxTargetGb ({max_gb}). `playbook disk clean` removes idle ones.",
            super::size::gb(bytes)
        )
    })
}

/// Build settings that keep an agent's target folder small. Measured on this
/// workspace with `cargo test --workspace --no-run`: 3.0 GB with the cargo
/// defaults, 1.1 GB with these. Incremental data and debug info are the two
/// big parts. A panic keeps its file and line.
pub const LEAN_BUILD_ENV: &[(&str, &str)] = &[
    ("CARGO_INCREMENTAL", "0"),
    ("CARGO_PROFILE_DEV_DEBUG", "0"),
    ("CARGO_PROFILE_TEST_DEBUG", "0"),
];

/// The `export` line an agent evaluates before a cargo command: the private
/// target folder, then [`LEAN_BUILD_ENV`].
/// `None` when the path is not valid UTF-8, since a lossy copy would name
/// another folder.
pub fn export_line(target: &Path) -> Option<String> {
    let mut line = format!(
        "export CARGO_TARGET_DIR={}",
        crate::common::shell::quote(target.to_str()?)
    );
    for (name, value) in LEAN_BUILD_ENV {
        line.push_str(&format!(" {name}={value}"));
    }
    Some(line)
}

/// How deep below a scan root [`find_cargo_targets`] looks.
pub const SCAN_DEPTH: usize = 5;
/// The most folders one scan reads, so a huge home folder stays fast.
pub const SCAN_MAX_DIRS: usize = 20_000;

/// Folders a scan never enters: big trees that hold no project builds. The
/// macOS media folders are left out too, so the scan does not ask for
/// access to them. Desktop, Documents and Downloads are scanned, since code
/// often lives there.
const SKIP: &[&str] = &[
    "Library",
    "node_modules",
    ".git",
    "Applications",
    ".Trash",
    "Pictures",
    "Movies",
    "Music",
];

/// Cargo target folders under `roots`, found by the `CACHEDIR.TAG` and
/// `.rustc_info.json` that cargo writes at the top of each one, and whether
/// the search finished. Read only and bounded: at most [`SCAN_DEPTH`] levels
/// and [`SCAN_MAX_DIRS`] folders, skipping hidden folders other than
/// `.worktrees`, symlinks, and the trees in `SKIP`, and stops at `deadline`.
/// A found folder is not entered. A root equal to or inside another root is
/// scanned once. Builds from any project on the volume show up, which
/// matters because free space is shared.
pub fn find_cargo_targets(
    roots: &[PathBuf],
    deadline: Option<std::time::Instant>,
) -> (Vec<PathBuf>, bool) {
    let mut unique = roots.to_vec();
    unique.sort();
    unique.dedup();
    let late = || deadline.is_some_and(|d| std::time::Instant::now() >= d);
    let mut found = Vec::new();
    let mut visited = 0usize;
    let mut complete = true;
    let mut queue: std::collections::VecDeque<(PathBuf, usize)> = unique
        .iter()
        .filter(|r| {
            !unique
                .iter()
                .any(|other| other != *r && r.starts_with(other))
        })
        .map(|r| (r.clone(), 0))
        .collect();
    while let Some((dir, depth)) = queue.pop_front() {
        if visited >= SCAN_MAX_DIRS || late() {
            complete = false;
            break;
        }
        visited += 1;
        if dir.join("CACHEDIR.TAG").is_file() && dir.join(".rustc_info.json").is_file() {
            found.push(dir);
            continue;
        }
        if depth >= SCAN_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            if visited + queue.len() >= SCAN_MAX_DIRS || late() {
                complete = false;
                break;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let hidden = name.starts_with('.') && name != ".worktrees";
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            if is_dir && !hidden && !SKIP.contains(&name.as_str()) {
                queue.push_back((entry.path(), depth + 1));
            }
        }
    }
    found.sort();
    (found, complete)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn worktrees_with_one_folder_name_get_different_target_folders() {
        let root =
            std::env::temp_dir().join(format!("pb-disk-target-for-{}", crate::testing::run_id()));
        fs::create_dir_all(root.join("app/fix-x")).unwrap();
        fs::create_dir_all(root.join("lib")).unwrap();
        let app = target_for(&root, &root.join("app/fix-x")).unwrap();
        let lib = target_for(&root, &root.join("lib/fix-x")).unwrap();
        assert_ne!(app, lib);
        assert_eq!(app.parent(), Some(root.as_path()));
        let name = app.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with("cargo-fix-x-") && name.len() == "cargo-fix-x-".len() + 8,
            "{name}"
        );
        fs::remove_dir_all(root.join("app/fix-x")).unwrap();
        assert_eq!(
            target_for(&root, &root.join("app/fix-x")).unwrap(),
            app,
            "same after removal"
        );
        assert_eq!(too_many_warning(4, 4), None);
        assert!(too_many_warning(5, 4).is_some_and(|w| w.contains("playbook disk clean")));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_size_cap_warns_only_above_it() {
        let gb = super::super::probe::GIB;
        assert_eq!(too_big_warning(20 * gb, 20), None);
        assert!(too_big_warning(20 * gb + 1, 20)
            .is_some_and(|w| w.contains("disk.maxTargetGb (20)") && w.contains("disk clean")));
    }

    #[test]
    fn the_export_line_quotes_the_folder_and_sets_the_lean_build() {
        assert_eq!(
            export_line(Path::new("/w/it's here/cargo-x-1")).as_deref(),
            Some(
                "export CARGO_TARGET_DIR='/w/it'\\''s here/cargo-x-1' CARGO_INCREMENTAL=0 \
             CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0"
            )
        );
    }

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
    fn the_scan_finds_targets_and_stays_out_of_hidden_and_skipped_folders() {
        let root = std::env::temp_dir().join(format!("pb-disk-scan-{}", crate::testing::run_id()));
        let target = |rel: &str| {
            let dir = root.join(rel);
            fs::create_dir_all(dir.join("debug")).unwrap();
            fs::write(
                dir.join("CACHEDIR.TAG"),
                "Signature: 8a477f597d28d172789f06886806bc55",
            )
            .unwrap();
            fs::write(dir.join(".rustc_info.json"), "{}").unwrap();
            dir
        };
        let a = target("code/app/target");
        let b = target("code/.worktrees/cargo-x");
        let deep = target("d1/d2/d3/d4/target");
        target("code/.hidden/target");
        target("Library/Caches/x/target");
        target("Pictures/x/target");
        target("code/app/target/debug/nested/target");
        target("e1/e2/e3/e4/e5/target");
        target("code/n1/n2/n3/n4/target");
        fs::create_dir_all(root.join("code/plain/target")).unwrap();
        std::os::unix::fs::symlink(root.join("code"), root.join("loop")).unwrap();
        let all = vec![b.clone(), a.clone(), deep.clone()];
        assert_eq!(
            find_cargo_targets(std::slice::from_ref(&root), None),
            (all.clone(), true)
        );
        let nested = vec![root.join("code"), root.clone(), root.clone()];
        assert_eq!(find_cargo_targets(&nested, None), (all, true));
        let past = Some(std::time::Instant::now());
        assert_eq!(
            find_cargo_targets(std::slice::from_ref(&root), past),
            (vec![], false)
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_worktree_maps_to_a_stable_cargo_folder() {
        let root = Path::new("/w/.worktrees");
        let wt = Path::new("/w/.worktrees/playbook/fix-x");
        assert_eq!(
            target_for(root, wt),
            Some(root.join("cargo-fix-x-85604e51"))
        );
        assert_eq!(target_for(root, Path::new("/")), None);
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
