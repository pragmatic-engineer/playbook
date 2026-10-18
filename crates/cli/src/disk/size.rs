// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! How much disk a folder takes, as `du` counts it.
//!
//! Expensive on a large tree: it reads the metadata of every file. Cargo
//! target folders hold hundreds of thousands of files, so callers run it in
//! commands, never in a hook.

use std::collections::HashSet;
use std::path::Path;
use std::time::SystemTime;

/// Bytes allocated under `path`, the path itself included. Symlinks are not
/// followed, and a file with several hard links counts once, as cargo links
/// the same build output into more than one folder. An unreadable entry
/// counts as 0.
pub fn allocated(path: &Path) -> u64 {
    scan(path).bytes
}

/// What one walk of a tree finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scan {
    /// Bytes allocated, as [`allocated`] counts them.
    pub bytes: u64,
    /// The latest change time of any entry, the path itself included.
    /// `UNIX_EPOCH` when nothing could be read.
    pub newest: SystemTime,
    /// False when the walk stopped at its deadline, so `bytes` is a floor.
    pub complete: bool,
}

/// Walks `path` once for its size and its most recent change.
pub fn scan(path: &Path) -> Scan {
    scan_until(path, None)
}

/// [`scan`] that stops at `deadline`, for reports that must stay fast.
pub fn scan_until(path: &Path, deadline: Option<std::time::Instant>) -> Scan {
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut found = Scan {
        bytes: 0,
        newest: SystemTime::UNIX_EPOCH,
        complete: true,
    };
    let mut stack = vec![path.to_path_buf()];
    while let Some(at) = stack.pop() {
        if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            found.complete = false;
            break;
        }
        let Ok(meta) = std::fs::symlink_metadata(&at) else {
            continue;
        };
        if let Ok(modified) = meta.modified() {
            found.newest = found.newest.max(modified);
        }
        if !first_link(&meta, &mut seen) {
            continue;
        }
        found.bytes = found.bytes.saturating_add(on_disk(&meta));
        if meta.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&at) {
                stack.extend(entries.filter_map(Result::ok).map(|e| e.path()));
            }
        }
    }
    found
}

#[cfg(unix)]
fn on_disk(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn on_disk(meta: &std::fs::Metadata) -> u64 {
    meta.len()
}

/// False for the second and later link of a file seen before.
#[cfg(unix)]
fn first_link(meta: &std::fs::Metadata, seen: &mut HashSet<(u64, u64)>) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.is_dir() || meta.nlink() < 2 || seen.insert((meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn first_link(_meta: &std::fs::Metadata, _seen: &mut HashSet<(u64, u64)>) -> bool {
    true
}

/// `bytes` as GB with one decimal, the way `disk status` prints sizes.
pub fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / super::probe::GIB as f64)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn counts_files_once_across_hard_links() {
        let dir = std::env::temp_dir().join(format!("pb-disk-size-{}", crate::testing::run_id()));
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::write(dir.join("a/b/data"), vec![7u8; 256 * 1024]).unwrap();
        let one = allocated(&dir);
        fs::hard_link(dir.join("a/b/data"), dir.join("a/link")).unwrap();
        std::os::unix::fs::symlink("/", dir.join("a/to-root")).unwrap();
        let linked = allocated(&dir);
        assert!(one >= 256 * 1024, "{one}");
        assert!(
            linked - one < 64 * 1024,
            "hard link or symlink counted twice: {one} then {linked}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scan_reports_the_newest_change_inside() {
        let dir = std::env::temp_dir().join(format!("pb-disk-scan-{}", crate::testing::run_id()));
        fs::create_dir_all(dir.join("deep/er")).unwrap();
        let before = SystemTime::now() - std::time::Duration::from_secs(2);
        fs::write(dir.join("deep/er/new"), b"x").unwrap();
        let found = scan(&dir);
        assert!(found.newest >= before, "{:?}", found.newest);
        assert_eq!(
            scan(Path::new("/pb-disk-scan-missing")).newest,
            SystemTime::UNIX_EPOCH
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_past_deadline_stops_the_walk() {
        let dir =
            std::env::temp_dir().join(format!("pb-disk-deadline-{}", crate::testing::run_id()));
        fs::create_dir_all(dir.join("x")).unwrap();
        let past = std::time::Instant::now();
        let found = scan_until(&dir, Some(past));
        assert!(!found.complete);
        assert_eq!(found.bytes, 0);
        assert!(scan_until(&dir, None).complete);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_path_is_zero() {
        assert_eq!(allocated(Path::new("/pb-disk-size-missing")), 0);
    }

    #[test]
    fn prints_one_decimal() {
        assert_eq!(gb(super::super::probe::GIB * 3 / 2), "1.5 GB");
    }
}
