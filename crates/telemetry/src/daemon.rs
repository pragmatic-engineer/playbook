// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The collector's lifetime lock.
//!
//! The collector takes an exclusive lock on `collector.lock` before it binds
//! and keeps it until it exits. The OS drops the lock on any exit, so a held
//! lock always means a live collector and there is no stale state to clean up.
//! The holder writes `<pid> <version>\n` into the file through the locked
//! handle, because `collector.json` is written late in startup and keeps a
//! dead pid after a crash.

use super::collector::{pid_alive, state_path};
use super::run::Paths;
use crate::util::atomic::ensure_private_dir;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const LOCK_FILE: &str = "collector.lock";
/// Owner only, like the rest of the data folder.
#[cfg(unix)]
const LOCK_MODE: u32 = 0o600;
/// A starter may hold a shared probe lock for a moment, so a busy lock is
/// retried a few times before it counts as a live collector.
const QUICK_RETRIES: u32 = 3;
const RETRY_PAUSE: Duration = Duration::from_millis(50);
/// How long to keep retrying while the holder line names no live process:
/// a descheduled starter's probe must not make a lone new collector give up.
const INCOMPLETE_HOLDER_WAIT: Duration = Duration::from_secs(1);

/// The result of asking for the lifetime lock.
#[derive(Debug)]
pub enum Lock {
    /// This process holds the lock until the file is dropped.
    Held(File),
    /// Another collector holds it.
    Busy,
    /// The lock file cannot be opened or locked.
    Unavailable(std::io::Error),
}

/// `collector.lock`, next to `collector.json`.
pub fn lock_path(paths: &Paths) -> PathBuf {
    state_path(paths).with_file_name(LOCK_FILE)
}

/// What a collector writes into the lock file.
fn holder_line() -> String {
    format!("{} {}\n", std::process::id(), env!("CARGO_PKG_VERSION"))
}

/// The pid in a whole `<pid> <version>\n` line. Without the newline the
/// holder is still starting, or the file is only held by a shared probe.
fn holder_pid(text: &str) -> Option<u32> {
    let (pid, version) = text.strip_suffix('\n')?.split_once(' ')?;
    pid.parse().ok().filter(|_| !version.is_empty())
}

/// Whether the lock file names a running collector. A dead collector leaves
/// a whole line behind, so a dead pid counts like a partial line.
fn holder_is_live(text: &str) -> bool {
    holder_pid(text).is_some_and(pid_alive)
}

fn holder_text(mut file: &File) -> String {
    let mut text = String::new();
    if file.rewind().is_ok() {
        let _ = file.read_to_string(&mut text);
    }
    text
}

/// Takes the lifetime lock and records this process as the holder.
pub fn acquire(paths: &Paths) -> Lock {
    let path = lock_path(paths);
    // Only a missing folder is created: an existing one keeps its mode, so a
    // read-only folder fails at the open below instead of being repaired.
    if let Some(parent) = path.parent().filter(|p| !p.exists()) {
        if let Err(e) = ensure_private_dir(parent) {
            return Lock::Unavailable(e);
        }
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(LOCK_MODE);
    }
    let mut file = match options.open(&path) {
        Ok(f) => f,
        Err(e) => return Lock::Unavailable(e),
    };
    let started = Instant::now();
    let mut tries = 0;
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::Error(e)) => return Lock::Unavailable(e),
            Err(TryLockError::WouldBlock) => {
                tries += 1;
                let settled = tries > QUICK_RETRIES
                    && (holder_is_live(&holder_text(&file))
                        || started.elapsed() >= INCOMPLETE_HOLDER_WAIT);
                if settled {
                    return Lock::Busy;
                }
                std::thread::sleep(RETRY_PAUSE);
            }
        }
    }
    // One write, so a reader sees either no line or the whole line. The
    // rewind matters: set_len does not move the cursor a read left behind.
    let wrote = file
        .set_len(0)
        .and_then(|()| file.seek(SeekFrom::Start(0)))
        .and_then(|_| file.write_all(holder_line().as_bytes()));
    match wrote {
        Ok(()) => Lock::Held(file),
        Err(e) => Lock::Unavailable(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::scratch_dir;

    fn paths(name: &str) -> Paths {
        let dir = scratch_dir(name);
        let mut p = Paths::from_home(&dir);
        p.db = dir.join("usage").join("usage.db");
        p
    }

    fn shared_probe(p: &Paths, hold: Duration) -> std::thread::JoinHandle<()> {
        let path = lock_path(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(&path)
            .unwrap();
        file.try_lock_shared().unwrap();
        std::thread::spawn(move || {
            std::thread::sleep(hold);
            drop(file);
        })
    }

    /// How long the probe holds in the seeded file tests: past the quick
    /// retries, well inside the wait.
    const SEEDED_PROBE_HOLD: Duration = Duration::from_millis(300);

    fn seed(p: &Paths, text: &str) {
        let path = lock_path(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_whole_holder_line_needs_a_pid_a_version_and_the_newline() {
        assert_eq!(holder_pid("123 0.39.0\n"), Some(123));
        assert_eq!(holder_pid("123 0.39.0"), None);
        assert_eq!(holder_pid(""), None);
        assert_eq!(holder_pid("abc\n"), None);
        assert_eq!(holder_pid("abc 1\n"), None);
        assert_eq!(holder_pid("123 \n"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_dead_holder_line_is_waited_out_and_replaced() {
        let p = paths("daemon-dead-holder");
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let dead = child.id();
        child.wait().unwrap();
        seed(&p, &format!("{dead} 0.0.1\n"));
        let probe = shared_probe(&p, SEEDED_PROBE_HOLD);

        assert!(matches!(acquire(&p), Lock::Held(_)));
        probe.join().unwrap();
        assert_eq!(
            std::fs::read_to_string(lock_path(&p)).unwrap(),
            holder_line()
        );
    }

    #[test]
    fn a_partial_line_read_while_waiting_is_overwritten_from_the_start() {
        let p = paths("daemon-partial-rewind");
        seed(&p, "123456789 0.0.1-partial");
        let probe = shared_probe(&p, SEEDED_PROBE_HOLD);

        let Lock::Held(_held) = acquire(&p) else {
            panic!("acquire must succeed once the probe ends");
        };
        probe.join().unwrap();
        assert_eq!(
            std::fs::read(lock_path(&p)).unwrap(),
            holder_line().as_bytes()
        );
    }

    #[test]
    fn a_short_shared_probe_is_waited_out() {
        let p = paths("daemon-probe-short");
        let probe = shared_probe(&p, Duration::from_millis(60));

        assert!(matches!(acquire(&p), Lock::Held(_)));
        probe.join().unwrap();
    }

    #[test]
    fn a_probe_held_past_the_limit_reads_as_busy() {
        let p = paths("daemon-probe-long");
        let probe = shared_probe(&p, INCOMPLETE_HOLDER_WAIT + Duration::from_millis(500));

        assert!(matches!(acquire(&p), Lock::Busy));
        probe.join().unwrap();
    }

    #[test]
    fn a_complete_holder_line_is_busy_without_the_long_wait() {
        let p = paths("daemon-complete-busy");
        let Lock::Held(_held) = acquire(&p) else {
            panic!("first acquire must succeed");
        };
        let started = Instant::now();

        assert!(matches!(acquire(&p), Lock::Busy));
        assert!(started.elapsed() < INCOMPLETE_HOLDER_WAIT);
    }
}
