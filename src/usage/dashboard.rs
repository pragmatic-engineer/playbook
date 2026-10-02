// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The local dashboard server. `playbook usage dashboard` is a short-lived
//! process: it reuses a live server or starts a detached one, opens the
//! browser, and exits. The server itself is a second process (the same binary
//! with a hidden flag) that outlives its parent, so no other `playbook`
//! command ever waits on it.

use super::lock::{self, Lock, Token};
use super::page;
use super::run::Paths;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a starter waits for the new server to write its own lock.
pub const LOCK_READY_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

pub type Opener<'a> = &'a dyn Fn(&str) -> Result<(), String>;

/// The link the browser opens. The token rides in the fragment, which a
/// browser never sends to the server, so it stays out of requests and logs.
pub fn ip_url(port: u16, token: &Token) -> String {
    format!("http://127.0.0.1:{port}/#{}", token.as_str())
}

/// `*.localhost` resolves to loopback on macOS and most Linux resolvers but
/// not all, so it is only ever printed, never used to open the browser.
pub fn alias_url(port: u16, token: &Token) -> String {
    format!("http://playbook.localhost:{port}/#{}", token.as_str())
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

/// Hosts the server answers to. A page on another origin, including a
/// DNS-rebinding one, arrives with a different `Host`, so it gets nothing.
pub fn host_allowed(host: Option<&str>, port: u16) -> bool {
    let Some(host) = host else { return false };
    [
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("playbook.localhost:{port}"),
    ]
    .iter()
    .any(|allowed| allowed == host)
}

/// Browsers label every request with `Sec-Fetch-Site`. The page's own fetch is
/// `same-origin` and a typed URL is `none`; another site's page is
/// `cross-site` or `same-site` and must not make the server ingest. A client
/// that sends no label (curl) is not a browser page and is allowed.
pub fn fetch_site_allowed(value: Option<&str>) -> bool {
    matches!(value, None | Some("same-origin") | Some("none"))
}

/// What the server answers for a path: (status, content type, body). The page
/// assets carry no data and need no token. `load_data` runs only for
/// `/api/data`, and only when `token_ok`.
pub fn respond_to(
    path: &str,
    token_ok: bool,
    load_data: impl FnOnce() -> Result<String, String>,
) -> (u16, &'static str, String) {
    match path {
        "/" => (200, "text/html; charset=utf-8", page::HTML.to_string()),
        "/app.js" => (200, "text/javascript; charset=utf-8", page::JS.to_string()),
        "/app.css" => (200, "text/css; charset=utf-8", page::CSS.to_string()),
        "/api/data" if !token_ok => (401, "text/plain; charset=utf-8", "unauthorized".to_string()),
        "/api/data" => match load_data() {
            Ok(body) => (200, "application/json", body),
            Err(e) => (
                500,
                "application/json",
                serde_json::json!({ "error": e }).to_string(),
            ),
        },
        _ => (404, "text/plain; charset=utf-8", "not found".to_string()),
    }
}

/// Ingest anything new, then build the dashboard JSON.
fn load_data(paths: &Paths) -> Result<String, String> {
    let (conn, _) = super::run::ingest_new(paths)?;
    let usage = super::db::load_usage_events(&conn)?;
    let tools = super::db::load_tool_events(&conn)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    serde_json::to_string(&super::api::data_json(&usage, &tools, now))
        .map_err(|e| format!("failed to encode usage data: {e}"))
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

/// An older server wrote a two field lock and has no token, so nothing can
/// authenticate to it. Retire it (only if it really is a dashboard server)
/// rather than leave it running without an owner.
fn retire_legacy_server(paths: &Paths) {
    let Some((pid, port)) = lock::read_legacy_lock(&paths.lock) else {
        return;
    };
    let named_is_live = crate::worktree::pid_is_alive(pid) && lock::port_accepts_connections(port);
    if named_is_live && lock::is_dashboard_process(pid) && terminate(pid).is_ok() {
        let deadline = Instant::now() + STOP_TIMEOUT;
        while crate::worktree::pid_is_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    lock::clear_lock(&paths.lock);
}

/// `playbook usage dashboard`.
pub fn run_dashboard(paths: &Paths, exe: &Path, open: Opener) -> Result<String, String> {
    retire_legacy_server(paths);
    lock::clear_if_stale(&paths.lock);
    let existing = lock::read_lock(&paths.lock).filter(|l| lock::is_live(*l));
    let (running, started) = match existing {
        Some(found) => (found, false),
        None => {
            spawn_detached(exe)?;
            (wait_for_live_lock(&paths.lock)?, true)
        }
    };

    let url = ip_url(running.port, &running.token);
    let opened = open(&url);
    let mut out = format!(
        "usage dashboard {} on {url}\n  also: {} (works where *.localhost resolves)",
        if started {
            "started"
        } else {
            "already running"
        },
        alias_url(running.port, &running.token),
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
    // Only a live dashboard server is signalled: a stale or tampered lock's
    // pid may now belong to an unrelated process.
    if !lock::is_live(found) {
        lock::clear_lock(&paths.lock);
        return Ok("usage dashboard: nothing is running (removed a stale lock)".to_string());
    }
    if !lock::is_dashboard_process(found.pid) {
        lock::clear_lock(&paths.lock);
        return Ok(
            "usage dashboard: nothing is running (removed a lock whose pid is not the dashboard server)"
                .to_string(),
        );
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

fn header_value(request: &tiny_http::Request, name: &'static str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str().to_string())
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
    let token = Token::generate()?;
    let me = Lock {
        pid: std::process::id(),
        port,
        token,
    };
    if !lock::try_claim(&paths.lock, me) {
        return Ok(());
    }
    for request in server.incoming_requests() {
        let host = header_value(&request, "Host");
        let fetch_site = header_value(&request, "Sec-Fetch-Site");
        let token_ok =
            header_value(&request, "X-Playbook-Token").is_some_and(|given| token.matches(&given));
        let path = request.url().split('?').next().unwrap_or("/").to_string();
        let (status, content_type, body) = if !host_allowed(host.as_deref(), port) {
            (
                403,
                "text/plain; charset=utf-8",
                "forbidden host".to_string(),
            )
        } else if path == "/api/data" && !fetch_site_allowed(fetch_site.as_deref()) {
            (
                403,
                "text/plain; charset=utf-8",
                "forbidden origin".to_string(),
            )
        } else {
            respond_to(&path, token_ok, || load_data(paths))
        };
        let mut response = tiny_http::Response::from_string(body).with_status_code(status);
        let mut headers = vec![
            ("Content-Type", content_type),
            ("Cache-Control", "no-store"),
            ("X-Content-Type-Options", "nosniff"),
        ];
        if status == 200 && path != "/api/data" {
            headers.push(("Content-Security-Policy", page::CONTENT_SECURITY_POLICY));
        }
        for (name, value) in headers {
            if let Ok(header) = tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()) {
                response.add_header(header);
            }
        }
        let _ = request.respond(response);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok() -> Token {
        Token::parse(&"ab".repeat(32)).expect("64 hex characters")
    }

    #[test]
    fn urls_carry_the_token_in_the_fragment_only() {
        let hex = "ab".repeat(32);

        assert_eq!(
            ip_url(4242, &tok()),
            format!("http://127.0.0.1:4242/#{hex}")
        );
        assert_eq!(
            alias_url(4242, &tok()),
            format!("http://playbook.localhost:4242/#{hex}")
        );
    }

    #[test]
    fn root_serves_the_page_data_runs_the_loader_and_anything_else_is_a_404() {
        let (status, content_type, body) =
            respond_to("/", false, || panic!("the page must not load data"));
        assert_eq!(status, 200);
        assert!(content_type.starts_with("text/html"));
        assert!(body.contains("/app.js"));

        let (status, content_type, body) =
            respond_to("/api/data", true, || Ok("{\"ok\":true}".to_string()));
        assert_eq!(
            (status, content_type, body.as_str()),
            (200, "application/json", "{\"ok\":true}")
        );

        let (status, _, body) = respond_to("/api/data", true, || Err("db locked".to_string()));
        assert_eq!(status, 500);
        assert!(body.contains("db locked"));

        assert_eq!(
            respond_to("/nope", true, || panic!("unknown paths load nothing")).0,
            404
        );
    }

    #[test]
    fn only_same_origin_or_unlabelled_requests_may_fetch_the_data() {
        assert!(fetch_site_allowed(None));
        assert!(fetch_site_allowed(Some("same-origin")));
        assert!(fetch_site_allowed(Some("none")));
        assert!(!fetch_site_allowed(Some("cross-site")));
        assert!(!fetch_site_allowed(Some("same-site")));
        assert!(!fetch_site_allowed(Some("")));
    }

    #[cfg(unix)]
    #[test]
    fn stop_never_signals_a_live_process_that_is_not_the_dashboard() {
        use super::super::lock::SOCKET_LOCK;
        use std::process::Command;

        let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Arrange: a lock naming a live, unrelated process and a listening
        // port, which is exactly what a reused pid looks like.
        let home = crate::common::test_support::scratch_dir("dash-stop-foreign");
        let paths = Paths::from_home(&home);
        std::fs::create_dir_all(paths.lock.parent().unwrap()).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut bystander = Command::new("sleep")
            .arg("5")
            .spawn()
            .expect("sleep starts");
        let port = listener.local_addr().unwrap().port();
        std::fs::write(
            &paths.lock,
            format!("{} {port} {}\n", bystander.id(), tok().as_str()),
        )
        .unwrap();

        // Act
        let message = run_stop(&paths).expect("stop reports, it does not fail");

        // Assert
        let still_running = bystander.try_wait().expect("wait works").is_none();
        let _ = bystander.kill();
        let _ = bystander.wait();
        assert!(
            still_running,
            "an unrelated process must never be signalled"
        );
        assert!(message.contains("not the dashboard server"), "{message}");
        assert!(!paths.lock.exists(), "the bogus lock must be removed");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn only_loopback_hosts_for_this_port_are_allowed() {
        assert!(host_allowed(Some("127.0.0.1:4242"), 4242));
        assert!(host_allowed(Some("localhost:4242"), 4242));
        assert!(host_allowed(Some("playbook.localhost:4242"), 4242));
        assert!(!host_allowed(Some("evil.example:4242"), 4242));
        assert!(!host_allowed(Some("127.0.0.1:9"), 4242));
        assert!(!host_allowed(Some("127.0.0.1"), 4242));
        assert!(!host_allowed(None, 4242));
    }

    #[test]
    fn the_data_route_needs_the_token_and_the_assets_do_not() {
        let never = || -> Result<String, String> { panic!("data must not load without the token") };

        let (status, _, body) = respond_to("/api/data", false, never);
        assert_eq!((status, body.as_str()), (401, "unauthorized"));

        for (path, kind) in [
            ("/", "text/html"),
            ("/app.js", "text/javascript"),
            ("/app.css", "text/css"),
        ] {
            let (status, content_type, body) = respond_to(path, false, never);
            assert_eq!(status, 200, "{path}");
            assert!(content_type.starts_with(kind), "{path}: {content_type}");
            assert!(
                !body.contains(tok().as_str()),
                "{path} must not carry the token"
            );
        }
    }

    #[test]
    fn the_page_assets_hold_no_usage_data() {
        for path in ["/", "/app.js", "/app.css"] {
            let (_, _, body) = respond_to(path, false, || panic!("no data"));
            for leaked in ["dev@example.com", "\"cost_usd\":", "\"messages\":"] {
                assert!(!body.contains(leaked), "{path} has {leaked}");
            }
        }
    }
}
