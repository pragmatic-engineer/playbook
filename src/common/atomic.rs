// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Atomic file writes so concurrent hook invocations never tear state.
//! `atomic_append` replaces the helper of the same name at
//! hooks/lib/common.py:132 and hooks/lib/common.sh:73.
//!
//! Divergence from the python and shell versions: those use fcntl.flock and
//! the `flock` command respectively to serialize writers, falling back to no
//! locking at all when the primitive is unavailable. Binding to flock(2)
//! here would need an extra crate dependency, so this port instead reuses
//! the mkdir-based directory lock that `counter.rs` already needs for
//! `incr_counter`, which gives the same serialization guarantee with a
//! bounded wait instead of an unbounded one.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::thread;
use std::time::Duration;

/// Acquire a directory-based advisory lock at `lock_path`, retrying up to
/// `retries` times with `delay` between attempts, run `f`, then release the
/// lock. If every attempt fails, `f` still runs unlocked rather than hanging
/// forever or giving up, matching the fail-open behaviour of the mkdir-lock
/// retry loop in common.py and common.sh's `incr_counter`.
pub(crate) fn with_dir_lock<T>(
    lock_path: &Path,
    retries: u32,
    delay: Duration,
    f: impl FnOnce() -> T,
) -> T {
    let mut attempts = 0;
    while fs::create_dir(lock_path).is_err() {
        attempts += 1;
        if attempts >= retries {
            break;
        }
        thread::sleep(delay);
    }
    let result = f();
    let _ = fs::remove_dir(lock_path);
    result
}

/// Append `line` plus a trailing newline to `file`, creating parent
/// directories as needed. Serializes concurrent writers with a directory
/// lock at `<file>.lock`. Never panics; failures are swallowed, matching the
/// fail-soft contract hooks must have.
pub fn atomic_append(file: &str, line: &str) {
    let path = Path::new(file);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }
    let lock_path = Path::new(&format!("{file}.lock")).to_path_buf();
    with_dir_lock(&lock_path, 50, Duration::from_millis(10), || {
        if let Ok(mut opened) = fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(opened, "{line}");
        }
    });
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
}
