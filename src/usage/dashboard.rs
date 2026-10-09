// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The local dashboard server. `playbook usage dashboard` is a short-lived
//! process: it reuses a live server or starts a detached one, opens the
//! browser, and exits. The server itself is a second process (the same binary
//! with a hidden flag) that outlives its parent, so no other `playbook`
//! command ever waits on it.

use super::api::Range;
use super::live;
use super::lock::{self, Lock, Token};
use super::page;
use super::run::Paths;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long a starter waits for the new server to write its own lock.
const LOCK_READY_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

pub type Opener<'a> = &'a dyn Fn(&str) -> Result<(), String>;

/// The link the browser opens. The token rides in the fragment, which a
/// browser never sends to the server, so it stays out of requests and logs.
fn ip_url(port: u16, token: &Token) -> String {
    format!("http://127.0.0.1:{port}/#{}", token.as_str())
}

/// `*.localhost` resolves to loopback on macOS and most Linux resolvers but
/// not all, so it is only ever printed, never used to open the browser.
fn alias_url(port: u16, token: &Token) -> String {
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
fn host_allowed(host: Option<&str>, port: u16) -> bool {
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
fn fetch_site_allowed(value: Option<&str>) -> bool {
    matches!(value, None | Some("same-origin") | Some("none"))
}

/// The value of `name` in a query string, taken verbatim.
fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

/// A guarded data request, parsed and validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Api {
    Data(Range),
    Sessions(Range),
    Session(String),
    Live,
}

/// Why a loader had nothing to return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    NotFound,
    Internal(String),
}

const TEXT: &str = "text/plain; charset=utf-8";

/// Every path that returns usage data. They all share one guard: the Host
/// allowlist, the `Sec-Fetch-Site` check and the session token.
fn is_api(path: &str) -> bool {
    path.starts_with("/api/")
}

fn parse_api(path: &str, query: &str) -> Result<Api, (u16, &'static str, String)> {
    let range = || {
        Range::parse(query_param(query, "range")).ok_or((400, TEXT, "unknown range".to_string()))
    };
    match path {
        "/api/data" => range().map(Api::Data),
        "/api/sessions" => range().map(Api::Sessions),
        "/api/live" => Ok(Api::Live),
        "/api/session" => match query_param(query, "id") {
            Some(id) if super::api::valid_session_id(id) => Ok(Api::Session(id.to_string())),
            _ => Err((400, TEXT, "invalid session id".to_string())),
        },
        _ => Err((404, TEXT, "not found".to_string())),
    }
}

/// What the server answers for a path: (status, content type, body). The page
/// assets carry no data and need no token. `load` runs only for a known
/// `/api/` path, only when `token_ok`, and only with a validated request.
fn respond_to(
    path: &str,
    query: &str,
    token_ok: bool,
    load: impl FnOnce(Api) -> Result<String, Failure>,
) -> (u16, &'static str, String) {
    match path {
        "/" => (200, "text/html; charset=utf-8", page::HTML.to_string()),
        "/app.js" => (200, "text/javascript; charset=utf-8", page::JS.to_string()),
        "/app.css" => (200, "text/css; charset=utf-8", page::CSS.to_string()),
        _ if is_api(path) => {
            let request = match parse_api(path, query) {
                Err((404, ..)) => return (404, TEXT, "not found".to_string()),
                _ if !token_ok => return (401, TEXT, "unauthorized".to_string()),
                Err(refusal) => return refusal,
                Ok(request) => request,
            };
            let content_type = if request == Api::Live {
                super::live::EVENT_STREAM
            } else {
                "application/json"
            };
            match load(request) {
                Ok(body) => (200, content_type, body),
                Err(Failure::NotFound) => (
                    404,
                    "application/json",
                    serde_json::json!({ "error": "unknown session" }).to_string(),
                ),
                Err(Failure::Internal(e)) => (
                    500,
                    "application/json",
                    serde_json::json!({ "error": e }).to_string(),
                ),
            }
        }
        _ => (404, TEXT, "not found".to_string()),
    }
}

/// Ingest anything new, then build the JSON for one guarded request.
fn load(paths: &Paths, request: Api) -> Result<String, Failure> {
    if request == Api::Live {
        // Only the guard runs here; the stream computes its own summaries.
        return Ok(String::new());
    }
    let (conn, _) = super::run::ingest_new(paths).map_err(Failure::Internal)?;
    render(&conn, request, crate::common::time::now_secs())
}

