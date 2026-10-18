// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Free space on a volume, read with one `statvfs(2)` call and no
//! subprocess.
//!
//! A call takes a few microseconds. Results are kept in this process for
//! [`CACHE_TTL`], so `disk status` and the doctor, which ask about several
//! paths on the same volume, read each volume once. A hook is a new process
//! on every tool call, so it always reads fresh: a cache file would cost
//! more than the call it saves and cannot be written on a full disk.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// One GB as `df -g` counts it, 1024 to the power of 3 bytes.
pub const GIB: u64 = 1024 * 1024 * 1024;

/// How long a reading stays fresh in this process.
pub const CACHE_TTL: Duration = Duration::from_secs(5);

/// Debug builds read a fake free space, in GB, from this variable so the
/// tests can drive the guard at any level. Release builds ignore it.
pub const TEST_FREE_GB_VAR: &str = "PLAYBOOK_TEST_DISK_FREE_GB";

/// Space on the volume that holds a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Space {
    /// Bytes an ordinary user can still write, as `df` shows under
    /// Available.
    pub free: u64,
    /// The size of the volume in bytes.
    pub total: u64,
    /// The device id, which names the volume.
    pub device: u64,
}

/// Free space on the volume that holds `path`. A path that does not exist
/// is measured at its nearest existing parent folder.
pub fn space(path: &Path) -> io::Result<Space> {
    let existing = nearest_existing(path)?;
    if let Some(hit) = cached(&existing) {
        return Ok(hit);
    }
    let mut found = read(&existing)?;
    if let Some(free) = test_free_bytes() {
        found.free = free;
    }
    remember(existing, found);
    Ok(found)
}

/// Free space as `PLAYBOOK_TEST_DISK_FREE_GB` sets it, in debug builds only.
fn test_free_bytes() -> Option<u64> {
    if !cfg!(debug_assertions) {
        return None;
    }
    parse_free_gb(&std::env::var(TEST_FREE_GB_VAR).ok()?)
}

/// A count of GB as bytes: a finite number of 0 or more, else `None`.
fn parse_free_gb(text: &str) -> Option<u64> {
    let gb: f64 = text.trim().parse().ok()?;
    (gb.is_finite() && gb >= 0.0).then_some((gb * GIB as f64) as u64)
}

/// `path` or the closest folder above it that exists.
fn nearest_existing(path: &Path) -> io::Result<PathBuf> {
    let mut at = path;
    loop {
        if at.exists() {
            return Ok(at.to_path_buf());
        }
        at = at
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no existing parent"))?;
    }
}

type Entry = (PathBuf, Instant, Space);

fn cache() -> &'static Mutex<Vec<Entry>> {
    static CACHE: OnceLock<Mutex<Vec<Entry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

fn cached(path: &Path) -> Option<Space> {
    let entries = cache().lock().ok()?;
    entries
        .iter()
        .find(|(p, at, _)| p == path && at.elapsed() < CACHE_TTL)
        .map(|(_, _, space)| *space)
}

fn remember(path: PathBuf, space: Space) {
    if let Ok(mut entries) = cache().lock() {
        entries.retain(|(p, at, _)| p != &path && at.elapsed() < CACHE_TTL);
        entries.push((path, Instant::now(), space));
    }
}

/// Drops every cached reading, so the next call reads the volume again. Used
/// after a cleanup to report what it freed.
pub fn forget() {
    if let Ok(mut entries) = cache().lock() {
        entries.clear();
    }
}

#[cfg(unix)]
fn read(path: &Path) -> io::Result<Space> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;

    let device = std::fs::metadata(path)?.dev();
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path holds a NUL byte"))?;
    let mut out = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: `c_path` is a valid NUL-terminated string that outlives the
    // call, and `out` points to writable memory the size of one `statvfs`.
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), out.as_mut_ptr()) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `statvfs` returned 0, so it filled every field of `out`, and
    // the buffer started zeroed.
    let stat = unsafe { out.assume_init() };
    #[allow(clippy::unnecessary_cast)]
    let (block, available, blocks) = (
        stat.f_frsize as u64,
        stat.f_bavail as u64,
        stat.f_blocks as u64,
    );
    Ok(Space {
        free: available.saturating_mul(block),
        total: blocks.saturating_mul(block),
        device: device as u64,
    })
}

#[cfg(not(unix))]
fn read(_path: &Path) -> io::Result<Space> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "free space is read on Unix only",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn reads_a_real_volume() {
        let dir = std::env::temp_dir();
        let space = read(&dir).expect("statvfs on the temp folder");
        assert!(space.total > 0);
        assert!(space.free <= space.total);
    }

    #[test]
    fn a_missing_path_is_measured_at_its_parent() {
        let dir = std::env::temp_dir();
        let missing = dir.join("pb-disk-probe-missing").join("deeper");
        assert_eq!(nearest_existing(&missing).unwrap(), dir);
        assert!(space(&missing).is_ok());
    }

    #[test]
    fn one_reading_takes_under_five_milliseconds() {
        let dir = std::env::temp_dir();
        let runs = 200u32;
        let start = Instant::now();
        for _ in 0..runs {
            read(&dir).unwrap();
        }
        let each = start.elapsed() / runs;
        assert!(each < Duration::from_millis(5), "one statvfs took {each:?}");
    }

    #[test]
    fn a_reading_is_reused_within_the_cache_window() {
        let path = PathBuf::from("/pb-disk-probe-cache-key");
        let fake = Space {
            free: 7,
            total: 9,
            device: 1,
        };
        remember(path.clone(), fake);
        assert_eq!(cached(&path), Some(fake));
        forget();
        assert_eq!(cached(&path), None);
    }

    #[test]
    fn the_test_override_parses_gb() {
        let cases = [
            ("6", Some(6 * GIB)),
            (" 0.5 ", Some(GIB / 2)),
            ("0", Some(0)),
            ("-1", None),
            ("NaN", None),
            ("inf", None),
            ("abc", None),
            ("", None),
        ];
        for (text, want) in cases {
            assert_eq!(parse_free_gb(text), want, "{text:?}");
        }
    }
}
