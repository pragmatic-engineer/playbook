// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook usage collect`: the lifecycle around the local receiver.
//!
//! A state file `collector.json` next to the usage database (owner-only)
//! holds the pid, the port and the bearer token. The token and the port are
//! kept across restarts, so sessions started before a restart keep working
//! and send to the same place. A start finds a live collector by pid and a
//! connect to its port, and then does not start a second one. A stale file
//! (dead pid) is replaced. The default port is 4318, the OTLP/HTTP standard;
//! when it is taken by something else the receiver picks a free port and
//! records it. It stops by itself after the idle time (default 10 minutes,
//! 0 means never) and clears the pid on a clean exit.
//!
//! Claude Code may drop data that is sent while no receiver listens. That is
//! by design: such sessions are read from transcripts.

use super::receiver::{bind, serve, Config};
use super::run::Paths;
use super::telemetry_db::migrate_up;
use crate::common::atomic::write_atomic_private;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 4318;
pub const DEFAULT_IDLE_SECONDS: u64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct State {
    /// 0 when no collector is running.
    pub pid: u32,
    pub port: u16,
    pub token: String,
}

pub fn state_path(paths: &Paths) -> PathBuf {
    paths
        .db
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("collector.json")
}

pub fn read_state(path: &Path) -> Option<State> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn write_state(path: &Path, state: &State) -> Result<(), String> {
    let json = serde_json::to_vec(state).map_err(|e| e.to_string())?;
    write_atomic_private(path, json).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    false
}

fn port_answers(port: u16) -> bool {
    TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        Duration::from_millis(300),
    )
    .is_ok()
}

/// The running collector recorded in `state`, if it is really alive.
pub fn running(state: &State) -> bool {
    state.pid != 0 && pid_alive(state.pid) && port_answers(state.port)
}

/// 32 hex characters from the OS random source.
fn new_token() -> Result<String, String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("failed to read random bytes: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Runs the collector in the foreground until idle or stopped. `port` is an
/// explicit request (an error when busy); `None` prefers the last port, then
/// the default, then any free port.
pub fn run_collect(
    paths: &Paths,
    port: Option<u16>,
    idle_seconds: u64,
    stop: &Arc<AtomicBool>,
) -> Result<String, String> {
    let path = state_path(paths);
    let previous = read_state(&path);
    if let Some(prev) = previous.as_ref().filter(|p| running(p)) {
        return Ok(format!(
            "collector already running on port {} (pid {})",
            prev.port, prev.pid
        ));
    }
    let conn = super::db::open_db(&paths.db)?;
    migrate_up(&conn)?;
    let listener = match port {
        Some(p) => bind(p).map_err(|e| format!("cannot listen on 127.0.0.1:{p}: {e}"))?,
        None => {
            let preferred = previous.as_ref().map_or(DEFAULT_PORT, |p| p.port);
            bind(preferred)
                .or_else(|_| bind(DEFAULT_PORT))
                .or_else(|_| bind(0))
                .map_err(|e| format!("cannot listen on 127.0.0.1: {e}"))?
        }
    };
    let actual = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token = match previous {
        Some(p) if !p.token.is_empty() => p.token,
        _ => new_token()?,
    };
    let mut state = State {
        pid: std::process::id(),
        port: actual,
        token: token.clone(),
    };
    write_state(&path, &state)?;
    let cfg = Config {
        token: Some(token),
        idle: (idle_seconds > 0).then(|| Duration::from_secs(idle_seconds)),
    };
    eprintln!(
        "usage collect: listening on 127.0.0.1:{actual} (pid {})",
        state.pid
    );
    let result = serve(&listener, &conn, &cfg, stop, crate::common::time::now_secs);
    state.pid = 0;
    let _ = write_state(&path, &state);
    match result {
        Ok(why) => Ok(format!("collector stopped ({why})")),
        Err(e) => Err(format!("collector failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::sync::atomic::Ordering;

    fn paths(name: &str) -> Paths {
        let dir = scratch_dir(name);
        let mut p = Paths::from_home(&dir);
        p.db = dir.join("usage").join("usage.db");
        p
    }

    #[test]
    fn a_dead_pid_is_not_running_and_our_own_pid_is() {
        let dead = State {
            pid: 0,
            port: 1,
            token: "t".into(),
        };
        assert!(!running(&dead));
        assert!(pid_alive(std::process::id()));
    }

    #[test]
    fn it_idles_out_keeps_token_and_port_and_clears_the_pid() {
        let p = paths("collector-idle");
        let stop = Arc::new(AtomicBool::new(false));
        let msg = run_collect(&p, Some(0), 1, &stop).unwrap();
        assert_eq!(msg, "collector stopped (idle)");
        let first = read_state(&state_path(&p)).unwrap();
        assert_eq!(first.pid, 0);
        assert_eq!(first.token.len(), 32);

        // A restart reuses the token (sessions started earlier keep working).
        run_collect(&p, Some(0), 1, &stop).unwrap();
        assert_eq!(read_state(&state_path(&p)).unwrap().token, first.token);
    }

    #[test]
    fn a_second_start_reports_the_live_one_and_does_not_start() {
        let p = paths("collector-live");
        let listener = bind(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = State {
            pid: std::process::id(),
            port,
            token: "t".into(),
        };
        std::fs::create_dir_all(state_path(&p).parent().unwrap()).unwrap();
        write_state(&state_path(&p), &state).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let msg = run_collect(&p, None, 1, &stop).unwrap();
        assert!(msg.contains("already running"), "{msg}");
    }

    #[test]
    fn a_stale_state_is_replaced() {
        let p = paths("collector-stale");
        std::fs::create_dir_all(state_path(&p).parent().unwrap()).unwrap();
        let stale = State {
            pid: 999_999,
            port: 1,
            token: "keep".into(),
        };
        write_state(&state_path(&p), &stale).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        run_collect(&p, None, 1, &stop).unwrap();
        let after = read_state(&state_path(&p)).unwrap();
        assert_eq!(after.token, "keep");
        assert_ne!(after.port, 1);
    }

    #[test]
    fn an_explicit_busy_port_is_an_error_and_the_default_falls_back() {
        let p = paths("collector-busy");
        let holder = bind(0).unwrap();
        let busy = holder.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        assert!(run_collect(&p, Some(busy), 1, &stop).is_err());
        stop.store(true, Ordering::SeqCst);
        assert!(run_collect(&p, None, 1, &stop).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn the_state_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let p = paths("collector-mode");
        let stop = Arc::new(AtomicBool::new(true));
        run_collect(&p, Some(0), 0, &stop).unwrap();
        let mode = std::fs::metadata(state_path(&p))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
