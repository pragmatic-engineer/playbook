// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Atomic file writes so concurrent hook invocations never tear state.
//! `atomic_append` replaces the helper of the same name at
//! the retired shell originals.
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
//! Known gap: the stale check and its `remove_dir` are two steps, so two
//! callers that both see one dead lock as stale can both end up holding a
//! lock. Treat these locks as fail-open guards against torn writes, never as
//! strict mutual exclusion.
//!
//! `with_dir_lock` reports whether THIS call was the one that created the
//! lock directory, and leaves removing it entirely up to the caller: a
//! caller that never acquired the lock must not remove a directory another
//! process still owns. `atomic_append` honours that and only removes the
//! lock directory it created, and so does `incr_counter` (counter.rs).
//! `acquire_dir_lock` clears a lock directory older than `STALE_LOCK_AGE`
//! before it starts waiting, so a killed holder cannot slow every later call.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

/// Attempt to acquire a directory-based advisory lock at `lock_path`,
/// retrying up to `retries` times with `delay` between attempts. Runs `f`
/// regardless of whether the lock was acquired (fail-open: a hook must
/// never hang or refuse to act just because a lock is contended), and
/// returns `(acquired, result)`, where `acquired` says whether this call
/// actually created `lock_path`, so the caller can decide for itself whether
/// removing it afterward is safe.
///
/// A lock directory left by a killed holder is removed first when it is older
/// than [`STALE_LOCK_AGE`], so no caller waits out its retry budget and then
/// runs unguarded forever. Use [`with_dir_lock_aged`] when the critical
/// section can legitimately run longer than that.
pub(crate) fn with_dir_lock<T>(
    lock_path: &Path,
    retries: u32,
    delay: Duration,
    f: impl FnOnce() -> T,
) -> (bool, T) {
    with_dir_lock_aged(lock_path, retries, delay, STALE_LOCK_AGE, f)
}

/// [`with_dir_lock`] with a caller-chosen `stale_age`, for a critical section
/// (copying a store) that outlives [`STALE_LOCK_AGE`].
pub(crate) fn with_dir_lock_aged<T>(
    lock_path: &Path,
    retries: u32,
    delay: Duration,
    stale_age: Duration,
    f: impl FnOnce() -> T,
) -> (bool, T) {
    remove_stale_lock_dir(lock_path, stale_age);
    let acquired = acquire_dir_lock(lock_path, retries, delay);
    (acquired, f())
}

