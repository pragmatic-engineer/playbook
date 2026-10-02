// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The dashboard server's PID and port lock file. The file alone proves
//! nothing (a killed server leaves it behind), so a lock only counts as live
//! when its process is alive AND its port still accepts a connection.

use crate::common::atomic::with_dir_lock;
use crate::worktree::pid_is_alive;
use std::fs;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const LOCK_RETRIES: u32 = 50;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lock {
    pub pid: u32,
    pub port: u16,
}

pub fn read_lock(path: &Path) -> Option<Lock> {
    let text = fs::read_to_string(path).ok()?;
    let mut parts = text.split_whitespace();
    Some(Lock {
        pid: parts.next()?.parse().ok()?,
        port: parts.next()?.parse().ok()?,
    })
}

pub fn clear_lock(path: &Path) {
    let _ = fs::remove_file(path);
}

pub fn port_accepts_connections(port: u16) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).is_ok()
}

pub fn is_live(lock: Lock) -> bool {
    pid_is_alive(lock.pid) && port_accepts_connections(lock.port)
}

/// Claims the lock for `me` unless a DIFFERENT live server already holds it.
/// Returns whether `me` now owns it. The check and the write happen under one
/// directory lock, so two servers racing to start cannot both win; the loser
/// must see `false` and stop, since an atomic write alone does not stop a
/// process that is already running.
pub fn try_claim(path: &Path, me: Lock) -> bool {
    let guard = PathBuf::from(format!("{}.guard", path.display()));
    let (acquired, won) = with_dir_lock(&guard, LOCK_RETRIES, LOCK_RETRY_DELAY, || {
        if let Some(existing) = read_lock(path) {
            if existing != me && is_live(existing) {
                return false;
            }
        }
        write_atomically(path, me)
    });
    if acquired {
        let _ = fs::remove_dir(&guard);
    }
    won
}

/// Removes the lock file unless a live server holds it. Check and remove run
/// under the same directory lock as `try_claim`, so a starter cannot delete
/// a lock another starter's server has just claimed.
pub fn clear_if_stale(path: &Path) {
    let guard = PathBuf::from(format!("{}.guard", path.display()));
    let (acquired, ()) = with_dir_lock(&guard, LOCK_RETRIES, LOCK_RETRY_DELAY, || {
        if !read_lock(path).is_some_and(is_live) {
            clear_lock(path);
        }
    });
    if acquired {
        let _ = fs::remove_dir(&guard);
    }
}

fn write_atomically(path: &Path, lock: Lock) -> bool {
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    let tmp = PathBuf::from(format!("{}.tmp.{}", path.display(), lock.pid));
    if fs::write(&tmp, format!("{} {}\n", lock.pid, lock.port)).is_err() {
        return false;
    }
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::net::TcpListener;
    use std::sync::Mutex;

    // Tests that bind real loopback sockets run one at a time: a just-dropped
    // ephemeral port could otherwise be rebound by a sibling test.
    static SOCKET_LOCK: Mutex<()> = Mutex::new(());

    fn free_port_with_nothing_listening() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    #[test]
    fn a_missing_or_garbled_lock_file_reads_as_none() {
        let dir = scratch_dir("lock-read");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dashboard.lock");

        assert_eq!(read_lock(&path), None);
        fs::write(&path, "not a lock").unwrap();
        assert_eq!(read_lock(&path), None);
        fs::write(&path, "42 8080\n").unwrap();
        assert_eq!(
            read_lock(&path),
            Some(Lock {
                pid: 42,
                port: 8080
            })
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_dead_pid_is_not_live() {
        let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // Pid 0x7fff_fffe is far above any real pid limit.
        assert!(!is_live(Lock {
            pid: 0x7fff_fffe,
            port
        }));
    }

    #[test]
    fn a_live_pid_with_nothing_listening_is_not_live() {
        let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let port = free_port_with_nothing_listening();

        assert!(!is_live(Lock {
            pid: std::process::id(),
            port
        }));
    }

    #[test]
    fn a_live_pid_with_a_listening_port_is_live() {
        let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        assert!(is_live(Lock {
            pid: std::process::id(),
            port
        }));
    }

    #[test]
    fn a_second_claimant_loses_to_a_live_holder_and_the_file_keeps_the_winner() {
        let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch_dir("lock-claim");
        let path = dir.join("dashboard.lock");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let winner = Lock {
            pid: std::process::id(),
            port: listener.local_addr().unwrap().port(),
        };
        let loser = Lock {
            pid: 0x7fff_fffd,
            port: 9,
        };

        assert!(try_claim(&path, winner));
        assert!(!try_claim(&path, loser));

        assert_eq!(read_lock(&path), Some(winner));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stale_holder_is_replaced() {
        let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch_dir("lock-stale");
        let path = dir.join("dashboard.lock");
        let stale = Lock {
            pid: 0x7fff_fffe,
            port: free_port_with_nothing_listening(),
        };
        let fresh = Lock {
            pid: std::process::id(),
            port: 1,
        };
        assert!(try_claim(&path, stale));

        assert!(try_claim(&path, fresh));

        assert_eq!(read_lock(&path), Some(fresh));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn racing_claimants_produce_exactly_one_winner() {
        let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch_dir("lock-race");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dashboard.lock");
        // Eight claimants, each a live listener on its own port under this
        // live pid, so whichever writes first is a live holder the rest lose to.
        let listeners: Vec<TcpListener> = (0..8)
            .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
            .collect();
        let claimants: Vec<Lock> = listeners
            .iter()
            .map(|l| Lock {
                pid: std::process::id(),
                port: l.local_addr().unwrap().port(),
            })
            .collect();

        let handles: Vec<_> = claimants
            .iter()
            .map(|&me| {
                let path = path.clone();
                std::thread::spawn(move || (me, try_claim(&path, me)))
            })
            .collect();
        let results: Vec<(Lock, bool)> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        let winners: Vec<Lock> = results
            .iter()
            .filter(|(_, won)| *won)
            .map(|(l, _)| *l)
            .collect();
        assert_eq!(winners.len(), 1, "exactly one claimant may own the lock");
        assert_eq!(read_lock(&path), Some(winners[0]));
        let _ = fs::remove_dir_all(dir);
    }
}
