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

/// A lock directory older than this belongs to a holder that died inside its
/// critical section, which takes well under a second.
pub(crate) const STALE_LOCK_AGE: Duration = Duration::from_secs(10);

/// Removes the lock directory at `lock_path` when it is older than `max_age`.
/// A killed holder leaves its directory behind, and every later caller would
/// otherwise wait out the retry budget and then run unguarded forever.
pub(crate) fn remove_stale_lock_dir(lock_path: &Path, max_age: Duration) {
    let age = fs::metadata(lock_path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok());
    if age.is_some_and(|age| age > max_age) {
        let _ = fs::remove_dir(lock_path);
    }
}

/// Creates `dir` (and any missing parents) and makes the leaf owner-only on
/// unix, for state that holds account or repo names.
pub(crate) fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Try to `mkdir` `lock_path`, retrying up to `retries` times with `delay`
/// between attempts. Returns whether this call created the directory.
pub(crate) fn acquire_dir_lock(lock_path: &Path, retries: u32, delay: Duration) -> bool {
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

/// Replaces the contents of `path` with `content` through a temp file in the
/// same directory and a rename, so a reader never sees a partial file. The
/// file keeps its permissions, and a failed write leaves it as it was.
pub fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = fs::write(&tmp, content)
        .and_then(|()| match fs::metadata(path) {
            Ok(meta) => fs::set_permissions(&tmp, meta.permissions()),
            Err(_) => Ok(()),
        })
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
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

    #[cfg(unix)]
    fn age_dir(dir: &Path, age: Duration) {
        let past = std::time::SystemTime::now() - age;
        fs::File::open(dir)
            .and_then(|handle| handle.set_modified(past))
            .expect("a directory's mtime is settable");
    }

    #[cfg(unix)]
    #[test]
    fn an_old_lock_directory_is_removed_and_a_fresh_one_is_kept() {
        // Arrange
        let root = scratch_dir("atomic-stale-lock");
        let old = root.join("old.lock");
        let fresh = root.join("fresh.lock");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&fresh).unwrap();
        age_dir(&old, STALE_LOCK_AGE * 3);

        // Act
        remove_stale_lock_dir(&old, STALE_LOCK_AGE);
        remove_stale_lock_dir(&fresh, STALE_LOCK_AGE);

        // Assert
        assert!(!old.exists(), "a dead holder's directory must be cleared");
        assert!(fresh.exists(), "a live holder's directory must stay");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_private_directory_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        // Arrange
        let root = scratch_dir("atomic-private-dir");
        let dir = root.join("usage");

        // Act
        ensure_private_dir(&dir).unwrap();

        // Assert
        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn write_atomic_replaces_the_content_and_keeps_the_mode() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch_dir("atomic-write");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("msg");
        fs::write(&file, "old").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();

        write_atomic(&file, "new").unwrap();

        assert_eq!(fs::read_to_string(&file).unwrap(), "new");
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let leftovers: Vec<_> = fs::read_dir(&root).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "the temp file is gone");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn write_atomic_into_a_missing_directory_fails_without_a_stray_file() {
        let root = scratch_dir("atomic-write-missing");

        let result = write_atomic(&root.join("nope").join("msg"), "x");

        assert!(result.is_err());
        assert!(!root.exists());
    }
}
