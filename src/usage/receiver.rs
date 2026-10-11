// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A small local OTLP/HTTP receiver for Claude Code telemetry.
//!
//! Hand rolled on `std::net` so no HTTP crate is added. It binds 127.0.0.1
//! only, handles one connection at a time with hard limits (header size, body
//! size, content type, request rate, read deadline), requires a bearer token
//! when one is set, and answers `Connection: close` to every request. Only the
//! JSON protocol is supported. `/v1/logs` is parsed by `otlp::parse_logs`
//! (the attribute allowlist) and stored; `/v1/metrics` and `/v1/traces` are
//! accepted and dropped.

use super::otlp;
use super::telemetry_db::{insert_error, insert_request, insert_tool};
use rusqlite::Connection;
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Largest header block read.
pub const MAX_HEADER_BYTES: usize = 16 * 1024;
/// Largest body accepted, plain or after chunk decoding.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
/// Requests allowed in one second; more get 429.
pub const MAX_REQUESTS_PER_SEC: u32 = 50;
/// Deadline for reading one whole request.
pub const READ_DEADLINE: Duration = Duration::from_secs(5);

/// How the receiver behaves.
pub struct Config {
    /// Bearer token the client must send, or `None` for no check.
    pub token: Option<String>,
    /// Stop after this long with no request, or `None` to run until stopped.
    pub idle: Option<Duration>,
}

/// A parsed request: the path (no query) and the body.
#[derive(Debug)]
struct Request {
    path: String,
    body: Vec<u8>,
}

type Headers = [(String, String)];
type Check<'a> = dyn Fn(&str, &str, &Headers) -> Result<(), HttpError> + 'a;

/// A failure that has an HTTP answer.
#[derive(Debug, PartialEq)]
struct HttpError(u16, &'static str);

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn read_more(
    stream: &mut impl Read,
    buf: &mut Vec<u8>,
    deadline: Instant,
) -> Result<(), HttpError> {
    if Instant::now() >= deadline {
        return Err(HttpError(408, "request timeout"));
    }
    let mut chunk = [0u8; 4096];
    match stream.read(&mut chunk) {
        Ok(0) => Err(HttpError(400, "connection closed early")),
        Ok(n) => {
            buf.extend_from_slice(&chunk[..n]);
            Ok(())
        }
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            Err(HttpError(408, "request timeout"))
        }
        Err(_) => Err(HttpError(400, "read failed")),
    }
}

/// Decodes a chunked body from `buf` (bytes after the headers), reading more
/// as needed, never holding more than `MAX_BODY_BYTES`.
fn read_chunked(
    stream: &mut impl Read,
    mut buf: Vec<u8>,
    deadline: Instant,
) -> Result<Vec<u8>, HttpError> {
    let mut body = Vec::new();
    loop {
        let line_end = loop {
            if let Some(i) = find(&buf, b"\r\n") {
                break i;
            }
            if buf.len() > 64 {
                return Err(HttpError(400, "bad chunk size"));
            }
            read_more(stream, &mut buf, deadline)?;
        };
        let size_text = std::str::from_utf8(&buf[..line_end])
            .map_err(|_| HttpError(400, "bad chunk size"))?
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        let size =
            usize::from_str_radix(size_text, 16).map_err(|_| HttpError(400, "bad chunk size"))?;
        buf.drain(..line_end + 2);
        if size == 0 {
            return Ok(body);
        }
        if body.len() + size > MAX_BODY_BYTES {
            return Err(HttpError(413, "body too large"));
        }
        while buf.len() < size + 2 {
            read_more(stream, &mut buf, deadline)?;
        }
        body.extend_from_slice(&buf[..size]);
        if &buf[size..size + 2] != b"\r\n" {
            return Err(HttpError(400, "bad chunk"));
        }
        buf.drain(..size + 2);
    }
}

