// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Spawned-binary tests for the dashboard's local-access hardening: who may
//! fetch the data, what the page policy forbids, and who can read the files.
//! Each test gets a scratch `$HOME` and a stopped server on drop.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

// Declared per test binary: a static Mutex cannot coordinate across the
// separate processes cargo runs for each test file.
static SOCKET_LOCK: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("playbook-sec-{}-{tag}-{n}", std::process::id()));
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

    fn usage_dir(&self) -> PathBuf {
        self.0.join(".config/playbook/usage")
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self.run(&["usage", "dashboard", "stop"]);
        if let Ok(text) = fs::read_to_string(self.usage_dir().join("dashboard.lock")) {
            if let Some(pid) = text.split_whitespace().next() {
                let _ = Command::new("kill").args(["-9", pid]).output();
            }
        }
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn start(home: &Home) -> u16 {
    let out = home.run(&["usage", "dashboard"]);
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{text}");
    text.split("http://127.0.0.1:")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok())
        .unwrap_or_else(|| panic!("no port in: {text}"))
}

/// The session token the server wrote into its lock, as the opened link carries it.
fn token(home: &Home) -> String {
    let text = fs::read_to_string(home.usage_dir().join("dashboard.lock")).expect("a lock file");
    text.split_whitespace()
        .nth(2)
        .expect("a token field")
        .to_string()
}

fn get(port: u16, path: &str, site: Option<&str>, token: &str) -> (u16, Option<String>) {
    let mut request =
        ureq::get(&format!("http://127.0.0.1:{port}{path}")).header("X-Playbook-Token", token);
    if let Some(site) = site {
        request = request.header("Sec-Fetch-Site", site);
    }
    let response = request
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .expect("the dashboard server should answer");
    let policy = response
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    (response.status().as_u16(), policy)
}

#[test]
fn another_sites_page_cannot_make_the_server_ingest_but_the_page_itself_can() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("fetch-site");
    home.seed();
    let port = start(&home);

    for (site, expected) in [
        (None, 200),
        (Some("same-origin"), 200),
        (Some("none"), 200),
        (Some("cross-site"), 403),
        (Some("same-site"), 403),
    ] {
        assert_eq!(
            get(port, "/api/data", site, &token(&home)).0,
            expected,
            "{site:?}"
        );
    }
}

#[test]
fn the_page_forbids_being_framed() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("csp");
    let port = start(&home);

    let (status, policy) = get(port, "/", None, "");

    assert_eq!(status, 200);
    let policy = policy.expect("the page carries a policy");
    assert!(policy.contains("frame-ancestors 'none'"), "{policy}");
}

#[test]
fn the_usage_directory_database_and_lock_are_owner_only() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("modes");
    home.seed();
    let port = start(&home);
    assert_eq!(get(port, "/api/data", None, &token(&home)).0, 200);

    let mode = |p: PathBuf| fs::metadata(p).unwrap().permissions().mode() & 0o777;

    assert_eq!(mode(home.usage_dir()), 0o700);
    assert_eq!(mode(home.usage_dir().join("usage.db")), 0o600);
    assert_eq!(mode(home.usage_dir().join("dashboard.lock")), 0o600);
}

#[test]
fn stop_ignores_a_lock_that_names_pid_one_or_zero() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("stop-low-pid");
    fs::create_dir_all(home.usage_dir()).unwrap();

    for text in ["0 8080\n", "1 8080\n"] {
        fs::write(home.usage_dir().join("dashboard.lock"), text).unwrap();

        let out = home.run(&["usage", "dashboard", "stop"]);

        let said = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(out.status.success(), "{said}");
        assert!(said.contains("nothing is running"), "{said}");
    }
}

#[test]
fn the_session_routes_refuse_another_sites_page_like_the_data_route() {
    let _guard = SOCKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new("fetch-site-sessions");
    home.seed();
    let port = start(&home);

    for path in ["/api/sessions?range=all", "/api/session?id=s1"] {
        for (site, expected) in [
            (None, 200),
            (Some("same-origin"), 200),
            (Some("none"), 200),
            (Some("cross-site"), 403),
            (Some("same-site"), 403),
        ] {
            let (status, policy) = get(port, path, site, &token(&home));
            assert_eq!(status, expected, "{path} {site:?}");
            assert!(policy.is_none(), "JSON carries no page policy");
        }
    }
}