/// The events a ranged response is built from. `data_json` and `sessions_json`
/// both drop everything before the first day of the range, so all time is the
/// only range that needs every row.
fn load_window(
    conn: &rusqlite::Connection,
    range: Range,
    now: i64,
) -> Result<Vec<super::UsageEvent>, String> {
    match range.days(now) {
        Some((first_day, _)) => super::db::load_usage_events_since(
            conn,
            first_day.saturating_mul(super::aggregate::SECONDS_PER_DAY),
        ),
        None => super::db::load_usage_events(conn),
    }
}

/// Build the JSON for one data request from what the store holds at `now`.
fn render(conn: &rusqlite::Connection, request: Api, now: i64) -> Result<String, Failure> {
    use super::db;
    let internal = Failure::Internal;
    let value = match request {
        Api::Data(range) => {
            let usage = load_window(conn, range, now).map_err(internal)?;
            let tools = db::load_tool_events(conn).map_err(Failure::Internal)?;
            super::api::data_json(&usage, &tools, now, range)
        }
        Api::Sessions(range) => {
            let usage = load_window(conn, range, now).map_err(internal)?;
            super::api::sessions_json(&usage, now, range)
        }
        Api::Session(id) => {
            let usage = db::load_usage_events_of_sessions(conn, std::slice::from_ref(&id))
                .map_err(internal)?;
            super::api::session_json(&usage, &id).ok_or(Failure::NotFound)?
        }
        Api::Live => return Ok(String::new()),
    };
    serde_json::to_string(&value)
        .map_err(|e| Failure::Internal(format!("failed to encode usage data: {e}")))
}

