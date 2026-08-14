// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Atomic file writes so concurrent hook invocations never tear state.
//! `atomic_append` replaces the helper of the same name at
//! hooks/lib/common.py:132 and hooks/lib/common.sh:73.
//!
//! Divergence from the python and shell versions: those use fcntl.flock and
//! the `flock` command respectively, which BLOCK until the lock is acquired
//! and never delete the lock file. Binding to flock(2) here would need an
//! extra crate dependency, so this port instead uses an mkdir-based
//! directory lock (mkdir is atomic on every POSIX filesystem) with a bounded
//! number of retries: up to 50 attempts, 10ms apart, after which it gives up
//! on the lock and proceeds anyway. This is a WEAKER guarantee than the
//! python original: two writers that both exhaust their retries at the same
//! moment can still interleave, where `flock`'s unbounded blocking wait
//! never allows that. The bound exists because a hook must never hang
//! forever waiting on a lock; "rare torn write under sustained contention"
//! was chosen deliberately over "hook that never returns".
//!
//! `with_dir_lock` reports whether THIS call was the one that created the
//! lock directory, and leaves removing it entirely up to the caller: a
//! caller that never acquired the lock must not remove a directory another
//! process still owns. `atomic_append` honours that and only removes the
//! lock directory it created. `incr_counter` (counter.rs) does not: it
//! removes the lock directory unconditionally, including on the
//! exhausted-retries path, because that is what python's and bash's
//! `incr_counter` both do. That quirk is preserved there deliberately; it is
//! not repeated here.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::thread;
use std::time::Duration;

/// Attempt to acquire a directory-based advisory lock at `lock_path`,
/// retrying up to `retries` times with `delay` between attempts. Runs `f`
/// regardless of whether the lock was acquired (fail-open: a hook must
/// never hang or refuse to act just because a lock is contended), and
/// returns `(acquired, result)`, where `acquired` says whether this call
/// actually created `lock_path`, so the caller can decide for itself whether
/// removing it afterward is safe.
pub(crate) fn with_dir_lock<T>(
    lock_path: &Path,
    retries: u32,
    delay: Duration,
    f: impl FnOnce() -> T,
) -> (bool, T) {
    let acquired = acquire_dir_lock(lock_path, retries, delay);
    (acquired, f())
}

/// Try to `mkdir` `lock_path`, retrying up to `retries` times with `delay`
/// between attempts. Returns whether this call created the directory.
fn acquire_dir_lock(lock_path: &Path, retries: u32, delay: Duration) -> bool {
    let mut attempts = 0;
    loop {
        if fs::create_dir(lock_path).is_ok() {
            return true;
        }
        attempts += 1;
        if attempts >= retries {
            return false;
        }
        thread::sleep(delay);
    }
}

/// Append `line` plus a trailing newline to `file`, creating parent
/// directories as needed. Serializes concurrent writers with a directory
/// lock at `<file>.lock` on a best-effort basis (see the module comment for
/// how this differs from `flock`); it never removes a lock directory it did
/// not create, so it can never destroy another writer's in-progress lock.
/// Never panics; failures are swallowed, matching the fail-soft contract
/// hooks must have.
pub fn atomic_append(file: &str, line: &str) {
    let path = Path::new(file);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }
    let lock_path = Path::new(&format!("{file}.lock")).to_path_buf();
    let (acquired, ()) = with_dir_lock(&lock_path, 50, Duration::from_millis(10), || {
        if let Ok(mut opened) = fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(opened, "{line}");
        }
    });
    if acquired {
        let _ = fs::remove_dir(&lock_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    #[test]
    fn appends_two_lines_in_order() {
        // Arrange
        let root = scratch_dir("atomic-append");
        let file = root.join("append").join("test.log");
        let file_str = file.to_str().unwrap();

        // Act
        atomic_append(file_str, "line one");
        atomic_append(file_str, "line two");

        // Assert
        let contents = fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines, vec!["line one", "line two"]);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn creates_missing_parent_directories() {
        // Arrange
        let root = scratch_dir("atomic-append-mkdir");
        let file = root.join("nested").join("deeper").join("test.log");

        // Act
        atomic_append(file.to_str().unwrap(), "line one");

        // Assert
        assert!(file.is_file());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn concurrent_appends_do_not_lose_or_interleave_a_line() {
        // Arrange
        let root = scratch_dir("atomic-append-concurrent");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("log");
        let file_str = file.to_str().unwrap().to_string();
        let thread_count = 20usize;

        // Act
        let handles: Vec<_> = (0..thread_count)
            .map(|i| {
                let f = file_str.clone();
                thread::spawn(move || atomic_append(&f, &format!("line-{i}")))
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        // Assert: every line survived, none truncated or interleaved with
        // another (a torn write would fail to parse back to a clean
        // "line-<i>").
        let contents = fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), thread_count, "a line was lost: {lines:?}");
        let mut seen: Vec<usize> = lines
            .iter()
            .map(|line| {
                line.strip_prefix("line-")
                    .and_then(|n| n.parse::<usize>().ok())
                    .unwrap_or_else(|| panic!("line was truncated or interleaved: {line:?}"))
            })
            .collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..thread_count).collect::<Vec<_>>());

        let _ = fs::remove_dir_all(&root);
    }
}
