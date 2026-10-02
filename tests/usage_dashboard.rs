// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Spawned-binary tests for the dashboard server lifecycle. Each test gets a
//! scratch `$HOME`, sets `PLAYBOOK_USAGE_NO_BROWSER` so no real browser is
//! launched, and stops its server on drop. Tests that start a server run one
//! at a time behind `SOCKET_LOCK`.

#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

// Declared per test binary: a static Mutex cannot coordinate across the
// separate processes cargo runs for unit and integration tests.
static SOCKET_LOCK: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("playbook-dash-{}-{tag}-{n}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Home(dir.canonicalize().unwrap())
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_playbook"));
        command
            .args(args)
            .env("HOME", &self.0)
            .env("PLAYBOOK_USAGE_NO_BROWSER", "1");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// Copies a committed fixture transcript and the account file into the
    /// scratch home, so the server never reads the real `~/.claude`.
    fn seed(&self, fixture: &str) {
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/usage");
        let project = self.0.join(".claude/projects/proj-one");
        fs::create_dir_all(&project).unwrap();
        fs::copy(fixtures.join(fixture), project.join("seeded.jsonl")).unwrap();
        fs::copy(
            fixtures.join("account/claude.json"),
            self.0.join(".claude.json"),
        )
        .unwrap();
    }

    fn lock_path(&self) -> PathBuf {
        self.0.join(".config/playbook/usage/dashboard.lock")
    }

    fn lock(&self) -> Option<(u32, u16)> {
        let text = fs::read_to_string(self.lock_path()).ok()?;
        let mut parts = text.split_whitespace();
        Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self.run(&["usage", "dashboard", "stop"]);
        if let Some((pid, _)) = self.lock() {
            let _ = Command::new("kill").args(["-9", &pid.to_string()]).output();
        }
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn port_in(text: &str) -> u16 {
    let after = text
        .split("http://127.0.0.1:")
        .nth(1)
        .expect("an IP URL in the output");
    after
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn http_status(port: u16, path: &str) -> u16 {
    let response = ureq::get(&format!("http://127.0.0.1:{port}{path}"))
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .expect("the dashboard server should answer");
    response.status().as_u16()
}

fn serve_process_count(exe: &std::path::Path) -> usize {
    let pattern = format!("{} usage dashboard --serve", exe.display());
    let out = Command::new("pgrep")
        .args(["-f", &pattern])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).lines().count()
}

#[test]
fn start_serves_http_and_a_second_call_reuses_the_same_server() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("reuse");

    let first = home.run(&["usage", "dashboard"]);
    let first_text = stdout(&first);
    let port = port_in(&first_text);
    let (pid, locked_port) = home.lock().expect("the server writes its own lock");
    let second = stdout(&home.run(&["usage", "dashboard"]));

    assert!(first.status.success());
    assert!(first_text.contains("started"), "{first_text}");
    assert!(first_text.contains(&format!("http://playbook.localhost:{port}")));
    assert_eq!(locked_port, port);
    assert!(alive(pid));
    assert_eq!(http_status(port, "/"), 200);
    assert_eq!(http_status(port, "/missing"), 404);
    assert!(second.contains("already running"), "{second}");
    assert_eq!(port_in(&second), port);
}

#[test]
fn a_stale_lock_is_replaced_by_a_fresh_server() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("stale");
    fs::create_dir_all(home.lock_path().parent().unwrap()).unwrap();
    fs::write(home.lock_path(), "2147483646 9\n").unwrap();

    let out = home.run(&["usage", "dashboard"]);

    let text = stdout(&out);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("started"), "{text}");
    let (pid, port) = home.lock().unwrap();
    assert_ne!(pid, 2147483646);
    assert_eq!(http_status(port, "/"), 200);
}

#[test]
fn a_server_killed_out_of_band_is_recovered_on_the_next_call() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("crash");
    home.run(&["usage", "dashboard"]);
    let (dead_pid, _) = home.lock().unwrap();
    Command::new("kill")
        .args(["-9", &dead_pid.to_string()])
        .output()
        .unwrap();
    wait_until("the killed server to exit", || !alive(dead_pid));

    let out = home.run(&["usage", "dashboard"]);

    let text = stdout(&out);
    assert!(text.contains("started"), "{text}");
    let (new_pid, port) = home.lock().unwrap();
    assert_ne!(new_pid, dead_pid);
    assert_eq!(http_status(port, "/"), 200);
}

