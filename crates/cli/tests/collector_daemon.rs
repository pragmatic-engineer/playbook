// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The collector daemon end to end: which playbook commands and hooks start
//! it, and the done bars D1, D2, D3, D5 and D6 of the collector daemon plan.
//! Every test runs the real binary in a scratch HOME, and every test that can
//! start a collector holds a `Daemon` guard that kills it.

#[path = "support/isolated.rs"]
mod isolated;

use playbook::cc::telemetry::TEST_SPAWN_VAR;
use playbook::usage::collector::{probe, Probe};
use playbook::usage::daemon::{
    lock_path, lock_state, stamp_path, LockState, STAMP_SECS_VAR, TEST_HANG_VAR, TEST_VARS,
    TEST_VERSION_VAR,
};
use playbook::usage::run::Paths;
use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const TOKEN: &str = "test-token";
/// D1: an OTLP request is accepted this soon after the hook starts.
const D1_BOUND: Duration = Duration::from_millis(1500);
/// D5: a hung collector is replaced this soon after the hook starts.
const D5_BOUND: Duration = Duration::from_secs(5);
/// How long a collector the test or a command started may take to come up.
const UP_BOUND: Duration = Duration::from_secs(10);
/// How long a killed collector may take to let go of the lock.
const GONE_BOUND: Duration = Duration::from_secs(5);
/// D2: how long the test waits after the last hook, so every losing
/// collector has had time to start and give up.
const SETTLE: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(50);
/// How long the guard watches for a late collector after the lock freed.
const RECHECK: Duration = Duration::from_millis(500);
/// The CI test count for D2, and the stress count.
const CI_STARTS: usize = 50;
const STRESS_STARTS: usize = 1000;
/// The version a staged older collector reports (D6).
const OLD_VERSION: &str = "0.0.1";
const SESSION_START: &[u8] = br#"{"session_id":"s1","source":"startup"}"#;
const PROMPT: &[u8] =
    br#"{"session_id":"s1","prompt":"hello","hook_event_name":"UserPromptSubmit"}"#;

