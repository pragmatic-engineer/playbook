// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Spawned-binary tests for the dashboard's session token and script policy.
//! Scratch `$HOME`, no browser, one server at a time behind `SOCKET_LOCK`.

#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

// Declared per test binary: a static Mutex cannot span separate processes.
static SOCKET_LOCK: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("playbook-auth-{}-{tag}-{n}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Home(dir.canonicalize().unwrap())
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(args)
            .env("HOME", &self.0)
            .env("PLAYBOOK_USAGE_NO_BROWSER", "1")
            .output()
            .unwrap()
    }

    fn lock_path(&self) -> PathBuf {
        self.0.join(".config/playbook/usage/dashboard.lock")
    }

    fn lock_fields(&self) -> Vec<String> {
        fs::read_to_string(self.lock_path())
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect()
    }

    fn seed(&self) {
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/usage");
        let project = self.0.join(".claude/projects/proj-one");
        fs::create_dir_all(&project).unwrap();
        fs::copy(
            fixtures.join("usage/proj-one/s1.jsonl"),
            project.join("seeded.jsonl"),
        )
        .unwrap();
        fs::copy(
            fixtures.join("account/claude.json"),
            self.0.join(".claude.json"),
        )
        .unwrap();
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self.run(&["usage", "dashboard", "stop"]);
        if let Some(pid) = self.lock_fields().first() {
            let _ = Command::new("kill").args(["-9", pid]).output();
        }
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `(port, token)` from the first printed link `http://127.0.0.1:<port>/#<token>`.
fn link_in(text: &str) -> (u16, String) {
    let after = text.split("http://127.0.0.1:").nth(1).expect("an IP URL");
    let port = after
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let token = after
        .split("/#")
        .nth(1)
        .expect("a fragment")
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect();
    (port, token)
}

fn start(home: &Home) -> (u16, String) {
    link_in(&stdout(&home.run(&["usage", "dashboard"])))
}

fn get(port: u16, path: &str, token: Option<&str>) -> (u16, String) {
    let mut request = ureq::get(&format!("http://127.0.0.1:{port}{path}"));
    if let Some(token) = token {
        request = request.header("X-Playbook-Token", token);
    }
    let mut response = request
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .expect("the dashboard server should answer");
    let status = response.status().as_u16();
    (status, response.body_mut().read_to_string().unwrap())
}

#[test]
fn the_data_route_answers_401_without_a_token_and_with_a_wrong_one() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("denied");
    home.seed();
    let (port, token) = start(&home);

    let none = get(port, "/api/data", None);
    let wrong = get(port, "/api/data", Some(&"0".repeat(64)));
    let short = get(port, "/api/data", Some(&token[..10]));
    let longer = get(port, "/api/data", Some(&format!("{token}0")));

    for (status, body) in [none, wrong, short, longer] {
        assert_eq!(status, 401);
        assert_eq!(body, "unauthorized");
    }
}

#[test]
fn the_right_token_gets_the_data() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("granted");
    home.seed();
    let (port, token) = start(&home);

    let (status, body) = get(port, "/api/data", Some(&token));

    assert_eq!(status, 200);
    let data: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(data["totals"]["messages"], 3);
    assert_eq!(data["groups"]["account"][0]["key"], "dev@example.com");
}

