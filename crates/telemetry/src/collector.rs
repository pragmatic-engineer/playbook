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
use super::telemetry_db::{migrate_up, prune, Pruned, Retention};
use crate::util::atomic::write_atomic_private;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 4318;
pub const DEFAULT_IDLE_SECONDS: u64 = 600;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    /// 0 when no collector is running.
    pub pid: u32,
    pub port: u16,
    pub token: String,
    /// When the collector started, in unix seconds. 0 in a file written by an
    /// older version.
    #[serde(default)]
    pub started_at: i64,
    /// The idle limit it runs with, in seconds; 0 means never.
    #[serde(default)]
    pub idle_seconds: u64,
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

/// What answers on a port that `running` found open.
#[derive(Debug, PartialEq, Eq)]
pub enum Probe {
    /// A playbook collector replied.
    Collector,
    /// Something replied that is not a collector: the pid was reused.
    Foreign,
    /// Nothing replied in time: a collector stuck in a slow write, or a
    /// program that accepts and never answers.
    Silent,
}

const PROBE_DEADLINE: Duration = Duration::from_millis(1000);

/// Sends a harmless request to `port` and classifies the reply. A collector
/// answers every request, including this unauthenticated `GET`, with the
/// `X-Playbook-Collector` header.
pub fn probe(port: u16) -> Probe {
    use std::io::{Read, Write};
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, PROBE_DEADLINE) else {
        return Probe::Silent;
    };
    let _ = stream.set_read_timeout(Some(PROBE_DEADLINE));
    let _ = stream.set_write_timeout(Some(PROBE_DEADLINE));
    let request = "GET /v1/logs HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if stream.write_all(request.as_bytes()).is_err() {
        return Probe::Silent;
    }
    let mut buf = [0u8; 1024];
    let n = match stream.read(&mut buf) {
        Ok(n) if n > 0 => n,
        _ => return Probe::Silent,
    };
    classify_reply(&buf[..n])
}

fn classify_reply(reply: &[u8]) -> Probe {
    let text = String::from_utf8_lossy(reply).to_ascii_lowercase();
    if text.contains("x-playbook-collector:") {
        Probe::Collector
    } else {
        Probe::Foreign
    }
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

/// The collector state with a token and port recorded, creating the file when
/// there is none (no process, `pid` 0). `init --telemetry` uses this so the
/// settings it writes name the same port and token the collector reuses.
pub fn ensure_state(paths: &Paths) -> Result<State, String> {
    let path = state_path(paths);
    if let Some(state) = read_state(&path).filter(|s| !s.token.is_empty() && s.port != 0) {
        return Ok(state);
    }
    let previous = read_state(&path).unwrap_or_default();
    let state = State {
        pid: if running(&previous) { previous.pid } else { 0 },
        port: if previous.port == 0 {
            DEFAULT_PORT
        } else {
            previous.port
        },
        token: if previous.token.is_empty() {
            new_token()?
        } else {
            previous.token
        },
        started_at: previous.started_at,
        idle_seconds: previous.idle_seconds,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
    }
    write_state(&path, &state)?;
    Ok(state)
}

/// What `playbook usage status` reports.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    pub running: bool,
    pub pid: Option<u32>,
    /// The port of the running collector, else the one it used last.
    pub port: Option<u16>,
    pub started_at: Option<i64>,
    pub idle_seconds: Option<u64>,
    pub requests: i64,
    pub tool_calls: i64,
    pub api_errors: i64,
    pub metric_points: i64,
    /// The limits telemetry rows are pruned to.
    pub retention: Retention,
    /// Time of the newest stored event, unix seconds.
    pub last_event: Option<i64>,
    /// How telemetry is configured, in words, and the one-line fix if any.
    pub telemetry: String,
    pub telemetry_fix: Option<String>,
}

/// Reads the collector state file and the telemetry row counts. Starts
/// nothing, and never shows the token.
pub fn status(paths: &Paths, retention: Retention) -> Result<Status, String> {
    let state = read_state(&state_path(paths));
    let mode = super::telemetry_status::detect_here(paths);
    let live = state.as_ref().filter(|s| running(s));
    let (mut requests, mut tool_calls, mut api_errors, mut metric_points, mut last_event) =
        (0, 0, 0, 0, None);
    if paths.db.exists() {
        let conn = super::db::open_db(&paths.db)?;
        let count = |table: &str| -> Result<(i64, Option<i64>), String> {
            conn.query_row(
                &format!("SELECT COUNT(*), MAX(timestamp) FROM {table}"),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| format!("failed to read {table}: {e}"))
        };
        let (r, tr) = count("telemetry_requests")?;
        let (t, tt) = count("telemetry_tools")?;
        let (e, te) = count("telemetry_errors")?;
        let (m, tm) = count("telemetry_metrics")?;
        (requests, tool_calls, api_errors, metric_points) = (r, t, e, m);
        last_event = [tr, tt, te, tm].into_iter().flatten().max();
    }
    Ok(Status {
        running: live.is_some(),
        pid: live.map(|s| s.pid),
        port: state.as_ref().map(|s| s.port),
        started_at: live.map(|s| s.started_at).filter(|t| *t > 0),
        idle_seconds: live.map(|s| s.idle_seconds),
        requests,
        tool_calls,
        api_errors,
        metric_points,
        retention,
        last_event,
        telemetry: mode.text().to_string(),
        telemetry_fix: mode.fix().map(str::to_string),
    })
}