#[test]
fn two_concurrent_starts_leave_exactly_one_server() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("race");
    // A uniquely named copy of the binary, so only this test's servers match
    // the process count, whatever else is running on the machine.
    let exe = home.0.join("playbook-race");
    fs::copy(env!("CARGO_BIN_EXE_playbook"), &exe).unwrap();
    let spawn_piped = || {
        let mut command = Command::new(&exe);
        command
            .args(["usage", "dashboard"])
            .env("HOME", &home.0)
            .env("PLAYBOOK_USAGE_NO_BROWSER", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.spawn().unwrap()
    };

    let a = spawn_piped();
    let b = spawn_piped();
    let (a, b) = (a.wait_with_output().unwrap(), b.wait_with_output().unwrap());

    let (a_text, b_text) = (stdout(&a), stdout(&b));
    assert!(
        a.status.success() && b.status.success(),
        "{a_text} | {b_text}"
    );
    assert_eq!(
        port_in(&a_text),
        port_in(&b_text),
        "both callers must report the winner"
    );
    wait_until("the losing server to exit", || {
        serve_process_count(&exe) == 1
    });
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        serve_process_count(&exe),
        1,
        "the loser must not come back or linger"
    );
    let (pid, port) = home.lock().unwrap();
    assert!(alive(pid));
    assert_eq!(http_status(port, "/"), 200);
}

#[test]
fn stop_ends_the_server_and_clears_the_lock_and_is_safe_when_nothing_runs() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("stop");
    home.run(&["usage", "dashboard"]);
    let (pid, _) = home.lock().unwrap();

    let stopped = home.run(&["usage", "dashboard", "stop"]);
    let again = home.run(&["usage", "dashboard", "stop"]);

    assert!(stopped.status.success());
    assert!(stdout(&stopped).contains("stopped"), "{}", stdout(&stopped));
    assert!(!alive(pid));
    assert!(!home.lock_path().exists());
    assert!(again.status.success());
    assert!(stdout(&again).contains("nothing is running"));
}

fn http_get(port: u16, path: &str) -> (u16, String) {
    let mut response = ureq::get(&format!("http://127.0.0.1:{port}{path}"))
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .expect("the dashboard server should answer");
    let status = response.status().as_u16();
    (status, response.body_mut().read_to_string().unwrap())
}

fn start(home: &Home) -> u16 {
    port_in(&stdout(&home.run(&["usage", "dashboard"])))
}

#[test]
fn api_data_ingests_transcripts_and_returns_the_fixture_totals() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("data");
    home.seed("usage/proj-one/s1.jsonl");
    let port = start(&home);

    let (status, body) = http_get(port, "/api/data");

    assert_eq!(status, 200);
    let data: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(data["totals"]["messages"], 3);
    assert_eq!(data["totals"]["output_tokens"], 1470);
    assert!((data["totals"]["cost_usd"].as_f64().unwrap() - 0.14045815).abs() < 1e-9);
    assert_eq!(data["groups"]["model"].as_array().unwrap().len(), 3);
    assert_eq!(data["groups"]["account"][0]["key"], "dev@example.com");
    assert_eq!(data["groups"]["day"].as_array().unwrap().len(), 2);
    assert_eq!(
        data["charts"]["cost_by_day"]
            .as_str()
            .unwrap()
            .matches("<rect")
            .count(),
        2
    );
}

#[test]
fn api_data_reports_skill_and_agent_counts() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("tools");
    home.seed("tools/proj-one/s2.jsonl");
    let port = start(&home);

    let (_, body) = http_get(port, "/api/data");

    let data: serde_json::Value = serde_json::from_str(&body).unwrap();
    let names = |key: &str| -> Vec<String> {
        data[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(names("skills"), vec!["playbook:implement", "playbook:plan"]);
    assert_eq!(names("agents"), vec!["Draft briefs", "Explore"]);
}

#[test]
fn a_new_transcript_shows_up_on_the_next_request_without_a_restart() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("refresh");
    let port = start(&home);
    let empty: serde_json::Value = serde_json::from_str(&http_get(port, "/api/data").1).unwrap();

    home.seed("usage/proj-one/s1.jsonl");
    let filled: serde_json::Value = serde_json::from_str(&http_get(port, "/api/data").1).unwrap();

    assert_eq!(empty["totals"]["messages"], 0);
    assert_eq!(filled["totals"]["messages"], 3);
}

#[test]
fn the_page_is_served_with_security_headers() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("page");
    let port = start(&home);

    let response = ureq::get(&format!("http://127.0.0.1:{port}/"))
        .call()
        .unwrap();

    assert_eq!(response.status().as_u16(), 200);
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .map(|v| v.to_str().unwrap().to_string())
    };
    assert!(header("content-type").unwrap().starts_with("text/html"));
    assert_eq!(header("x-content-type-options").as_deref(), Some("nosniff"));
    assert!(header("content-security-policy")
        .unwrap()
        .contains("default-src 'none'"));
}

#[test]
fn a_request_with_a_foreign_host_header_is_refused() {
    use std::io::{Read, Write};
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("host");
    let port = start(&home);

    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .write_all(b"GET /api/data HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();

    assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
    assert!(
        !reply.contains("totals"),
        "no data may leak to a foreign host"
    );
}