#[test]
fn a_right_token_with_a_foreign_host_still_gets_the_host_refusal() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("host");
    home.seed();
    let (port, token) = start(&home);

    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .write_all(
            format!(
                "GET /api/data HTTP/1.1\r\nHost: evil.example\r\nX-Playbook-Token: {token}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();

    assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
    assert!(!reply.contains("totals"), "no data may leak: {reply}");
}

#[test]
fn a_right_token_with_a_cross_site_label_is_still_refused() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("site");
    home.seed();
    let (port, token) = start(&home);

    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .write_all(
            format!(
                "GET /api/data HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nSec-Fetch-Site: cross-site\r\nX-Playbook-Token: {token}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();

    assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
    assert!(!reply.contains("totals"));
}

#[test]
fn the_reuse_path_prints_a_working_link_with_the_token_from_the_lock() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("reuse");
    home.seed();
    let first = stdout(&home.run(&["usage", "dashboard"]));
    let second = stdout(&home.run(&["usage", "dashboard"]));

    let (port, token) = link_in(&second);

    assert!(first.contains("started"), "{first}");
    assert!(second.contains("already running"), "{second}");
    assert_eq!(token, home.lock_fields()[2]);
    assert_eq!(link_in(&first), (port, token.clone()));
    assert!(second.contains(&format!("http://playbook.localhost:{port}/#{token}")));
    assert_eq!(get(port, "/api/data", Some(&token)).0, 200);
}

#[test]
fn the_token_appears_only_in_the_two_links() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("print");
    let out = home.run(&["usage", "dashboard"]);
    let text = stdout(&out);
    let (_, token) = link_in(&text);

    assert_eq!(text.matches(&token).count(), 2, "{text}");
    assert!(!String::from_utf8_lossy(&out.stderr).contains(&token));
    let stopped = home.run(&["usage", "dashboard", "stop"]);
    assert!(!stdout(&stopped).contains(&token));
}

#[test]
fn an_old_two_field_lock_is_retired_and_replaced_by_a_tokened_server() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("legacy");
    fs::create_dir_all(home.lock_path().parent().unwrap()).unwrap();
    fs::write(home.lock_path(), "2147483646 9\n").unwrap();

    let out = home.run(&["usage", "dashboard"]);

    let text = stdout(&out);
    assert!(out.status.success(), "{text}");
    let fields = home.lock_fields();
    assert_eq!(fields.len(), 3, "{fields:?}");
    assert_ne!(fields[0], "2147483646");
    assert_eq!(fields[2].len(), 64);
}

#[test]
fn the_lock_file_and_its_directory_are_owner_only() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("mode");
    start(&home);

    let file = fs::metadata(home.lock_path()).unwrap().permissions().mode() & 0o777;
    let dir = fs::metadata(home.lock_path().parent().unwrap())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;

    assert_eq!(file, 0o600);
    assert_eq!(dir, 0o700);
}

#[test]
fn each_server_start_gets_a_fresh_token() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("fresh");
    let (_, first) = start(&home);
    home.run(&["usage", "dashboard", "stop"]);

    let (_, second) = start(&home);

    assert_ne!(first, second);
}

#[test]
fn the_page_and_its_assets_serve_without_a_token_and_carry_no_data_or_token() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("assets");
    home.seed();
    let (port, token) = start(&home);

    for (path, kind) in [
        ("/", "<script src=\"/app.js\">"),
        ("/app.js", "X-Playbook-Token"),
        ("/app.css", "--bg"),
    ] {
        let (status, body) = get(port, path, None);
        assert_eq!(status, 200, "{path}");
        assert!(body.contains(kind), "{path}");
        assert!(!body.contains(&token), "{path} must not carry the token");
        assert!(
            !body.contains("dev@example.com"),
            "{path} must hold no data"
        );
    }
}

#[test]
fn assets_carry_the_strict_policy_and_the_right_content_types() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("headers");
    let (port, _) = start(&home);

    for (path, kind) in [
        ("/", "text/html"),
        ("/app.js", "text/javascript"),
        ("/app.css", "text/css"),
    ] {
        let response = ureq::get(&format!("http://127.0.0.1:{port}{path}"))
            .call()
            .unwrap();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .map(|v| v.to_str().unwrap().to_string())
        };
        assert!(header("content-type").unwrap().starts_with(kind), "{path}");
        assert_eq!(header("x-content-type-options").as_deref(), Some("nosniff"));
        let policy = header("content-security-policy").expect("a policy on every asset");
        assert!(policy.contains("script-src 'self'"), "{policy}");
        assert!(!policy.contains("unsafe-inline"), "{policy}");
    }
}

