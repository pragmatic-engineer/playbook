// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Atomic counter increment. Ports `incr_counter` (hooks/lib/common.py:192)
//! and `_incr_counter` (hooks/lib/common.sh:125), including their mkdir-lock
//! plus temp-file-swap semantics and the up-to-50-tries retry behaviour.

use crate::common::atomic::with_dir_lock;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

/// Atomically increment the integer stored in `file`, returning the new
/// value. A missing or unparsable file starts from 0. Never panics; any
/// failure along the way leaves the counter unresolved and best-effort, the
/// same fail-soft contract common.py's `incr_counter` has.
pub fn incr_counter(file: &str) -> i64 {
    let path = Path::new(file);
    let lock_path = Path::new(&format!("{file}.lock")).to_path_buf();
    let (_, next) = with_dir_lock(&lock_path, 50, Duration::from_millis(10), || {
        let current = fs::read_to_string(path)
            .ok()
            .and_then(|contents| contents.trim().parse::<i64>().ok())
            .unwrap_or(0);
        // Python's int is arbitrary precision; i64 is not. Saturate rather
        // than silently wrapping to a negative counter when a corrupted or
        // hand-edited counter file holds a value at or near i64::MAX.
        let next = current.saturating_add(1);
        write_atomically(path, &next.to_string());
        next
    });
    // Deliberately unconditional, even though the lock may not have been
    // acquired: matches hooks/lib/common.py's and hooks/lib/common.sh's
    // incr_counter, which both remove the lock directory even after
    // exhausting every retry. See atomic.rs's module comment for the full
    // rationale; do not change this to a conditional removal.
    let _ = fs::remove_dir(&lock_path);
    next
}

/// Write `contents` to `path` via a temp file in the same directory plus a
/// rename, so a reader never observes a partially written file. Never
/// panics; a failed write leaves the previous file content in place.
fn write_atomically(path: &Path, contents: &str) {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let tmp_path = parent.join(format!(
        ".tmp-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let Ok(mut tmp_file) = fs::File::create(&tmp_path) else {
        return;
    };
    if tmp_file.write_all(contents.as_bytes()).is_ok() {
        let _ = fs::rename(&tmp_path, path);
    } else {
        let _ = fs::remove_file(&tmp_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    #[test]
    fn missing_file_starts_at_one() {
        // Arrange
        let root = scratch_dir("counter-missing-file");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("cnt");

        // Act
        let got = incr_counter(file.to_str().unwrap());

        // Assert
        assert_eq!(got, 1);
        assert_eq!(fs::read_to_string(&file).unwrap(), "1");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn second_call_returns_two() {
        // Arrange
        let root = scratch_dir("counter-second-call");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("cnt");
        incr_counter(file.to_str().unwrap());

        // Act
        let got = incr_counter(file.to_str().unwrap());

        // Assert
        assert_eq!(got, 2);
        assert_eq!(fs::read_to_string(&file).unwrap(), "2");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn lock_directory_removed_after_call() {
        // Arrange
        let root = scratch_dir("counter-lock-dir");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("cnt");

        // Act
        incr_counter(file.to_str().unwrap());

        // Assert
        let lock_dir = root.join("cnt.lock");
        assert!(!lock_dir.exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pre_seeded_file_increments_from_existing_value() {
        // Arrange
        let root = scratch_dir("counter-pre-seeded");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("cnt");
        fs::write(&file, "41").unwrap();

        // Act
        let got = incr_counter(file.to_str().unwrap());

        // Assert
        assert_eq!(got, 42);
        assert_eq!(fs::read_to_string(&file).unwrap(), "42");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn counter_at_i64_max_saturates_instead_of_wrapping() {
        // Arrange
        let root = scratch_dir("counter-overflow");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("cnt");
        fs::write(&file, i64::MAX.to_string()).unwrap();

        // Act
        let got = incr_counter(file.to_str().unwrap());

        // Assert: a wrapping add would go negative here.
        assert_eq!(got, i64::MAX);
        assert_eq!(fs::read_to_string(&file).unwrap(), i64::MAX.to_string());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn concurrent_increments_do_not_lose_a_count() {
        // Arrange
        let root = scratch_dir("counter-concurrent");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("cnt");
        let file_str = file.to_str().unwrap().to_string();
        let thread_count = 20i64;

        // Act
        let handles: Vec<_> = (0..thread_count)
            .map(|_| {
                let f = file_str.clone();
                std::thread::spawn(move || incr_counter(&f))
            })
            .collect();
        let mut results: Vec<i64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        results.sort_unstable();

        // Assert
        let expected: Vec<i64> = (1..=thread_count).collect();
        assert_eq!(results, expected);
        let final_value: i64 = fs::read_to_string(&file).unwrap().trim().parse().unwrap();
        assert_eq!(final_value, thread_count);

        let _ = fs::remove_dir_all(&root);
    }
}