/// Reads one request, enforcing the limits. `check` sees the method, path and
/// headers before the body is read, so a rejected request costs no body read.
fn read_request(stream: &mut impl Read, check: &Check) -> Result<Request, HttpError> {
    let deadline = Instant::now() + READ_DEADLINE;
    let mut buf = Vec::new();
    let head_end = loop {
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEADER_BYTES {
            return Err(HttpError(431, "headers too large"));
        }
        read_more(stream, &mut buf, deadline)?;
    };
    if head_end > MAX_HEADER_BYTES {
        return Err(HttpError(431, "headers too large"));
    }
    let head = std::str::from_utf8(&buf[..head_end]).map_err(|_| HttpError(400, "bad headers"))?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (Some(method), Some(path), Some(version), None) =
        (first.next(), first.next(), first.next(), first.next())
    else {
        return Err(HttpError(400, "bad request line"));
    };
    if !version.starts_with("HTTP/1.") {
        return Err(HttpError(400, "unsupported version"));
    }
    let mut headers = Vec::new();
    for line in lines {
        let (k, v) = line.split_once(':').ok_or(HttpError(400, "bad header"))?;
        headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
    }
    check(method, path, &headers)?;
    let get = |n: &str| {
        headers
            .iter()
            .find(|(k, _)| k == n)
            .map(|(_, v)| v.as_str())
    };
    let path = path.to_string();
    let rest = buf.split_off(head_end + 4);
    let body = match (get("content-length"), get("transfer-encoding")) {
        (Some(_), Some(_)) => return Err(HttpError(400, "ambiguous body length")),
        (None, Some(te)) if te.eq_ignore_ascii_case("chunked") => {
            read_chunked(stream, rest, deadline)?
        }
        (None, Some(_)) => return Err(HttpError(501, "unsupported transfer encoding")),
        (Some(len), None) => {
            let len: usize = len
                .parse()
                .map_err(|_| HttpError(400, "bad content length"))?;
            if len > MAX_BODY_BYTES {
                return Err(HttpError(413, "body too large"));
            }
            let mut body = rest;
            while body.len() < len {
                read_more(stream, &mut body, deadline)?;
            }
            body.truncate(len);
            body
        }
        (None, None) => return Err(HttpError(411, "length required")),
    };
    Ok(Request {
        path: path.split('?').next().unwrap_or("").to_string(),
        body,
    })
}