#[test]
fn a_hostile_branch_name_reaches_the_data_and_the_svg_only_as_escaped_text() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("xss");
    let project = home.0.join(".claude/projects/proj-xss");
    fs::create_dir_all(&project).unwrap();
    let line = "{\"type\":\"assistant\",\"uuid\":\"u1\",\"sessionId\":\"s\",\"timestamp\":\"2026-09-05T10:00:01.000Z\",\"cwd\":\"/w/<img src=x onerror=alert(1)>\",\"gitBranch\":\"<img src=x onerror=alert(1)>\",\"message\":{\"id\":\"msg_xss\",\"model\":\"claude-sonnet-5\",\"content\":[],\"usage\":{\"input_tokens\":1000,\"output_tokens\":1000}}}\n";
    fs::write(project.join("x.jsonl"), line).unwrap();
    let (port, token) = start(&home);

    let (status, body) = get(port, "/api/data", Some(&token));

    assert_eq!(status, 200);
    let data: serde_json::Value = serde_json::from_str(&body).unwrap();
    let branches = data["groups"]["branch"].as_array().unwrap();
    assert_eq!(branches[0]["key"], "<img src=x onerror=alert(1)>");
    for chart in ["cost_by_day", "cost_by_model"] {
        let svg = data["charts"][chart].as_str().unwrap();
        assert!(!svg.contains("<img"), "{chart}: {svg}");
    }
    let (_, page) = get(port, "/app.js", None);
    assert!(
        page.contains("textContent"),
        "dynamic text goes through textContent"
    );
}

const GUARDED: [&str; 3] = [
    "/api/data?range=30d",
    "/api/sessions?range=all",
    "/api/session?id=s1",
];

#[test]
fn every_data_route_answers_401_without_a_token_and_json_with_one() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("guarded");
    home.seed();
    let (port, token) = start(&home);

    for path in GUARDED {
        assert_eq!(
            get(port, path, None),
            (401, "unauthorized".to_string()),
            "{path}"
        );
        assert_eq!(get(port, path, Some(&"0".repeat(64))).0, 401, "{path}");
        let (status, body) = get(port, path, Some(&token));
        assert_eq!(status, 200, "{path}");
        serde_json::from_str::<serde_json::Value>(&body).expect("JSON only");
    }
}

#[test]
fn the_sessions_route_lists_sessions_and_the_session_route_returns_the_timeline() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("sessions");
    home.seed();
    let (port, token) = start(&home);

    let (_, body) = get(port, "/api/sessions?range=all", Some(&token));
    let list: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(list["total"], 1);
    assert_eq!(list["sessions"][0]["id"], "s1");
    assert_eq!(list["sessions"][0]["messages"], 3);

    let (status, body) = get(port, "/api/session?id=s1", Some(&token));
    let one: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(status, 200);
    assert_eq!(one["messages"].as_array().unwrap().len(), 3);
    assert!(one["chart"].as_str().unwrap().starts_with("<svg"));

    assert_eq!(get(port, "/api/session?id=nope", Some(&token)).0, 404);
    for bad in [
        "",
        "?id=",
        "?id=a%2Fb",
        "?id=..",
        "?id=a%20b",
        &format!("?id={}", "a".repeat(129)),
    ] {
        let path = format!("/api/session{bad}");
        assert_eq!(get(port, &path, Some(&token)).0, 400, "{path}");
    }
    assert_eq!(get(port, "/api/sessions?range=7d", Some(&token)).0, 400);
}

#[test]
fn a_foreign_host_or_a_cross_site_label_is_refused_on_every_data_route() {
    let _guard = SOCKET_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Home::new("guard-all");
    home.seed();
    let (port, token) = start(&home);

    for path in GUARDED {
        for headers in [
            "Host: evil.example\r\n".to_string(),
            format!("Host: 127.0.0.1:{port}\r\nSec-Fetch-Site: cross-site\r\n"),
            format!("Host: 127.0.0.1:{port}\r\nSec-Fetch-Site: same-site\r\n"),
        ] {
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .write_all(
                    format!(
                        "GET {path} HTTP/1.1\r\n{headers}X-Playbook-Token: {token}\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).unwrap();
            assert!(
                reply.starts_with("HTTP/1.1 403"),
                "{path} {headers}: {reply}"
            );
            assert!(!reply.contains("messages"), "no data may leak: {reply}");
        }
    }
}