/// Polls `done` every `POLL` until `bound`, checking it before the deadline
/// so a condition met on the last poll still counts.
fn within(bound: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 only checks that the pid exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Sets a test-only variable on `cmd`. A release build never reads one, so a
/// test that needs it would pass for the wrong reason: fail loudly instead.
fn test_var(cmd: &mut Command, name: &str, value: &str) {
    if !cfg!(debug_assertions) {
        panic!("{name} is read only by debug builds; run these tests in the debug profile");
    }
    cmd.env(name, value);
}

/// A scratch home where telemetry is set up for the playbook collector: a
/// recorded port and token, and user settings that point at them.
struct Home {
    home: PathBuf,
    port: u16,
}

impl Home {
    fn new() -> Home {
        Home::with_state(|port| format!(r#"{{"pid":0,"port":{port},"token":"{TOKEN}"}}"#))
    }

    /// A home whose `collector.json` is `state(port)`.
    fn with_state(state: impl Fn(u16) -> String) -> Home {
        let home = Home::bare();
        let data = home.data();
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("collector.json"), state(home.port)).unwrap();
        home.point_settings_here();
        home
    }

    /// A scratch home with a free port picked and nothing written.
    fn bare() -> Home {
        // A port free now: the shared default would make parallel tests
        // fight over one port. Nothing holds it, so a clash stays possible
        // but rare.
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        Home {
            home: isolated::fresh_home(),
            port,
        }
    }

    fn point_settings_here(&self) {
        fs::create_dir_all(self.home.join(".claude")).unwrap();
        fs::write(
            self.home.join(".claude/settings.json"),
            format!(
                r#"{{"env":{{"CLAUDE_CODE_ENABLE_TELEMETRY":"1","OTEL_EXPORTER_OTLP_ENDPOINT":"http://127.0.0.1:{}"}}}}"#,
                self.port
            ),
        )
        .unwrap();
    }

    fn data(&self) -> PathBuf {
        self.home.join(".config/playbook/data")
    }

    fn paths(&self) -> Paths {
        Paths::from_home(&self.home)
    }

    /// The binary in this home, allowed to start a collector, with no
    /// headless switch from the caller (CI sets `CI=true`).
    fn cmd(&self) -> Command {
        let mut cmd = isolated::playbook_in(&self.home);
        cmd.env_remove("CI")
            .env_remove("CLAUDE_CODE_ENABLE_TELEMETRY")
            .current_dir(&self.home);
        for (name, _) in std::env::vars() {
            if name.starts_with("OTEL_") {
                cmd.env_remove(name);
            }
        }
        // Only debug builds read it; a release build spawns without it.
        cmd.env(TEST_SPAWN_VAR, "1");
        cmd
    }

    fn run(&self, mut cmd: Command, stdin: &[u8]) -> Output {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child.stdin.take().unwrap().write_all(stdin);
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{out:?}");
        out
    }

    fn hook(&self, name: &str, stdin: &[u8]) -> Output {
        let mut cmd = self.cmd();
        cmd.args(["hook", name]);
        self.run(cmd, stdin)
    }

    fn session_start(&self) -> Output {
        self.hook("session-init", SESSION_START)
    }

    fn lock(&self) -> LockState {
        lock_state(&self.paths())
    }

    fn holder(&self) -> Option<(u32, String)> {
        match self.lock() {
            LockState::Held(h) => Some((h.pid?, h.version?)),
            _ => None,
        }
    }

    /// Nothing was started or attempted: no lock file and no stamp.
    fn assert_untouched(&self) {
        let paths = self.paths();
        assert!(!lock_path(&paths).exists(), "a lock file was created");
        assert!(!stamp_path(&paths).exists(), "a spawn was attempted");
    }

    fn log(&self) -> String {
        fs::read_to_string(self.data().join("collector.log")).unwrap_or_default()
    }

    /// A collector this test starts itself, with test-only variables.
    fn start_collector(&self, daemon: &mut Daemon, vars: &[(&str, &str)]) -> u32 {
        let mut cmd = isolated::playbook_in(&self.home);
        cmd.args(["usage", "collect", "--idle-seconds", "0"])
            .current_dir(&self.home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (name, value) in vars {
            test_var(&mut cmd, name, value);
        }
        daemon.child = Some(cmd.spawn().unwrap());
        assert!(
            within(UP_BOUND, || self.holder().is_some() && answers(self.port)),
            "the collector did not come up"
        );
        self.holder().unwrap().0
    }
}

fn answers(port: u16) -> bool {
    TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_ok()
}

/// Posts one empty OTLP log batch and returns the status code.
fn post_log(port: u16) -> u16 {
    let Ok(mut s) = TcpStream::connect((Ipv4Addr::LOCALHOST, port)) else {
        return 0;
    };
    let body = br#"{"resourceLogs":[]}"#;
    let head = format!(
        "POST /v1/logs HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    if s.write_all(head.as_bytes()).is_err() || s.write_all(body).is_err() {
        return 0;
    }
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out.split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0)
}

/// Kills whatever collector holds the lock of `paths` when the test ends,
/// passing or not, and waits for the lock to free.
struct Daemon {
    paths: Paths,
    home: PathBuf,
    child: Option<Child>,
}

impl Daemon {
    fn new(home: &Home) -> Daemon {
        // A test variable in this process would reach a child that the
        // helper does not clean, and a release build would ignore it.
        let stray: Vec<_> = TEST_VARS
            .iter()
            .filter(|v| std::env::var_os(v).is_some())
            .collect();
        assert!(
            stray.is_empty() || cfg!(debug_assertions),
            "test-only variables {stray:?} are set but this is a release build"
        );
        Daemon {
            paths: home.paths(),
            home: home.home.clone(),
            child: None,
        }
    }

    /// Reaps the collector this test started, once it has exited.
    fn reap(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.try_wait();
        }
    }

    fn kill_holder(&self) -> bool {
        match lock_state(&self.paths) {
            LockState::Held(h) => {
                if let Some(pid) = h.pid.and_then(|p| libc::pid_t::try_from(p).ok()) {
                    // SAFETY: plain kill(2) on the pid the lock names.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                }
                true
            }
            _ => false,
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // A collector still starting writes its pid a moment later, and a
        // late spawn can take the lock after the first one goes: repeat.
        let free = within(GONE_BOUND, || {
            self.kill_holder();
            lock_state(&self.paths) == LockState::Free
        });
        let settled = free && !within(RECHECK, || self.kill_holder());
        let _ = fs::remove_dir_all(&self.home);
        if !std::thread::panicking() {
            assert!(free && settled, "a collector kept the lock after SIGKILL");
        }
    }
}

#[test]
fn d1_a_session_start_with_no_collector_accepts_otlp_within_the_bound() {
    let home = Home::new();
    let _daemon = Daemon::new(&home);
    let began = Instant::now();

    home.session_start();
    let accepted = within(D1_BOUND.saturating_sub(began.elapsed()), || {
        post_log(home.port) == 200
    });

    assert!(
        accepted,
        "no 200 within {D1_BOUND:?} (took {:?}); log: {}",
        began.elapsed(),
        home.log()
    );
}

/// Starts `count` `session-init` hooks at once with the stamp window at zero,
/// so every one of them may spawn, then checks that one collector won.
fn concurrent_starts_leave_one_collector(count: usize) {
    let home = Home::new();
    let _daemon = Daemon::new(&home);
    let mut children: Vec<Child> = (0..count)
        .map(|_| {
            let mut cmd = home.cmd();
            cmd.args(["hook", "session-init"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            test_var(&mut cmd, STAMP_SECS_VAR, "0");
            cmd.spawn().unwrap()
        })
        .collect();
    for child in &mut children {
        let _ = child.stdin.take().unwrap().write_all(SESSION_START);
    }
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    std::thread::sleep(SETTLE);

    let log = home.log();
    assert_eq!(log.matches("listening on").count(), 1, "{log}");
    let (pid, version) = home.holder().expect("the lock is held");
    assert!(pid_alive(pid));
    assert_eq!(version, VERSION);
    assert_eq!(probe(home.port), Probe::Collector);
    let lock = fs::File::open(lock_path(&home.paths())).unwrap();
    assert!(matches!(lock.try_lock(), Err(fs::TryLockError::WouldBlock)));
}

#[test]
fn d2_fifty_session_starts_at_once_leave_exactly_one_collector() {
    concurrent_starts_leave_one_collector(CI_STARTS);
}

/// The stress form of D2: `just stress-collector` runs it in the debug
/// profile, because the test-only stamp variable needs a debug build.
#[test]
#[ignore = "stress test: run with `just stress-collector`"]
fn d2_stress_a_thousand_session_starts_leave_one_collector() {
    concurrent_starts_leave_one_collector(STRESS_STARTS);
}

#[test]
fn d3_version_doctor_and_tool_hooks_start_nothing() {
    let home = Home::new();
    let _daemon = Daemon::new(&home);

    for args in [&["--version"][..], &["version"], &["doctor"]] {
        let mut cmd = home.cmd();
        cmd.args(args);
        let _ = cmd.output().unwrap();
    }
    home.hook(
        "preread-size-check",
        br#"{"session_id":"s1","tool_name":"Read","tool_input":{"file_path":"/nonexistent"}}"#,
    );
    home.hook(
        "post-edit-track",
        br#"{"session_id":"s1","tool_name":"Edit","tool_input":{"file_path":"/nonexistent"}}"#,
    );

    home.assert_untouched();
}

#[test]
fn d5_a_hung_collector_is_replaced_by_the_next_session_start() {
    let home = Home::new();
    let mut daemon = Daemon::new(&home);
    let hung = home.start_collector(&mut daemon, &[(TEST_HANG_VAR, "1")]);
    let began = Instant::now();

    home.session_start();
    daemon.reap();
    let replaced = within(D5_BOUND.saturating_sub(began.elapsed()), || {
        home.holder().is_some_and(|(pid, _)| pid != hung)
    });

    assert!(
        replaced,
        "not replaced within {D5_BOUND:?} (took {:?}); log: {}",
        began.elapsed(),
        home.log()
    );
}

#[test]
fn d6_a_command_hands_over_from_a_collector_of_another_version() {
    let home = Home::new();
    let mut daemon = Daemon::new(&home);
    home.start_collector(&mut daemon, &[(TEST_VERSION_VAR, OLD_VERSION)]);
    assert_eq!(home.holder().map(|h| h.1).as_deref(), Some(OLD_VERSION));

    let mut sessions = home.cmd();
    sessions.arg("sessions");
    home.run(sessions, b"");
    daemon.reap();

    assert!(
        within(UP_BOUND, || home
            .holder()
            .is_some_and(|(_, v)| v == VERSION)),
        "not handed over; log: {}",
        home.log()
    );
}

#[test]
fn headless_runs_start_nothing() {
    let home = Home::new();
    let _daemon = Daemon::new(&home);

    let mut cmd = home.cmd();
    cmd.args(["hook", "session-init"]).env("CI", "1");
    home.run(cmd, SESSION_START);

    home.assert_untouched();
}

#[test]
fn telemetry_off_starts_nothing() {
    let home = Home::bare();
    let _daemon = Daemon::new(&home);
    // Before the files that set telemetry up, so this command cannot start one.
    let mut off = home.cmd();
    off.args(["config", "set", "--global", "usage.telemetry", "off"]);
    home.run(off, b"");
    fs::create_dir_all(home.data()).unwrap();
    fs::write(
        home.data().join("collector.json"),
        format!(r#"{{"pid":0,"port":{},"token":"{TOKEN}"}}"#, home.port),
    )
    .unwrap();
    home.point_settings_here();

    home.session_start();

    home.assert_untouched();
}

#[test]
fn a_broken_or_partial_collector_record_starts_nothing() {
    let records = [
        |_port: u16| r#"{"pid":0,"port":"#.to_string(),
        |port: u16| format!(r#"{{"pid":0,"port":{port}}}"#),
        |_port: u16| format!(r#"{{"pid":0,"token":"{TOKEN}"}}"#),
    ];
    for record in records {
        let home = Home::with_state(record);
        let _daemon = Daemon::new(&home);

        home.session_start();

        home.assert_untouched();
    }
}

#[test]
fn settings_that_do_not_point_at_the_collector_start_nothing() {
    let home = Home::new();
    let _daemon = Daemon::new(&home);
    fs::write(home.home.join(".claude/settings.json"), "{}").unwrap();

    home.session_start();

    let paths = home.paths();
    assert_eq!(home.lock(), LockState::Free);
    assert!(!stamp_path(&paths).exists(), "a spawn was attempted");
}

#[test]
fn a_prompt_hook_with_a_running_collector_spawns_nothing_and_connects_nowhere() {
    let home = Home::new();
    let _daemon = Daemon::new(&home);
    // This test plays the running collector: it holds the lock at this
    // version and listens on the recorded port, counting connections.
    let lock_file = lock_path(&home.paths());
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&lock_file)
        .unwrap();
    lock.try_lock().unwrap();
    (&lock)
        .write_all(format!("{} {VERSION}\n", std::process::id()).as_bytes())
        .unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, home.port)).unwrap();
    listener.set_nonblocking(true).unwrap();

    home.hook("context-hygiene", PROMPT);
    home.hook("auto-model-detect", PROMPT);

    assert!(
        matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "the prompt hook connected to the collector"
    );
    assert!(!stamp_path(&home.paths()).exists(), "a spawn was attempted");
    drop(lock);
}

#[test]
fn a_prompt_hook_restarts_a_killed_collector() {
    let home = Home::new();
    let mut daemon = Daemon::new(&home);
    let old = home.start_collector(&mut daemon, &[]);
    assert!(daemon.kill_holder());
    daemon.reap();
    assert!(within(GONE_BOUND, || home.lock() == LockState::Free));

    home.hook("context-hygiene", PROMPT);

    assert!(
        within(UP_BOUND, || home
            .holder()
            .is_some_and(|(pid, _)| pid != old)),
        "no new collector; log: {}",
        home.log()
    );
}

#[test]
fn usage_stop_frees_the_lock_and_reports_not_running_after() {
    let home = Home::new();
    let mut daemon = Daemon::new(&home);
    home.start_collector(&mut daemon, &[]);

    let mut stop = home.cmd();
    stop.args(["usage", "stop"]);
    let first = home.run(stop, b"");
    daemon.reap();
    let mut again = home.cmd();
    again.args(["usage", "stop"]);
    let second = home.run(again, b"");

    assert_eq!(home.lock(), LockState::Free);
    assert!(
        String::from_utf8_lossy(&first.stdout).contains("stopped"),
        "{first:?}"
    );
    assert!(
        String::from_utf8_lossy(&second.stdout).contains("not running"),
        "{second:?}"
    );
}
