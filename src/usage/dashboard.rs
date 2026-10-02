// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The local dashboard server. `playbook usage dashboard` is a short-lived
//! process: it reuses a live server or starts a detached one, opens the
//! browser, and exits. The server itself is a second process (the same binary
//! with a hidden flag) that outlives its parent, so no other `playbook`
//! command ever waits on it.

use super::lock::{self, Lock};
use super::run::Paths;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a starter waits for the new server to write its own lock.
pub const LOCK_READY_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

pub type Opener<'a> = &'a dyn Fn(&str) -> Result<(), String>;

pub fn ip_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// `*.localhost` resolves to loopback on macOS and most Linux resolvers but
/// not all, so it is only ever printed, never used to open the browser.
pub fn alias_url(port: u16) -> String {
    format!("http://playbook.localhost:{port}")
}

pub fn open_in_browser(url: &str) -> Result<(), String> {
    let program = match std::env::consts::OS {
        "macos" => "open",
        "linux" => "xdg-open",
        other => return Err(format!("no browser launcher known for {other}")),
    };
    std::process::Command::new(program)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to run {program}: {e}"))
}

pub fn no_browser(_url: &str) -> Result<(), String> {
    Ok(())
}

/// What the server answers for a path: (status, content type, body).
pub fn route(path: &str) -> (u16, &'static str, String) {
    match path {
        "/" => (
            200,
            "text/html; charset=utf-8",
            "<!doctype html><title>playbook usage</title><h1>playbook usage dashboard</h1>".into(),
        ),
        _ => (404, "text/plain; charset=utf-8", "not found".into()),
    }
}

#[cfg(unix)]
fn spawn_detached(exe: &Path) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut command = Command::new(exe);
    command
        .args(["usage", "dashboard", "--serve"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid only changes the child's own session and is
    // async-signal-safe, so it is allowed between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to start the dashboard server: {e}"))
}

#[cfg(not(unix))]
fn spawn_detached(_exe: &Path) -> Result<(), String> {
    Err("the usage dashboard is only supported on macOS and Linux".to_string())
}

fn wait_for_live_lock(path: &Path) -> Result<Lock, String> {
    let deadline = Instant::now() + LOCK_READY_TIMEOUT;
    loop {
        if let Some(found) = lock::read_lock(path) {
            if lock::is_live(found) {
                return Ok(found);
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the dashboard server did not become ready within {} seconds",
                LOCK_READY_TIMEOUT.as_secs()
            ));
        }
        std::thread::sleep(LOCK_POLL_INTERVAL);
    }
}

/// `playbook usage dashboard`.
pub fn run_dashboard(paths: &Paths, exe: &Path, open: Opener) -> Result<String, String> {
    lock::clear_if_stale(&paths.lock);
    let existing = lock::read_lock(&paths.lock).filter(|l| lock::is_live(*l));
    let (running, started) = match existing {
        Some(found) => (found, false),
        None => {
            spawn_detached(exe)?;
            (wait_for_live_lock(&paths.lock)?, true)
        }
    };

    let url = ip_url(running.port);
    let opened = open(&url);
    let mut out = format!(
        "usage dashboard {} on {url}\n  also: {} (works where *.localhost resolves)",
        if started {
            "started"
        } else {
            "already running"
        },
        alias_url(running.port),
    );
    if let Err(e) = opened {
        out.push_str(&format!(
            "\n  could not open a browser ({e}); open the URL yourself"
        ));
    }
    Ok(out)
}

/// `playbook usage dashboard stop`.
pub fn run_stop(paths: &Paths) -> Result<String, String> {
    let Some(found) = lock::read_lock(&paths.lock) else {
        return Ok("usage dashboard: nothing is running".to_string());
    };
    // Only a live server is signalled: a stale lock's pid may now belong to
    // an unrelated process.
    if !lock::is_live(found) {
        lock::clear_lock(&paths.lock);
        return Ok("usage dashboard: nothing is running (removed a stale lock)".to_string());
    }
    terminate(found.pid)?;
    let deadline = Instant::now() + STOP_TIMEOUT;
    while crate::worktree::pid_is_alive(found.pid) {
        if Instant::now() >= deadline {
            return Err(format!(
                "the dashboard server (pid {}) did not stop",
                found.pid
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    lock::clear_lock(&paths.lock);
    Ok(format!("usage dashboard: stopped (pid {})", found.pid))
}

#[cfg(unix)]
fn terminate(pid: u32) -> Result<(), String> {
    let pid = i32::try_from(pid).map_err(|_| format!("pid {pid} is out of range"))?;
    // SAFETY: kill with a valid pid and signal has no memory-safety effect.
    if unsafe { libc::kill(pid, libc::SIGTERM) } == -1 {
        return Err(format!(
            "failed to signal pid {pid}: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn terminate(_pid: u32) -> Result<(), String> {
    Err("the usage dashboard is only supported on macOS and Linux".to_string())
}

/// The detached child: bind first, then claim the lock with the real port. A
/// loser of a start-up race closes its listener and returns without serving.
pub fn serve(paths: &Paths) -> Result<(), String> {
    let server = tiny_http::Server::http("127.0.0.1:0")
        .map_err(|e| format!("failed to bind the dashboard server: {e}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|addr| addr.port())
        .ok_or("the dashboard server has no IP address")?;
    let me = Lock {
        pid: std::process::id(),
        port,
    };
    if !lock::try_claim(&paths.lock, me) {
        return Ok(());
    }
    for request in server.incoming_requests() {
        let path = request.url().split('?').next().unwrap_or("/").to_string();
        let (status, content_type, body) = route(&path);
        let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
            .map_err(|_| "invalid content type header".to_string())?;
        let response = tiny_http::Response::from_string(body)
            .with_status_code(status)
            .with_header(header);
        let _ = request.respond(response);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_use_the_ip_for_launching_and_the_alias_only_for_display() {
        assert_eq!(ip_url(4242), "http://127.0.0.1:4242");
        assert_eq!(alias_url(4242), "http://playbook.localhost:4242");
    }

    #[test]
    fn root_serves_html_and_anything_else_is_a_404() {
        let (status, content_type, body) = route("/");
        assert_eq!(status, 200);
        assert!(content_type.starts_with("text/html"));
        assert!(body.contains("playbook usage dashboard"));
        assert_eq!(route("/nope").0, 404);
    }
}
