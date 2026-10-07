// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Spawned-binary tests for the live stream (`/api/live`): the guard, the first
//! event, new data, the stream cap, a freed slot, and the rest of the server
//! staying responsive while streams are open. Scratch `$HOME`, one server at a
//! time behind `SOCKET_LOCK`.

#![cfg(unix)]

use playbook::usage::aggregate::date_key;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// Declared per test binary: a static Mutex cannot span separate processes.
static SOCKET_LOCK: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("playbook-live-{}-{tag}-{n}", std::process::id()));
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

    fn lock_fields(&self) -> Vec<String> {
        fs::read_to_string(self.0.join(".config/playbook/usage/dashboard.lock"))
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect()
    }

    fn transcript(&self) -> PathBuf {
        self.0.join(".claude/projects/proj-live/live.jsonl")
    }

    /// Appends one assistant message dated now.
    fn say(&self, id: &str, model: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let (day, rest) = (now.div_euclid(86_400), now.rem_euclid(86_400));
        let stamp = format!(
            "{}T{:02}:{:02}:{:02}.000Z",
            date_key(day),
            rest / 3600,
            rest % 3600 / 60,
            rest % 60
        );
        let line = format!(
            "{{\"type\":\"assistant\",\"uuid\":\"u-{id}\",\"sessionId\":\"live-1\",\"timestamp\":\"{stamp}\",\"cwd\":\"/home/u/Work/proj-live\",\"gitBranch\":\"main\",\"effort\":\"high\",\"message\":{{\"id\":\"{id}\",\"model\":\"{model}\",\"content\":[],\"usage\":{{\"input_tokens\":5,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0,\"output_tokens\":7}}}}}}\n"
        );
        fs::create_dir_all(self.transcript().parent().unwrap()).unwrap();
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.transcript())
            .unwrap();
        file.write_all(line.as_bytes()).unwrap();
    }

    fn start(&self) -> (u16, String) {
        let out = self.run(&["usage", "dashboard"]);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
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
            .unwrap()
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect();
        (port, token)
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

fn request(port: u16, extra_headers: &str, token: Option<&str>) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let host = if extra_headers.contains("Host:") {
        String::new()
    } else {
        format!("Host: 127.0.0.1:{port}\r\n")
    };
    let token = token.map_or(String::new(), |t| format!("X-Playbook-Token: {t}\r\n"));
    stream
        .write_all(format!("GET /api/live HTTP/1.1\r\n{host}{extra_headers}{token}\r\n").as_bytes())
        .unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    stream
}

/// Reads until `needle` appears or `within` passes; returns what arrived.
fn read_until(stream: &mut TcpStream, needle: &str, within: Duration) -> String {
    let deadline = Instant::now() + within;
    let mut seen = Vec::new();
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => seen.extend_from_slice(&buf[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
        if String::from_utf8_lossy(&seen).contains(needle) {
            break;
        }
    }
    String::from_utf8_lossy(&seen).into_owned()
}

fn status_line(reply: &str) -> &str {
    reply.lines().next().unwrap_or("")
}

#[test]
fn the_stream_needs_the_token_the_host_and_a_same_origin_label() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("guard");
    home.say("m1", "claude-sonnet-5");
    let (port, token) = home.start();

    let none = read_until(
        &mut request(port, "", None),
        "\r\n\r\n",
        Duration::from_secs(5),
    );
    let wrong = read_until(
        &mut request(port, "", Some(&"0".repeat(64))),
        "\r\n\r\n",
        Duration::from_secs(5),
    );
    let host = read_until(
        &mut request(port, "Host: evil.example\r\n", Some(&token)),
        "\r\n\r\n",
        Duration::from_secs(5),
    );
    let cross = read_until(
        &mut request(port, "Sec-Fetch-Site: cross-site\r\n", Some(&token)),
        "\r\n\r\n",
        Duration::from_secs(5),
    );

    assert!(status_line(&none).contains("401"), "{none}");
    assert!(status_line(&wrong).contains("401"), "{wrong}");
    assert!(status_line(&host).contains("403"), "{host}");
    assert!(status_line(&cross).contains("403"), "{cross}");
    for reply in [none, wrong, host, cross] {
        assert!(!reply.contains("event: live"), "no data may leak: {reply}");
    }
}

#[test]
fn the_first_event_arrives_promptly_and_a_new_transcript_line_shows_in_a_later_one() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("events");
    home.say("m1", "claude-sonnet-5");
    let (port, token) = home.start();
    let began = Instant::now();

    let mut stream = request(port, "", Some(&token));
    let first = read_until(&mut stream, "\n\n", Duration::from_secs(5));

    assert!(began.elapsed() < Duration::from_secs(5));
    assert!(status_line(&first).contains("200"), "{first}");
    assert!(first.contains("text/event-stream"), "{first}");
    assert!(first.contains("Transfer-Encoding: chunked"), "{first}");
    assert!(first.contains("event: live"), "{first}");
    assert!(first.contains("claude-sonnet-5"), "{first}");
    assert!(!first.contains("claude-opus-5"));

    home.say("m2", "claude-opus-5");
    let later = read_until(&mut stream, "claude-opus-5", Duration::from_secs(10));

    assert!(
        later.contains("claude-opus-5"),
        "the new line must stream: {later}"
    );
}

#[test]
fn the_cap_returns_503_a_closed_stream_frees_its_slot_and_the_server_stays_responsive() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("cap");
    home.say("m1", "claude-sonnet-5");
    let (port, token) = home.start();

    let mut open: Vec<TcpStream> = (0..4)
        .map(|_| {
            let mut s = request(port, "", Some(&token));
            let first = read_until(&mut s, "event: live", Duration::from_secs(10));
            assert!(status_line(&first).contains("200"), "{first}");
            s
        })
        .collect();

    // The fifth is refused, and the rest of the server still answers.
    let fifth = read_until(
        &mut request(port, "", Some(&token)),
        "\r\n\r\n",
        Duration::from_secs(5),
    );
    assert!(status_line(&fifth).contains("503"), "{fifth}");
    let began = Instant::now();
    let mut data = TcpStream::connect(("127.0.0.1", port)).unwrap();
    data.write_all(
        format!("GET /api/data?range=all HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Playbook-Token: {token}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .unwrap();
    let mut body = String::new();
    data.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    data.read_to_string(&mut body).unwrap();
    assert!(status_line(&body).contains("200"), "{body}");
    assert!(began.elapsed() < Duration::from_secs(8));

    // Closing one stream frees its slot within a tick or two.
    drop(open.pop());
    let deadline = Instant::now() + Duration::from_secs(15);
    let freed = loop {
        let mut again = request(port, "", Some(&token));
        let reply = read_until(&mut again, "\r\n\r\n", Duration::from_secs(3));
        if status_line(&reply).contains("200") {
            break true;
        }
        if Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    assert!(freed, "a disconnected client must free its slot");
}