impl Status {
    /// The text a person reads.
    pub fn text(&self, now: i64) -> String {
        use crate::tz::iso;
        let mut lines = vec![self.telemetry.clone()];
        if let Some(fix) = &self.telemetry_fix {
            lines.push(format!("fix: {fix}"));
        }
        match (self.running, self.pid, self.port) {
            (true, Some(pid), Some(port)) => {
                lines.push(format!(
                    "collector: running (pid {pid}) on 127.0.0.1:{port}"
                ));
                if let Some(t) = self.started_at {
                    lines.push(format!("started: {}", iso(t)));
                }
                lines.push(match self.idle_seconds {
                    Some(0) | None => "idle shutdown: never".to_string(),
                    Some(s) => format!("idle shutdown: after {s} s with no request"),
                });
            }
            _ => {
                lines.push("collector: not running".to_string());
                lines.push(format!(
                    "start it with `{}`, or see the usage dashboard guide for a background setup",
                    super::host::command("usage collect")
                ));
                if let Some(port) = self.port {
                    lines.push(format!("last port: {port}"));
                }
            }
        }
        lines.push(format!(
            "rows: {} requests, {} tool calls, {} api errors, {} metric points",
            self.requests, self.tool_calls, self.api_errors, self.metric_points
        ));
        let limit = |n: u64, unit: &str| {
            if n == 0 {
                "no limit".to_string()
            } else {
                format!("{n} {unit}")
            }
        };
        lines.push(format!(
            "retention: {}, {} per table (usage.telemetryRetentionDays, usage.telemetryMaxRows)",
            limit(self.retention.days, "days"),
            limit(self.retention.max_rows, "rows")
        ));
        lines.push(match self.last_event {
            Some(t) => format!(
                "last event: {} ({} ago)",
                iso(t),
                age(now.saturating_sub(t))
            ),
            None => "last event: none".to_string(),
        });
        lines.join("\n")
    }
}

/// `45s`, `7m`, `3h`, `2d`.
fn age(seconds: i64) -> String {
    match seconds.max(0) {
        s if s < 60 => format!("{s}s"),
        s if s < 3_600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3_600),
        s => format!("{}d", s / 86_400),
    }
}

/// The telemetry limits from the `usage.telemetryRetentionDays` and
/// `usage.telemetryMaxRows` config keys, the defaults when unset or invalid.
pub fn configured_retention() -> Retention {
    let read = |key: &str, default: u64| {
        super::host::config_value_now(key)
            .and_then(|value| value.as_u64())
            .unwrap_or(default)
    };
    Retention {
        days: read(
            "usage.telemetryRetentionDays",
            super::telemetry_db::DEFAULT_RETENTION_DAYS,
        ),
        max_rows: read(
            "usage.telemetryMaxRows",
            super::telemetry_db::DEFAULT_MAX_ROWS,
        ),
    }
}