/// Ingest anything new, then build only the live summary: the last hour and
/// today, the active sessions and the newest messages, not the whole dataset.
fn live_summary(paths: &Paths) -> Result<String, String> {
    use super::{api, db};
    let (conn, _) = super::run::ingest_new(paths)?;
    let now = crate::common::time::now_secs();
    let window = db::load_usage_events_since(&conn, api::live_window_start(now))?;
    let ids = api::active_session_ids(&window, now);
    let sessions = db::load_usage_events_of_sessions(&conn, &ids)?;
    let latest = db::load_latest_usage_events(&conn, api::FEED_LIMIT as i64)?;
    serde_json::to_string(&api::live_json(&window, &sessions, &latest, now))
        .map_err(|e| format!("failed to encode the live summary: {e}"))
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
    let live_state = live::Live::new();
    let timing = live::Timing::from_env();
    for request in server.incoming_requests() {
        let host = header_value(&request, "Host");
        let fetch_site = header_value(&request, "Sec-Fetch-Site");
        let token_ok =
            header_value(&request, "X-Playbook-Token").is_some_and(|given| token.matches(&given));
        let url = request.url().to_string();
        let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
        let path = path.to_string();
        let (status, content_type, body) = if !host_allowed(host.as_deref(), port) {
            (
                403,
                "text/plain; charset=utf-8",
                "forbidden host".to_string(),
            )
        } else if is_api(&path) && !fetch_site_allowed(fetch_site.as_deref()) {
            (
                403,
                "text/plain; charset=utf-8",
                "forbidden origin".to_string(),
            )
        } else {
            respond_to(&path, query, token_ok, |api| load(paths, api))
        };
        if status == 200 && content_type == live::EVENT_STREAM {
            // The slot is taken on the stream's own thread, which may wait a
            // moment for a client that just left, so the accept loop never blocks.
            let (state, owned) = (std::sync::Arc::clone(&live_state), paths.clone());
            // If the thread cannot start, the request drops with the closure.
            let _ = std::thread::Builder::new()
                .name("usage-live".to_string())
                .spawn(move || {
                    let Some(slot) = state.acquire_within(live::MAX_STREAMS, timing.reopen_grace)
                    else {
                        let mut refused = tiny_http::Response::from_string(
                            "too many live streams are open".to_string(),
                        )
                        .with_status_code(503);
                        for (name, value) in [
                            ("Content-Type", "text/plain; charset=utf-8"),
                            ("Cache-Control", "no-store"),
                            ("X-Content-Type-Options", "nosniff"),
                        ] {
                            if let Ok(header) =
                                tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes())
                            {
                                refused.add_header(header);
                            }
                        }
                        let _ = request.respond(refused);
                        return;
                    };
                    let _slot = slot;
                    let stuck = state.stuck();
                    let next = || state.summary(|| live_summary(&owned));
                    let writer = request.into_writer();
                    let _ = live::stream_guarded(writer, next, timing, stuck);
                });
            continue;
        }
        let mut response = tiny_http::Response::from_string(body).with_status_code(status);
        let mut headers = vec![
            ("Content-Type", content_type),
            ("Cache-Control", "no-store"),
            ("X-Content-Type-Options", "nosniff"),
        ];
        if status == 200 && !is_api(&path) {
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
    use crate::usage::UsageEvent;

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
            respond_to("/", "", false, |_| panic!("the page must not load data"));
        assert_eq!(status, 200);
        assert!(content_type.starts_with("text/html"));
        assert!(body.contains("/app.js"));

        let (status, content_type, body) =
            respond_to("/api/data", "", true, |_| Ok("{\"ok\":true}".to_string()));
        assert_eq!(
            (status, content_type, body.as_str()),
            (200, "application/json", "{\"ok\":true}")
        );

        let (status, _, body) = respond_to("/api/data", "", true, |_| {
            Err(Failure::Internal("db locked".to_string()))
        });
        assert_eq!(status, 500);
        assert!(body.contains("db locked"));

        assert_eq!(
            respond_to("/nope", "", true, |_| panic!("unknown paths load nothing")).0,
            404
        );
    }

    #[test]
    fn every_range_button_is_accepted_by_the_server_and_echoed_back() {
        let list = page::JS
            .split("const RANGES = [")
            .nth(1)
            .and_then(|rest| rest.split("];").next())
            .expect("the page declares RANGES");
        let keys: Vec<&str> = list
            .split("],[")
            .map(|pair| pair.split('"').nth(1).expect("a quoted key"))
            .collect();

        assert_eq!(keys, ["30d", "60d", "90d", "month", "all"]);
        for key in keys {
            let range = Range::parse(Some(key)).unwrap_or_else(|| panic!("{key} is refused"));
            assert_eq!(
                super::super::api::data_json(&[], &[], 0, range)["range"]["key"],
                key
            );
        }
    }

    #[test]
    fn the_data_route_passes_the_range_and_refuses_an_unknown_one() {
        let seen = |query: &str| {
            let mut got = None;
            let status = respond_to("/api/data", query, true, |api| {
                if let Api::Data(range) = api {
                    got = Some(range);
                }
                Ok(String::new())
            })
            .0;
            (status, got)
        };

        assert_eq!(seen(""), (200, Some(Range::All)));
        assert_eq!(seen("range=30d"), (200, Some(Range::LastDays(30))));
        assert_eq!(seen("x=1&range=month"), (200, Some(Range::CurrentMonth)));
        assert_eq!(seen("range=7d"), (400, None));
        assert_eq!(seen("range=<script>"), (400, None));
    }

    #[test]
    fn the_session_routes_pass_validated_requests_and_refuse_the_rest() {
        let seen = |path: &str, query: &str| {
            let mut got = None;
            let status = respond_to(path, query, true, |api| {
                got = Some(api);
                Ok(String::new())
            })
            .0;
            (status, got)
        };

        assert_eq!(
            seen("/api/sessions", "range=90d"),
            (200, Some(Api::Sessions(Range::LastDays(90))))
        );
        assert_eq!(seen("/api/sessions", "range=7d"), (400, None));
        assert_eq!(
            seen("/api/session", "id=0199aaaa-0000-7000.x_Y"),
            (
                200,
                Some(Api::Session("0199aaaa-0000-7000.x_Y".to_string()))
            )
        );
        for bad in ["", "id=", "id=a%2Fb", "id=a/b", "id=a b", "id=<s>", "x=1"] {
            assert_eq!(seen("/api/session", bad), (400, None), "{bad}");
        }
        let long = format!("id={}", "a".repeat(129));
        assert_eq!(seen("/api/session", &long), (400, None));
        assert_eq!(seen("/api/nope", ""), (404, None));
        let live = respond_to("/api/live", "", true, |api| {
            assert_eq!(api, Api::Live);
            Ok(String::new())
        });
        assert_eq!((live.0, live.1), (200, live::EVENT_STREAM));
        let missing = respond_to("/api/session", "id=zz", true, |_| Err(Failure::NotFound));
        assert_eq!(missing.0, 404);
        assert_eq!(missing.1, "application/json");
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
        let never =
            |_: Api| -> Result<String, Failure> { panic!("data must not load without the token") };

        for (path, query) in [
            ("/api/data", "range=30d"),
            ("/api/sessions", "range=30d"),
            ("/api/session", "id=abc"),
            ("/api/session", "id=../x"),
            ("/api/live", ""),
            ("/api/data", "range=bogus"),
        ] {
            let (status, _, body) = respond_to(path, query, false, never);
            assert_eq!((status, body.as_str()), (401, "unauthorized"), "{path}");
        }

        for (path, kind) in [
            ("/", "text/html"),
            ("/app.js", "text/javascript"),
            ("/app.css", "text/css"),
        ] {
            let (status, content_type, body) = respond_to(path, "", false, never);
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
            let (_, _, body) = respond_to(path, "", false, |_| panic!("no data"));
            for leaked in ["dev@example.com", "\"cost_usd\":", "\"messages\":"] {
                assert!(!body.contains(leaked), "{path} has {leaked}");
            }
        }
    }

    const PINNED: &[u64] = &[
        17484291421679850613,
        3730294920132424157,
        1033622341910043654,
        10862299611591569038,
        17588408457794840280,
        2494723156773533385,
        11994428673198853629,
        2431074417907975038,
        4008528684616667504,
        3262773478835288402,
        12295639343689629795,
        17375345325142460155,
        11946032276010068899,
        4632317121851229859,
        17650650940913503287,
        7820942360683494649,
        1916967026913426392,
    ];
    const PINNED_EMPTY: &[u64] = &[
        10774984880922801611,
        50327236147303898,
        6045690372409743390,
        4881509650723624483,
        7216419771988443760,
        10397446657680869382,
    ];

    // 2026-10-08 12:00 UTC.
    const NOW: i64 = 1_791_460_800;
    const DAY: i64 = 86_400;

    fn usage_event(id: &str, session: &str, at: i64, model: &str, agent: &str) -> UsageEvent {
        UsageEvent {
            event_id: id.into(),
            timestamp: at,
            session_id: session.into(),
            account: "dev@example.com".into(),
            agent: agent.into(),
            model: model.into(),
            effort: "high".into(),
            repo: format!("acme/{agent}"),
            branch: format!("b-{session}"),
            input_tokens: 1000 + at as u64 % 7,
            output_tokens: 500,
            cache_creation_tokens: 200,
            cache_read_tokens: 3000,
            ..UsageEvent::default()
        }
    }

    /// Sessions: `old` (far past), `edge` (spans the 30 day boundary),
    /// `recent`, a Codex one, an unpriced model, an unusable id, a future one.
    fn fixture() -> rusqlite::Connection {
        let conn = empty_store();
        let window_start = (NOW / DAY - 29) * DAY;
        let events = [
            usage_event(
                "o1",
                "old",
                NOW - 200 * DAY,
                "claude-opus-4-1",
                "claude-code",
            ),
            usage_event(
                "o2",
                "old",
                NOW - 199 * DAY,
                "claude-opus-4-1",
                "claude-code",
            ),
            usage_event(
                "e1",
                "edge",
                window_start - 1,
                "claude-sonnet-4-5",
                "claude-code",
            ),
            usage_event(
                "e2",
                "edge",
                window_start,
                "claude-sonnet-4-5",
                "claude-code",
            ),
            usage_event(
                "e3",
                "edge",
                window_start + 60,
                "claude-haiku-4-5",
                "claude-code",
            ),
            usage_event(
                "r1",
                "recent",
                NOW - 3600,
                "claude-sonnet-4-5",
                "claude-code",
            ),
            usage_event("r2", "recent", NOW - 60, "claude-opus-4-1", "claude-code"),
            usage_event("c1", "codex-1", NOW - 5 * DAY, "gpt-5-codex", "codex"),
            usage_event("c2", "codex-1", NOW - 5 * DAY + 90, "gpt-5-codex", "codex"),
            usage_event(
                "u1",
                "mystery",
                NOW - 2 * DAY,
                "mystery-model",
                "claude-code",
            ),
            usage_event(
                "b1",
                "bad id!",
                NOW - DAY,
                "claude-sonnet-4-5",
                "claude-code",
            ),
            usage_event(
                "f1",
                "future",
                NOW + 2 * DAY,
                "claude-sonnet-4-5",
                "claude-code",
            ),
            usage_event(
                "m1",
                "monthly",
                (NOW / DAY - 7) * DAY,
                "claude-sonnet-4-5",
                "claude-code",
            ),
            usage_event(
                "m0",
                "monthly",
                (NOW / DAY - 8) * DAY - 1,
                "claude-sonnet-4-5",
                "claude-code",
            ),
        ];
        for e in &events {
            super::super::db::upsert_usage_event(&conn, e).unwrap();
        }
        for (id, session, at, kind, name) in [
            (
                "t1",
                "edge",
                NOW - DAY,
                super::super::ToolKind::Skill,
                "commit",
            ),
            (
                "t2",
                "recent",
                NOW - 60,
                super::super::ToolKind::Agent,
                "reviewer",
            ),
            (
                "t3",
                "old",
                NOW - 200 * DAY,
                super::super::ToolKind::Skill,
                "ancient",
            ),
        ] {
            let tool = super::super::ToolInvocationEvent {
                event_id: id.into(),
                timestamp: at,
                session_id: session.into(),
                account: "dev@example.com".into(),
                kind,
                name: name.into(),
            };
            super::super::db::insert_tool_event(&conn, &tool).unwrap();
        }
        conn
    }

    fn empty_store() -> rusqlite::Connection {
        let dir = crate::common::test_support::scratch_dir("usage-render");
        super::super::db::open_db(&dir.join("usage.db")).unwrap()
    }

    fn fnv(body: &str) -> u64 {
        body.bytes().fold(0xcbf29ce484222325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        })
    }

    fn pinned(request: Api) -> Result<u64, Failure> {
        render(&fixture(), request, NOW).map(|body| fnv(&body))
    }

    #[test]
    fn every_data_endpoint_keeps_its_exact_json_on_a_fixed_fixture() {
        let ranges = [
            Range::All,
            Range::LastDays(30),
            Range::LastDays(60),
            Range::LastDays(90),
            Range::CurrentMonth,
        ];
        let mut got = Vec::new();
        for range in ranges {
            got.push(pinned(Api::Data(range)).unwrap());
            got.push(pinned(Api::Sessions(range)).unwrap());
        }
        for id in [
            "old", "edge", "recent", "codex-1", "mystery", "monthly", "future",
        ] {
            got.push(pinned(Api::Session(id.to_string())).unwrap());
        }
        assert_eq!(got.as_slice(), PINNED);
        assert_eq!(pinned(Api::Session("nope".into())), Err(Failure::NotFound));
    }

    #[test]
    fn an_empty_store_renders_every_endpoint_without_error() {
        let conn = empty_store();
        let mut got = Vec::new();
        for range in [Range::All, Range::LastDays(30), Range::CurrentMonth] {
            got.push(fnv(&render(&conn, Api::Data(range), NOW).unwrap()));
            got.push(fnv(&render(&conn, Api::Sessions(range), NOW).unwrap()));
        }
        assert_eq!(got.as_slice(), PINNED_EMPTY);
        assert_eq!(
            render(&conn, Api::Session("x".into()), NOW),
            Err(Failure::NotFound)
        );
    }

    #[test]
    fn a_window_with_no_events_is_empty_but_well_formed() {
        let conn = fixture();
        let far = NOW + 400 * DAY;
        let data: serde_json::Value =
            serde_json::from_str(&render_at(&conn, Api::Data(Range::LastDays(30)), far)).unwrap();
        assert_eq!(data["totals"]["messages"], 0);
        let sessions: serde_json::Value =
            serde_json::from_str(&render_at(&conn, Api::Sessions(Range::LastDays(30)), far))
                .unwrap();
        assert_eq!(sessions["total"], 0);
    }

    fn render_at(conn: &rusqlite::Connection, request: Api, now: i64) -> String {
        render(conn, request, now).unwrap()
    }

    #[test]
    fn the_edge_session_keeps_its_whole_cost_in_its_own_view_but_only_the_window_in_the_list() {
        let conn = fixture();
        let one: serde_json::Value =
            serde_json::from_str(&render_at(&conn, Api::Session("edge".into()), NOW)).unwrap();
        assert_eq!(one["total"], 3);
        let list: serde_json::Value =
            serde_json::from_str(&render_at(&conn, Api::Sessions(Range::LastDays(30)), NOW))
                .unwrap();
        let edge = list["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "edge")
            .unwrap();
        assert_eq!(edge["messages"], 2);
    }
}
