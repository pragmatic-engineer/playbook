// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The SessionStart hook keeps the telemetry collector running for plain
//! `claude` sessions, end to end with the real binary in a scratch HOME.

use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Home {
        let dir = std::env::temp_dir().join(format!(
            "pb-session-collector-{tag}-{}",
            playbook::testing::run_id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".claude")).unwrap();
        Home(dir)
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_playbook"));
        cmd.env("HOME", &self.0)
            .env("SHELL", "/bin/zsh")
            .env("CLAUDE_PLUGIN_ROOT", env!("PLAYBOOK_REPO_ROOT"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .current_dir(&self.0);
        for (name, _) in std::env::vars() {
            if name == "CLAUDE_CODE_ENABLE_TELEMETRY" || name.starts_with("OTEL_") {
                cmd.env_remove(name);
            }
        }
        cmd
    }

    /// Runs `init --only telemetry` with a port that is free now. Init would
    /// pick the shared default 47318 for every test that runs at the same
    /// moment, and only one collector can hold it, so the others never come
    /// up. The port is not held, so another process could still take it, but
    /// that is rare next to the shared default.
    fn init_telemetry(&self) -> std::process::Output {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let dir = self.0.join(".config/playbook/data");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("collector.json"),
            format!(r#"{{"pid":0,"port":{port},"token":"test-token"}}"#),
        )
        .unwrap();
        let out = self
            .cmd()
            .args(["init", "--only", "telemetry"])
            .output()
            .unwrap();
        assert_eq!(
            self.state().map(|s| s["port"].clone()),
            Some(Value::from(port)),
            "init must keep the port chosen for this test"
        );
        out
    }

    fn session_start(&self) {
        let mut child = self
            .cmd()
            .args(["hook", "session-init"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"session_id":"s1","source":"startup"}"#)
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            "",
            "the hook is quiet"
        );
    }

    fn state(&self) -> Option<Value> {
        let text = fs::read_to_string(self.0.join(".config/playbook/data/collector.json")).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn wait_for_live_collector(&self) -> Option<(u32, u16)> {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Some(s) = self.state() {
                let pid = u32::try_from(s["pid"].as_u64().unwrap_or(0)).unwrap_or(0);
                let port = u16::try_from(s["port"].as_u64().unwrap_or(0)).unwrap_or(0);
                if pid != 0 && std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                    return Some((pid, port));
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        if let Some(pid) = self
            .state()
            .and_then(|s| s["pid"].as_u64())
            .filter(|p| *p != 0)
        {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_session_start_brings_up_the_collector_and_a_second_one_reuses_it() {
    let home = Home::new("start");
    let init = home.init_telemetry();
    assert!(init.status.success());

    home.session_start();
    let first = home.wait_for_live_collector();
    home.session_start();
    let second = home.wait_for_live_collector();

    assert!(first.is_some(), "no collector came up");
    assert_eq!(first, second, "a second session must reuse the collector");
}

#[test]
fn a_session_start_without_telemetry_set_up_starts_nothing() {
    let home = Home::new("none");

    home.session_start();

    assert!(home.state().is_none());
}

#[test]
fn two_sessions_starting_at_once_bring_up_one_collector() {
    let home = Home::new("race");
    let init = home.init_telemetry();
    assert!(init.status.success());
    let spawn = |home: &Home| {
        home.cmd()
            .args(["hook", "session-init"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };
    let mut a = spawn(&home);
    let mut b = spawn(&home);
    for child in [&mut a, &mut b] {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"session_id":"s","source":"startup"}"#)
            .unwrap();
    }

    let _ = a.wait();
    let _ = b.wait();
    let live = home.wait_for_live_collector();

    let log =
        fs::read_to_string(home.0.join(".config/playbook/data/collector.log")).unwrap_or_default();
    assert!(live.is_some(), "no collector came up");
    assert_eq!(log.matches("listening on").count(), 1, "{log}");
}