/// `playbook usage prune`: deletes telemetry rows past the limits, or with
/// `dry_run` only counts them. Creates no database when there is none.
pub fn run_prune(paths: &Paths, dry_run: bool) -> Result<String, String> {
    if !paths.db.exists() {
        return Ok("nothing to prune: no usage database yet".to_string());
    }
    let conn = super::db::open_db(&paths.db)?;
    migrate_up(&conn)?;
    let retention = configured_retention();
    let p: Pruned = prune(&conn, crate::util::time::now_secs(), retention, dry_run)?;
    let verb = if dry_run { "would remove" } else { "removed" };
    Ok(format!(
        "{verb} {} requests, {} tool calls, {} api errors, {} metric points (keeping {} days, {} rows per table; 0 means no limit)",
        p.requests, p.tools, p.errors, p.metrics, retention.days, retention.max_rows
    ))
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
    let mut foreign = false;
    if let Some(prev) = previous.as_ref().filter(|p| running(p)) {
        match probe(prev.port) {
            Probe::Collector => {
                return Ok(format!(
                    "collector already running on port {} (pid {})",
                    prev.port, prev.pid
                ));
            }
            Probe::Silent => {
                return Err(format!(
                    "something on port {} does not answer (recorded collector pid {}); check what holds the port before stopping anything, then run again",
                    prev.port, prev.pid
                ));
            }
            // The pid now belongs to another program: the state is stale.
            Probe::Foreign => foreign = true,
        }
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
    // A foreign program held the recorded port, so sessions may have sent it
    // the old bearer token; a fresh collector gets a fresh one.
    let token = match previous {
        Some(p) if !p.token.is_empty() && !foreign => p.token,
        _ => new_token()?,
    };
    let mut state = State {
        pid: std::process::id(),
        port: actual,
        token: token.clone(),
        started_at: crate::util::time::now_secs(),
        idle_seconds,
    };
    write_state(&path, &state)?;
    let retention = configured_retention();
    let _ = prune(&conn, crate::util::time::now_secs(), retention, false);
    let cfg = Config {
        token: Some(token),
        idle: (idle_seconds > 0).then(|| Duration::from_secs(idle_seconds)),
        prune: Some(retention),
    };
    crate::say::notice(&format!(
        "usage collect: listening on 127.0.0.1:{actual} (pid {})",
        state.pid
    ));
    let result = serve(&listener, &conn, &cfg, stop, crate::util::time::now_secs);
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
    use crate::util::scratch_dir;
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
            ..State::default()
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
            ..State::default()
        };
        std::fs::create_dir_all(state_path(&p).parent().unwrap()).unwrap();
        write_state(&state_path(&p), &state).unwrap();
        // A real receiver answers the probe.
        let db = crate::db::open_db(&p.db).unwrap();
        migrate_up(&db).unwrap();
        let serve_stop = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&serve_stop);
        let server = std::thread::spawn(move || {
            let cfg = Config {
                token: None,
                idle: None,
                prune: None,
            };
            let _ = serve(&listener, &db, &cfg, &server_stop, || 0);
        });
        let stop = Arc::new(AtomicBool::new(false));

        let msg = run_collect(&p, None, 1, &stop).unwrap();

        serve_stop.store(true, Ordering::SeqCst);
        let _ = server.join();
        assert!(msg.contains("already running"), "{msg}");
    }

    fn state_on(p: &Paths, port: u16) {
        let state = State {
            pid: std::process::id(),
            port,
            token: "t".into(),
            ..State::default()
        };
        std::fs::create_dir_all(state_path(p).parent().unwrap()).unwrap();
        write_state(&state_path(p), &state).unwrap();
    }

    #[test]
    fn a_listener_that_never_answers_is_reported_not_called_running() {
        let p = paths("collector-silent");
        let listener = bind(0).unwrap();
        state_on(&p, listener.local_addr().unwrap().port());
        let stop = Arc::new(AtomicBool::new(false));

        let err = run_collect(&p, None, 1, &stop).unwrap_err();

        assert!(err.contains("does not answer"), "{err}");
        drop(listener);
    }

    #[test]
    fn a_foreign_program_on_the_port_means_a_stale_state_and_a_fresh_start() {
        use std::io::{Read, Write};
        let p = paths("collector-foreign");
        let foreign = bind(0).unwrap();
        let port = foreign.local_addr().unwrap().port();
        state_on(&p, port);
        let server = std::thread::spawn(move || {
            // The bare liveness connect comes first, then the probe.
            for _ in 0..2 {
                if let Ok((mut s, _)) = foreign.accept() {
                    let mut buf = [0u8; 512];
                    let _ = s.read(&mut buf);
                    let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                }
            }
        });
        let stop = Arc::new(AtomicBool::new(false));

        let msg = run_collect(&p, None, 1, &stop).unwrap();

        let _ = server.join();
        assert!(!msg.contains("already running"), "{msg}");
        assert_ne!(
            read_state(&state_path(&p)).unwrap().token,
            "t",
            "the old token may have reached the foreign program"
        );
    }

    #[test]
    fn replies_are_classified_by_the_collector_header() {
        assert_eq!(
            classify_reply(b"HTTP/1.1 405 Method Not Allowed\r\nX-Playbook-Collector: 1\r\n\r\n"),
            Probe::Collector
        );
        assert_eq!(classify_reply(b"HTTP/1.1 200 OK\r\n\r\n"), Probe::Foreign);
    }

    #[test]
    fn a_stale_state_is_replaced() {
        let p = paths("collector-stale");
        std::fs::create_dir_all(state_path(&p).parent().unwrap()).unwrap();
        let stale = State {
            pid: 999_999,
            port: 1,
            token: "keep".into(),
            ..State::default()
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

    #[test]
    fn status_with_nothing_set_up_says_not_running_and_creates_no_files() {
        let p = paths("collector-status-none");
        let s = status(&p, Retention::default()).unwrap();
        assert!(!s.running && s.pid.is_none() && s.last_event.is_none());
        assert_eq!((s.requests, s.tool_calls, s.api_errors), (0, 0, 0));
        assert!(!p.db.exists(), "status must not create the database");
        let text = s.text(1_000);
        assert!(
            text.contains("not running") && text.contains("last event: none"),
            "{text}"
        );
    }

    #[test]
    fn status_reports_a_live_collector_its_rows_and_the_last_event_without_the_token() {
        use crate::telemetry_db::{insert_request, insert_tool, TelemetryRequest, TelemetryTool};
        let p = paths("collector-status-live");
        let listener = bind(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::create_dir_all(state_path(&p).parent().unwrap()).unwrap();
        let state = State {
            pid: std::process::id(),
            port,
            token: "supersecrettoken".into(),
            started_at: 1_760_000_000,
            idle_seconds: 600,
        };
        write_state(&state_path(&p), &state).unwrap();
        let conn = crate::db::open_db(&p.db).unwrap();
        let request = TelemetryRequest {
            session_id: "s".into(),
            request_id: "r".into(),
            timestamp: 1_760_000_100,
            ..TelemetryRequest::default()
        };
        insert_request(&conn, &request).unwrap();
        let tool = TelemetryTool {
            session_id: "s".into(),
            tool_use_id: "t".into(),
            timestamp: 1_760_000_200,
            ..TelemetryTool::default()
        };
        insert_tool(&conn, &tool).unwrap();
        drop(conn);

        let s = status(&p, Retention::default()).unwrap();
        assert!(s.running);
        assert_eq!((s.pid, s.port), (Some(std::process::id()), Some(port)));
        assert_eq!((s.requests, s.tool_calls, s.api_errors), (1, 1, 0));
        assert_eq!(s.last_event, Some(1_760_000_200));
        let text = s.text(1_760_000_260);
        assert!(
            text.contains("running") && text.contains("after 600 s"),
            "{text}"
        );
        assert!(
            text.contains("1 requests, 1 tool calls, 0 api errors"),
            "{text}"
        );
        assert!(text.contains("(1m ago)"), "{text}");
        assert!(
            text.contains("0 metric points")
                && text.contains("retention: 90 days, 500000 rows per table"),
            "{text}"
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(!text.contains("supersecret") && !json.contains("supersecret"));
    }

    #[test]
    fn prune_says_so_without_a_database_and_otherwise_removes_old_rows() {
        let p = paths("collector-prune");
        assert!(run_prune(&p, false).unwrap().contains("no usage database"));
        assert!(!p.db.exists(), "prune must not create the database");
        std::fs::create_dir_all(p.db.parent().unwrap()).unwrap();
        let conn = crate::db::open_db(&p.db).unwrap();
        migrate_up(&conn).unwrap();
        let old = crate::telemetry_db::TelemetryTool {
            session_id: "s".into(),
            tool_use_id: "old".into(),
            timestamp: 1_000,
            ..Default::default()
        };
        crate::telemetry_db::insert_tool(&conn, &old).unwrap();
        drop(conn);
        let dry = run_prune(&p, true).unwrap();
        assert!(
            dry.starts_with("would remove 0 requests, 1 tool calls"),
            "{dry}"
        );
        let done = run_prune(&p, false).unwrap();
        assert!(
            done.starts_with("removed 0 requests, 1 tool calls"),
            "{done}"
        );
        assert!(run_prune(&p, false)
            .unwrap()
            .starts_with("removed 0 requests, 0 tool calls"));
    }

    #[test]
    fn a_stale_state_reads_as_not_running_and_keeps_the_last_port() {
        let p = paths("collector-status-stale");
        std::fs::create_dir_all(state_path(&p).parent().unwrap()).unwrap();
        let stale = State {
            pid: 999_999,
            port: 4318,
            token: "t".into(),
            ..State::default()
        };
        write_state(&state_path(&p), &stale).unwrap();
        let s = status(&p, Retention::default()).unwrap();
        assert!(!s.running);
        assert!(s.text(1).contains("last port: 4318"));
    }

    #[test]
    fn a_state_file_from_an_older_version_still_reads() {
        let old = br#"{"pid":5,"port":4318,"token":"abc"}"#;
        let state: State = serde_json::from_slice(old).unwrap();
        assert_eq!((state.started_at, state.idle_seconds), (0, 0));
    }

    #[test]
    fn ages_read_in_their_largest_unit() {
        assert_eq!(
            [age(5), age(125), age(7_300), age(200_000)],
            ["5s", "2m", "2h", "2d"]
        );
    }
}
