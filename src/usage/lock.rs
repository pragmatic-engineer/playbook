// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The dashboard server's PID and port lock file. The file alone proves
//! nothing (a killed server leaves it behind), so a lock only counts as live
//! when its process is alive AND its port still accepts a connection.

use crate::common::atomic::{
    ensure_private_dir, remove_stale_lock_dir, with_dir_lock, STALE_LOCK_AGE,
};
use crate::worktree::pid_is_alive;
use std::fs;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(unix)]
const PS_TIMEOUT: Duration = Duration::from_secs(2);
const LOCK_RETRIES: u32 = 50;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(20);

const TOKEN_BYTES: usize = 32;
const TOKEN_HEX_LEN: usize = TOKEN_BYTES * 2;

/// The per-session secret the data route requires. Hex text, fixed size so a
/// `Lock` stays `Copy`. `Debug` hides it, so it cannot reach a log or an error.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Token([u8; TOKEN_HEX_LEN]);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(hidden)")
    }
}

impl Token {
    /// 32 random bytes from the system, hex encoded.
    #[cfg(unix)]
    pub fn generate() -> Result<Token, String> {
        use std::io::Read;
        let mut raw = [0u8; TOKEN_BYTES];
        fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut raw))
            .map_err(|e| format!("could not read random bytes for the session token: {e}"))?;
        let mut hex = [0u8; TOKEN_HEX_LEN];
        for (i, byte) in raw.iter().enumerate() {
            hex[i * 2] = HEX[usize::from(byte >> 4)];
            hex[i * 2 + 1] = HEX[usize::from(byte & 0x0f)];
        }
        Ok(Token(hex))
    }

    #[cfg(not(unix))]
    pub fn generate() -> Result<Token, String> {
        Err("the usage dashboard is only supported on macOS and Linux".to_string())
    }

    /// Exactly 64 lowercase hex characters, or `None`.
    pub fn parse(text: &str) -> Option<Token> {
        let bytes = text.as_bytes();
        if bytes.len() != TOKEN_HEX_LEN || !bytes.iter().all(|b| HEX.contains(b)) {
            return None;
        }
        let mut hex = [0u8; TOKEN_HEX_LEN];
        hex.copy_from_slice(bytes);
        Some(Token(hex))
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or_default()
    }

    /// Compares without stopping at the first differing byte.
    pub fn matches(&self, candidate: &str) -> bool {
        let given = candidate.as_bytes();
        let mut diff = u8::from(given.len() != self.0.len());
        for (i, expected) in self.0.iter().enumerate() {
            diff |= expected ^ given.get(i).copied().unwrap_or(0);
        }
        diff == 0
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lock {
    pub pid: u32,
    pub port: u16,
    pub token: Token,
}

/// Pids 0 and 1 are never a dashboard server: signalling 0 hits the caller's
/// whole process group, so a lock naming either reads as garbled. A lock
/// without a token is the older two field format and reads as `None`.
pub fn read_lock(path: &Path) -> Option<Lock> {
    let text = fs::read_to_string(path).ok()?;
    let mut parts = text.split_whitespace();
    let lock = Lock {
        pid: parts.next()?.parse().ok()?,
        port: parts.next()?.parse().ok()?,
        token: Token::parse(parts.next()?)?,
    };
    (lock.pid > 1 && lock.port > 0).then_some(lock)
}

/// The pid and port of an older two field lock, so an upgrade can retire the
/// server it names instead of leaving it running without an owner.
pub fn read_legacy_lock(path: &Path) -> Option<(u32, u16)> {
    let text = fs::read_to_string(path).ok()?;
    let mut parts = text.split_whitespace();
    let pid: u32 = parts.next()?.parse().ok()?;
    let port: u16 = parts.next()?.parse().ok()?;
    (parts.next().is_none() && pid > 1 && port > 0).then_some((pid, port))
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

/// Whether `pid` is a dashboard server: its command line carries the hidden
/// `usage dashboard --serve` arguments `spawn_detached` starts it with. A live
/// pid and an open port alone prove nothing once a pid has been reused.
#[cfg(unix)]
pub fn is_dashboard_process(pid: u32) -> bool {
    let mut command = std::process::Command::new("ps");
    command.args(["-p", &pid.to_string(), "-o", "command="]);
    let Some(out) = crate::common::proc::run_with_timeout(&mut command, PS_TIMEOUT) else {
        return false;
    };
    let line = String::from_utf8_lossy(&out.stdout);
    out.status.success()
        && ["usage", "dashboard", "--serve"]
            .iter()
            .all(|word| line.split_whitespace().any(|part| part == *word))
}

#[cfg(not(unix))]
pub fn is_dashboard_process(_pid: u32) -> bool {
    false
}

/// The directory lock beside `path`. Its parent is created first: `mkdir`
/// fails when the parent is missing, and `with_dir_lock` then fails open, so
/// two starters on a fresh home would both run unguarded.
fn guard_path(path: &Path) -> PathBuf {
    if let Some(parent) = path.parent() {
        let _ = ensure_private_dir(parent);
    }
    let guard = PathBuf::from(format!("{}.guard", path.display()));
    remove_stale_lock_dir(&guard, STALE_LOCK_AGE);
    guard
}

/// Claims the lock for `me` unless a DIFFERENT live server already holds it.
/// Returns whether `me` now owns it. The check and the write happen under one
/// directory lock, so two servers racing to start cannot both win; the loser
/// must see `false` and stop, since an atomic write alone does not stop a
/// process that is already running.
pub fn try_claim(path: &Path, me: Lock) -> bool {
    let guard = guard_path(path);
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
    let guard = guard_path(path);
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
        if ensure_private_dir(parent).is_err() {
            return false;
        }
    }
    let body = format!(
        "{} {} {}
",
        lock.pid,
        lock.port,
        lock.token.as_str()
    );
    crate::common::atomic::write_atomic_private(path, body).is_ok()
}

/// Serializes unit tests that bind loopback sockets: a just-dropped ephemeral
/// port could otherwise be rebound by a sibling test.
#[cfg(test)]
pub(crate) static SOCKET_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::net::TcpListener;

    fn tok() -> Token {
        Token::parse(&"ab".repeat(32)).expect("64 hex characters")
    }

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
        fs::write(&path, format!("42 8080 {}\n", tok().as_str())).unwrap();
        assert_eq!(
            read_lock(&path),
            Some(Lock {
                pid: 42,
                port: 8080,
                token: tok(),
            })
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_lock_naming_pid_zero_or_one_or_port_zero_reads_as_garbled() {
        let dir = scratch_dir("lock-bad-pid");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dashboard.lock");

        let token = tok();
        for text in [
            format!("0 8080 {}\n", token.as_str()),
            format!("1 8080 {}\n", token.as_str()),
            format!("4242 0 {}\n", token.as_str()),
        ] {
            fs::write(&path, &text).unwrap();
            assert_eq!(read_lock(&path), None, "{text:?}");
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn only_a_process_started_as_the_dashboard_server_is_recognised() {
        use std::process::Command;

        // Arrange: a child whose command line carries the server's arguments,
        // and this test process, which does not.
        let mut server = Command::new("sh")
            .args(["-c", "sleep 5; true", "usage", "dashboard", "--serve"])
            .spawn()
            .expect("sh starts");
        let mut other = Command::new("sleep")
            .arg("5")
            .spawn()
            .expect("sleep starts");

        // Act
        let server_ok = is_dashboard_process(server.id());
        let other_ok = is_dashboard_process(other.id());
        let me_ok = is_dashboard_process(std::process::id());
        let gone_ok = is_dashboard_process(0x7fff_fffe);
        let _ = server.kill();
        let _ = other.kill();
        let _ = server.wait();
        let _ = other.wait();

        // Assert
        assert!(server_ok, "the server's own arguments must match");
        assert!(!other_ok && !me_ok && !gone_ok);
    }

    #[cfg(unix)]
    #[test]
    fn a_stale_guard_directory_does_not_make_claims_slow_or_unguarded() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Arrange: a guard left behind by a starter killed in its critical
        // section, plus a live holder whose claim must still be honoured.
        let dir = scratch_dir("lock-stale-guard");
        let path = dir.join("dashboard.lock");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let holder = Lock {
            pid: std::process::id(),
            port: listener.local_addr().unwrap().port(),
            token: tok(),
        };
        assert!(try_claim(&path, holder));
        let guard = PathBuf::from(format!("{}.guard", path.display()));
        fs::create_dir(&guard).unwrap();
        let past = std::time::SystemTime::now() - STALE_LOCK_AGE * 3;
        fs::File::open(&guard)
            .and_then(|h| h.set_modified(past))
            .unwrap();
        let started = std::time::Instant::now();

        // Act
        let loser_won = try_claim(
            &path,
            Lock {
                pid: 0x7fff_fffd,
                port: 9,
                token: tok(),
            },
        );

        // Assert
        assert!(!loser_won, "the live holder must still win");
        assert!(
            started.elapsed() < Duration::from_millis(400),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(read_lock(&path), Some(holder));
        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "the usage directory must be owner-only");
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
            port,
            token: tok(),
        }));
    }

    #[test]
    fn a_live_pid_with_nothing_listening_is_not_live() {
        let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let port = free_port_with_nothing_listening();

        assert!(!is_live(Lock {
            pid: std::process::id(),
            port,
            token: tok(),
        }));
    }

    #[test]
    fn a_live_pid_with_a_listening_port_is_live() {
        let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        assert!(is_live(Lock {
            pid: std::process::id(),
            port,
            token: tok(),
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
            token: tok(),
        };
        let loser = Lock {
            pid: 0x7fff_fffd,
            port: 9,
            token: tok(),
        };

        assert!(try_claim(&path, winner));
        assert!(!try_claim(&path, loser));

        assert_eq!(read_lock(&path), Some(winner));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn claiming_in_a_missing_directory_does_not_wait_out_the_lock_retries() {
        let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch_dir("lock-fresh-dir");
        let path = dir.join("dashboard.lock");
        let started = std::time::Instant::now();

        let won = try_claim(
            &path,
            Lock {
                pid: std::process::id(),
                port: 1,
                token: tok(),
            },
        );

        // An unguarded fail-open costs the full retry budget (about 1s).
        assert!(won);
        assert!(
            started.elapsed() < Duration::from_millis(800),
            "{:?}",
            started.elapsed()
        );
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
            token: tok(),
        };
        let fresh = Lock {
            pid: std::process::id(),
            port: 1,
            token: tok(),
        };
        assert!(try_claim(&path, stale));

        assert!(try_claim(&path, fresh));

        assert_eq!(read_lock(&path), Some(fresh));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn racing_claimants_produce_exactly_one_winner() {
        let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // The directory is deliberately not created: a fresh home has none.
        let dir = scratch_dir("lock-race");
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
                token: tok(),
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

    #[test]
    fn tokens_parse_only_as_64_lowercase_hex_characters() {
        assert!(Token::parse(&"0f".repeat(32)).is_some());
        for bad in [
            "",
            "abc",
            &"AB".repeat(32),
            &"zz".repeat(32),
            &"ab".repeat(33),
        ] {
            assert!(Token::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn a_token_matches_only_itself_and_never_prints() {
        let token = tok();

        assert!(token.matches(token.as_str()));
        assert!(!token.matches(""));
        assert!(!token.matches(&"ab".repeat(31)));
        assert!(!token.matches(&format!("{}0", token.as_str())));
        assert!(!token.matches(&"cd".repeat(32)));
        assert_eq!(format!("{token:?}"), "Token(hidden)");
        assert!(!format!(
            "{:?}",
            Lock {
                pid: 5,
                port: 6,
                token
            }
        )
        .contains("abab"));
    }

    #[cfg(unix)]
    #[test]
    fn generated_tokens_are_random_hex() {
        let (a, b) = (Token::generate().unwrap(), Token::generate().unwrap());

        assert_ne!(a, b);
        assert_eq!(a.as_str().len(), 64);
        assert!(Token::parse(a.as_str()).is_some());
    }

    #[test]
    fn a_two_field_lock_is_the_old_format_and_reads_as_none() {
        let dir = scratch_dir("lock-legacy");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dashboard.lock");
        fs::write(&path, "4242 8080\n").unwrap();

        assert_eq!(read_lock(&path), None);
        assert_eq!(read_legacy_lock(&path), Some((4242, 8080)));
        fs::write(&path, format!("4242 8080 {}\n", tok().as_str())).unwrap();
        assert_eq!(read_legacy_lock(&path), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_lock_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = scratch_dir("lock-mode");
        let path = dir.join("dashboard.lock");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let me = Lock {
            pid: std::process::id(),
            port: listener.local_addr().unwrap().port(),
            token: tok(),
        };

        assert!(try_claim(&path, me));

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = fs::remove_dir_all(dir);
    }
}