fn tokens_equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Method, path, auth, content type and encoding, checked before the body.
fn precheck(
    cfg: &Config,
    method: &str,
    path: &str,
    headers: &[(String, String)],
) -> Result<(), HttpError> {
    let get = |n: &str| {
        headers
            .iter()
            .find(|(k, _)| k == n)
            .map(|(_, v)| v.as_str())
    };
    let path = path.split('?').next().unwrap_or("");
    if !matches!(path, "/v1/logs" | "/v1/metrics" | "/v1/traces") {
        return Err(HttpError(404, "not found"));
    }
    if method != "POST" {
        return Err(HttpError(405, "method not allowed"));
    }
    if let Some(token) = &cfg.token {
        let ok = get("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
            .is_some_and(|t| tokens_equal(t.as_bytes(), token.as_bytes()));
        if !ok {
            return Err(HttpError(401, "unauthorized"));
        }
    }
    let json = get("content-type").is_some_and(|v| {
        v.split(';')
            .next()
            .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
    });
    if !json {
        return Err(HttpError(415, "content type must be application/json"));
    }
    if get("content-encoding").is_some_and(|v| !v.eq_ignore_ascii_case("identity")) {
        return Err(HttpError(415, "content encoding not supported"));
    }
    Ok(())
}

/// Stores a request's rows. Returns the status and body to send.
fn dispatch(conn: &Connection, req: &Request, now: i64) -> (u16, String) {
    if req.path != "/v1/logs" {
        return (200, "{}".to_string());
    }
    let Ok(rows) = otlp::parse_events(&req.body, now) else {
        return (400, r#"{"error":"invalid OTLP JSON"}"#.to_string());
    };
    let stored = conn.unchecked_transaction().and_then(|tx| {
        let bad = |_| rusqlite::Error::InvalidQuery;
        for row in &rows.requests {
            insert_request(&tx, row).map_err(bad)?;
        }
        for row in &rows.tools {
            insert_tool(&tx, row).map_err(bad)?;
        }
        for row in &rows.errors {
            insert_error(&tx, row).map_err(bad)?;
        }
        tx.commit()
    });
    match stored {
        Ok(()) => (200, r#"{"partialSuccess":{}}"#.to_string()),
        Err(_) => (503, r#"{"error":"store unavailable"}"#.to_string()),
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        501 => "Not Implemented",
        _ => "Service Unavailable",
    }
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let text = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        reason(status),
        body.len()
    );
    let _ = stream.write_all(text.as_bytes());
}

/// Counts requests in the current second.
struct RateLimiter {
    window: Instant,
    count: u32,
}

impl RateLimiter {
    fn allow(&mut self, now: Instant) -> bool {
        if now.duration_since(self.window) >= Duration::from_secs(1) {
            self.window = now;
            self.count = 0;
        }
        self.count += 1;
        self.count <= MAX_REQUESTS_PER_SEC
    }
}

/// Binds 127.0.0.1 on `port` (0 picks a free one).
pub fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
}

/// Serves until `stop` is set or the idle limit passes. Returns why it ended.
pub fn serve(
    listener: &TcpListener,
    conn: &Connection,
    cfg: &Config,
    stop: &Arc<AtomicBool>,
    now: impl Fn() -> i64,
) -> std::io::Result<&'static str> {
    listener.set_nonblocking(true)?;
    let mut last = Instant::now();
    let mut limiter = RateLimiter {
        window: Instant::now(),
        count: 0,
    };
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok("stopped");
        }
        if cfg.idle.is_some_and(|idle| last.elapsed() >= idle) {
            return Ok("idle");
        }
        let mut stream = match listener.accept() {
            Ok((s, _)) => s,
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
                continue;
            }
            Err(e) => return Err(e),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(READ_DEADLINE));
        let _ = stream.set_write_timeout(Some(READ_DEADLINE));
        if !limiter.allow(Instant::now()) {
            respond(&mut stream, 429, r#"{"error":"rate limit"}"#);
            continue;
        }
        let check = |m: &str, p: &str, h: &[(String, String)]| precheck(cfg, m, p, h);
        match read_request(&mut stream, &check) {
            Ok(req) => {
                // Only a real request keeps the collector alive. A bare
                // connect (a liveness probe from `cc` or `usage status`) or a
                // rejected request does not, so polling cannot hold it open.
                last = Instant::now();
                let (status, body) = dispatch(conn, &req, now());
                respond(&mut stream, status, &body);
            }
            Err(HttpError(status, msg)) => {
                respond(&mut stream, status, &format!(r#"{{"error":"{msg}"}}"#));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::telemetry_db::{count_requests, migrate_up};
    use std::io::Cursor;

    fn raw(method_path: &str, headers: &[&str], body: &[u8]) -> Vec<u8> {
        let mut v = format!("{method_path} HTTP/1.1\r\n").into_bytes();
        for h in headers {
            v.extend_from_slice(format!("{h}\r\n").as_bytes());
        }
        v.extend_from_slice(b"\r\n");
        v.extend_from_slice(body);
        v
    }

    fn cfg(token: Option<&str>) -> Config {
        Config {
            token: token.map(str::to_string),
            idle: None,
        }
    }

    fn parse(bytes: Vec<u8>, c: &Config) -> Result<Request, HttpError> {
        let check = |m: &str, p: &str, h: &[(String, String)]| precheck(c, m, p, h);
        read_request(&mut Cursor::new(bytes), &check)
    }

    const JSON: &str = "Content-Type: application/json";

    #[test]
    fn a_plain_json_post_is_read() {
        let req = parse(
            raw("POST /v1/logs", &[JSON, "Content-Length: 2"], b"{}"),
            &cfg(None),
        )
        .unwrap();
        assert_eq!(req.path, "/v1/logs");
        assert_eq!(req.body, b"{}");
    }

    #[test]
    fn a_chunked_body_is_decoded() {
        let req = parse(
            raw(
                "POST /v1/logs",
                &[JSON, "Transfer-Encoding: chunked"],
                b"2\r\n{\"\r\n2;x=1\r\na\"\r\n0\r\n\r\n",
            ),
            &cfg(None),
        )
        .unwrap();
        assert_eq!(req.body, b"{\"a\"");
    }

    #[test]
    fn a_body_over_the_limit_is_413_before_it_is_read() {
        let len = format!("Content-Length: {}", MAX_BODY_BYTES + 1);
        let err = parse(raw("POST /v1/logs", &[JSON, &len], b""), &cfg(None)).unwrap_err();
        assert_eq!(err.0, 413);
    }

    #[test]
    fn a_chunked_body_over_the_limit_is_413() {
        let big = format!("{:x}\r\n", MAX_BODY_BYTES + 1);
        let err = parse(
            raw(
                "POST /v1/logs",
                &[JSON, "Transfer-Encoding: chunked"],
                big.as_bytes(),
            ),
            &cfg(None),
        )
        .unwrap_err();
        assert_eq!(err.0, 413);
    }

    #[test]
    fn oversized_headers_are_431() {
        let long = format!("X-Pad: {}", "a".repeat(MAX_HEADER_BYTES + 10));
        let err = parse(raw("POST /v1/logs", &[JSON, &long], b""), &cfg(None)).unwrap_err();
        assert_eq!(err.0, 431);
    }

    #[test]
    fn wrong_path_method_type_encoding_and_token_are_refused() {
        let c = cfg(Some("secret"));
        let ok_auth = "Authorization: Bearer secret";
        let case = |req: Vec<u8>| parse(req, &c).unwrap_err().0;
        assert_eq!(
            case(raw(
                "POST /other",
                &[JSON, ok_auth, "Content-Length: 0"],
                b""
            )),
            404
        );
        assert_eq!(case(raw("GET /v1/logs", &[JSON, ok_auth], b"")), 405);
        assert_eq!(
            case(raw("POST /v1/logs", &[JSON, "Content-Length: 0"], b"")),
            401
        );
        assert_eq!(
            case(raw(
                "POST /v1/logs",
                &[JSON, "Authorization: Bearer nope", "Content-Length: 0"],
                b""
            )),
            401
        );
        assert_eq!(
            case(raw(
                "POST /v1/logs",
                &["Content-Type: text/plain", ok_auth, "Content-Length: 0"],
                b""
            )),
            415
        );
        assert_eq!(
            case(raw(
                "POST /v1/logs",
                &[JSON, ok_auth, "Content-Encoding: gzip", "Content-Length: 0"],
                b""
            )),
            415
        );
        assert_eq!(case(raw("POST /v1/logs", &[JSON, ok_auth], b"")), 411);
    }

    #[test]
    fn a_content_type_with_a_charset_is_accepted() {
        let req = raw(
            "POST /v1/logs",
            &[
                "Content-Type: application/json; charset=utf-8",
                "Content-Length: 0",
            ],
            b"",
        );
        assert!(parse(req, &cfg(None)).is_ok());
    }

    #[test]
    fn length_and_chunked_together_are_refused() {
        let req = raw(
            "POST /v1/logs",
            &[JSON, "Content-Length: 1", "Transfer-Encoding: chunked"],
            b"",
        );
        assert_eq!(parse(req, &cfg(None)).unwrap_err().0, 400);
    }

    #[test]
    fn the_rate_limiter_cuts_off_then_resets() {
        let t = Instant::now();
        let mut l = RateLimiter {
            window: t,
            count: 0,
        };
        for _ in 0..MAX_REQUESTS_PER_SEC {
            assert!(l.allow(t));
        }
        assert!(!l.allow(t));
        assert!(l.allow(t + Duration::from_millis(1001)));
    }

    // End to end over a real loopback socket.

    fn post(port: u16, path: &str, extra: &[&str], body: &[u8]) -> (u16, String) {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        let len = format!("Content-Length: {}", body.len());
        let mut headers = vec![JSON, len.as_str()];
        headers.extend_from_slice(extra);
        s.write_all(&raw(&format!("POST {path}"), &headers, body))
            .unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        let status = out
            .split(' ')
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        (status, out)
    }

    fn logs_body(request_id: &str) -> Vec<u8> {
        serde_json::json!({"resourceLogs":[{"scopeLogs":[{"logRecords":[{
        "attributes":[
            {"key":"event.name","value":{"stringValue":"api_request"}},
            {"key":"session.id","value":{"stringValue":"s1"}},
            {"key":"request_id","value":{"stringValue":request_id}},
            {"key":"prompt","value":{"stringValue":"top secret"}},
            {"key":"cost_usd","value":{"doubleValue":0.5}}
        ]}]}]}]})
        .to_string()
        .into_bytes()
    }

    #[test]
    fn the_server_stores_rows_dedupes_and_stops_on_idle() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_up(&conn).unwrap();
        let listener = bind(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(listener.local_addr().unwrap().ip().is_loopback());
        let c = Config {
            token: Some("tok".into()),
            idle: Some(Duration::from_millis(1500)),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let client = std::thread::spawn(move || {
            let auth = ["Authorization: Bearer tok"];
            let a = post(port, "/v1/logs", &auth, &logs_body("r1"));
            let b = post(port, "/v1/logs", &auth, &logs_body("r1"));
            let c = post(
                port,
                "/v1/logs",
                &["Authorization: Bearer bad"],
                &logs_body("r2"),
            );
            let d = post(port, "/v1/logs", &auth, b"nope");
            let e = post(port, "/v1/metrics", &auth, b"{}");
            [a.0, b.0, c.0, d.0, e.0]
        });
        let why = serve(&listener, &conn, &c, &stop, || 1).unwrap();
        assert_eq!(client.join().unwrap(), [200, 200, 401, 400, 200]);
        assert_eq!(why, "idle");
        assert_eq!(count_requests(&conn).unwrap(), 1);
        let dump: String = conn
            .query_row(
                "SELECT group_concat(model||effort||agent_name) FROM telemetry_requests",
                [],
                |r| r.get::<_, Option<String>>(0),
            )
            .unwrap()
            .unwrap_or_default();
        assert!(!dump.contains("secret"));
    }

    #[test]
    fn the_stop_flag_ends_the_server() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_up(&conn).unwrap();
        let listener = bind(0).unwrap();
        let stop = Arc::new(AtomicBool::new(true));
        assert_eq!(
            serve(&listener, &conn, &cfg(None), &stop, || 1).unwrap(),
            "stopped"
        );
    }

    #[test]
    fn tool_results_and_api_errors_are_stored_without_their_content() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_up(&conn).unwrap();
        let body = serde_json::json!({"resourceLogs":[{"scopeLogs":[{"logRecords":[
            {"timeUnixNano":"1760000000000000000","attributes":[
                {"key":"event.name","value":{"stringValue":"tool_result"}},
                {"key":"session.id","value":{"stringValue":"s1"}},
                {"key":"tool_use_id","value":{"stringValue":"t1"}},
                {"key":"tool_name","value":{"stringValue":"Bash"}},
                {"key":"success","value":{"stringValue":"false"}},
                {"key":"duration_ms","value":{"stringValue":"42"}},
                {"key":"tool_input","value":{"stringValue":"rm -rf secret"}}]},
            {"timeUnixNano":"1760000000000000000","attributes":[
                {"key":"event.name","value":{"stringValue":"api_error"}},
                {"key":"session.id","value":{"stringValue":"s1"}},
                {"key":"event.sequence","value":{"intValue":"7"}},
                {"key":"attempt","value":{"intValue":"2"}},
                {"key":"error","value":{"stringValue":"secret failure"}}]}
        ]}]}]})
        .to_string();
        let req = Request {
            path: "/v1/logs".into(),
            body: body.into_bytes(),
        };
        for _ in 0..2 {
            assert_eq!(dispatch(&conn, &req, 5).0, 200);
        }
        let h = crate::usage::telemetry_db::health(&conn, 0, i64::MAX).unwrap();
        assert_eq!(
            (h.tool_calls, h.tool_failures, h.errors, h.retried),
            (1, 1, 1, 1)
        );
        let dump: String = conn
            .query_row(
                "SELECT (SELECT group_concat(tool_name) FROM telemetry_tools)
                      || (SELECT group_concat(model) FROM telemetry_errors)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !dump.contains("secret") && !dump.contains("rm -rf"),
            "{dump}"
        );
    }

    #[test]
    fn a_probe_that_sends_nothing_does_not_keep_the_server_alive() {
        let conn = Connection::open_in_memory().unwrap();
        migrate_up(&conn).unwrap();
        let listener = bind(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        let c = Config {
            token: None,
            idle: Some(Duration::from_millis(1200)),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let flag = done.clone();
        let prober = std::thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                let _ = std::net::TcpStream::connect(("127.0.0.1", port));
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let started = Instant::now();
        let why = serve(&listener, &conn, &c, &stop, || 1).unwrap();
        done.store(true, Ordering::SeqCst);
        prober.join().unwrap();
        assert_eq!(why, "idle");
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