/// Stale age for a lock held across a long copy (a store or a checkout). Every
/// taker of such a lock must use the same age, or the shortest one steals a
/// live holder's lock.
pub(crate) const LONG_LOCK_STALE_AGE: Duration = Duration::from_secs(300);

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
    if !age.is_some_and(|age| age > max_age) {
        return;
    }
    // Rename first, so of several callers that saw the same stale lock only
    // one claims it, and a lock another caller has just taken is put back.
    let claimed = lock_path.with_extension(format!(
        "stale-{}-{}",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    if fs::rename(lock_path, &claimed).is_err() {
        return;
    }
    let still_old = fs::metadata(&claimed)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age > max_age);
    if still_old {
        let _ = fs::remove_dir(&claimed);
    } else {
        let _ = fs::rename(&claimed, lock_path);
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
        if attempts == 0 {
            // A holder killed mid-section leaves its directory behind; clear
            // it once and try again instead of waiting out the whole budget.
            remove_stale_lock_dir(lock_path, STALE_LOCK_AGE);
            if fs::create_dir(lock_path).is_ok() {
                return true;
            }
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

/// Distinguishes temp files of two writers in one process (threads, or two
/// calls racing on one path), which a process id alone does not.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Replaces the contents of `path` with `content` through a temp file in the
/// same directory and a rename, so a reader never sees a partial file. An
/// existing file keeps its permissions, and a failed write leaves it as it
/// was. The caller creates the directory.
pub fn write_atomic(path: &Path, content: impl AsRef<[u8]>) -> std::io::Result<()> {
    write_through_tmp(path, content.as_ref(), false)
}

/// Like [`write_atomic`], for a file that holds account or session data: a
/// new file is owner-only on unix, and the content is flushed to disk before
/// the rename.
pub fn write_atomic_private(path: &Path, content: impl AsRef<[u8]>) -> std::io::Result<()> {
    write_through_tmp(path, content.as_ref(), true)
}

fn write_through_tmp(path: &Path, content: &[u8], private: bool) -> std::io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.{}.{seq}.tmp", std::process::id()));
    let result = stage(&tmp, content, private)
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

fn stage(tmp: &Path, content: &[u8], private: bool) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(tmp)?;
    file.write_all(content)?;
    if private {
        file.sync_all()?;
    }
    Ok(())
}

/// The data names the account, repos, and branches, so the file is owner-only
/// on unix. Created first so SQLite never makes it with the default umask, and
/// the WAL and shm files copy this mode. An older, wider file is tightened.
pub fn make_db_file_private(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let error = |e: std::io::Error| format!("failed to secure {}: {e}", path.display());
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .map_err(error)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(error)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    #[cfg(unix)]
    #[test]
    fn a_stale_lock_is_healed_by_with_dir_lock_itself() {
        let root = scratch_dir("atomic-stale-heal");
        let lock = root.join("x.lock");
        fs::create_dir_all(&lock).unwrap();
        age_dir(&lock, STALE_LOCK_AGE * 3);
        let started = std::time::Instant::now();

        let (acquired, ()) = with_dir_lock(&lock, 50, Duration::from_millis(100), || {});

        assert!(acquired, "a dead holder's lock must not block forever");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn a_fresh_lock_is_respected() {
        let root = scratch_dir("atomic-fresh-keep");
        let lock = root.join("x.lock");
        fs::create_dir_all(&lock).unwrap();

        let (acquired, ()) = with_dir_lock(&lock, 3, Duration::from_millis(5), || {});

        assert!(!acquired);
        assert!(lock.exists(), "a live holder's lock is never removed");
    }

    #[cfg(unix)]
    #[test]
    fn a_lock_older_than_default_but_younger_than_the_custom_age_is_kept() {
        let root = scratch_dir("atomic-custom-age");
        let lock = root.join("x.lock");
        fs::create_dir_all(&lock).unwrap();
        age_dir(&lock, STALE_LOCK_AGE * 3);

        let (acquired, ()) = with_dir_lock_aged(
            &lock,
            3,
            Duration::from_millis(5),
            STALE_LOCK_AGE * 10,
            || {},
        );

        assert!(!acquired);
        assert!(lock.exists());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_append_recovers_from_a_stale_lock() {
        let root = scratch_dir("atomic-append-stale");
        let file = root.join("a.log");
        let lock = root.join("a.log.lock");
        fs::create_dir_all(&lock).unwrap();
        age_dir(&lock, STALE_LOCK_AGE * 3);

        atomic_append(file.to_str().unwrap(), "hello");

        assert_eq!(fs::read_to_string(&file).unwrap(), "hello\n");
        assert!(!lock.exists(), "the healed lock is released afterward");
    }

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
    fn append_clears_a_stale_lock_and_takes_it_without_waiting_out_the_retries() {
        // Arrange
        let root = scratch_dir("atomic-append-stale");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("log.jsonl");
        let lock = root.join("log.jsonl.lock");
        fs::create_dir_all(&lock).unwrap();
        age_dir(&lock, STALE_LOCK_AGE * 3);

        // Act
        atomic_append(file.to_str().unwrap(), "one");

        // Assert
        assert_eq!(fs::read_to_string(&file).unwrap(), "one\n");
        assert!(!lock.exists(), "the lock this call took must be released");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn append_leaves_a_fresh_foreign_lock_alone() {
        // Arrange
        let root = scratch_dir("atomic-append-fresh");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("log.jsonl");
        let lock = root.join("log.jsonl.lock");
        fs::create_dir_all(&lock).unwrap();

        // Act
        atomic_append(file.to_str().unwrap(), "one");

        // Assert
        assert!(lock.exists(), "a live holder's lock must not be removed");
        assert_eq!(fs::read_to_string(&file).unwrap(), "one\n");
        let _ = fs::remove_dir_all(&root);
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

    #[test]
    fn concurrent_writers_to_one_path_never_tear_the_file() {
        let root = scratch_dir("atomic-race");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("shared.json");
        let handles: Vec<_> = (0..8)
            .map(|n| {
                let file = file.clone();
                thread::spawn(move || {
                    let body = format!("{n}").repeat(4096);
                    for _ in 0..50 {
                        write_atomic(&file, &body).unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let text = fs::read_to_string(&file).unwrap();
        assert_eq!(text.len(), 4096);
        assert!(text.chars().all(|c| c == text.chars().next().unwrap()));
        let leftovers = fs::read_dir(&root).unwrap().count();
        assert_eq!(leftovers, 1, "no temp file may remain");
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_private_creates_an_owner_only_file() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch_dir("atomic-private");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("secret");
        write_atomic_private(&file, b"data").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"data");
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn make_db_file_private_creates_and_tightens_to_owner_only() {
        // Arrange
        use std::os::unix::fs::PermissionsExt;
        let root = scratch_dir("atomic-db-private");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("x.db");
        fs::write(&path, b"").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

        // Act
        make_db_file_private(&path).unwrap();
        let fresh = root.join("fresh.db");
        make_db_file_private(&fresh).unwrap();

        // Assert
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&fresh), 0o600);
    }
}
