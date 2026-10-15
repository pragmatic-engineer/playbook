// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Points a `cc` session at the local usage receiver, for that session only.
//!
//! The variables go in the launcher's own environment, which the child
//! inherits, the same way `memory.source` sets its switch. Nothing is written:
//! Claude Code's settings files are only read, to find out whether the user
//! already configured telemetry, and no `--settings` flag is added.
//!
//! It acts only when all of these hold, else it sets nothing:
//!
//! - `usage.telemetry` is `auto` (the default; `off` turns it off),
//! - no telemetry variable is set in the process environment or in the `env`
//!   block of a Claude Code settings file (user, project, local, managed),
//! - a live `playbook usage collect` receiver is recorded in `collector.json`.
//!
//! When the first two hold, the usage database already exists (the user has
//! used the usage view) and no receiver runs, `cc` starts one itself as a
//! detached child (`playbook usage collect`, loopback only, owner-only state
//! file) and waits briefly for it. It stops by itself after ten minutes with
//! no traffic, and a second `cc` that starts at the same moment waits for the
//! first one's receiver instead of starting another (a short lock file). If
//! the receiver does not come up, the session starts without telemetry and
//! falls back to transcripts. Nothing outside playbook's own data directory is
//! written: no launchd or systemd file and no Claude Code setting.
//!
//! Prompt, tool detail, tool content and raw body logging are never turned on
//! and the account uuid is left out of the metrics.

use crate::usage::collector::{probe, read_state, running, state_path, Probe, State};
use crate::usage::run::Paths;
pub use crate::usage::telemetry_detect::{
    configured, files_configured, process_configured, Configured,
};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

/// The variables for a session pointed at `state`.
pub fn env_for(state: &State) -> Vec<(String, String)> {
    let pairs = [
        ("CLAUDE_CODE_ENABLE_TELEMETRY", "1".to_string()),
        ("OTEL_LOGS_EXPORTER", "otlp".to_string()),
        ("OTEL_METRICS_EXPORTER", "otlp".to_string()),
        ("OTEL_TRACES_EXPORTER", "none".to_string()),
        ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json".to_string()),
        (
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            format!("http://127.0.0.1:{}", state.port),
        ),
        (
            "OTEL_EXPORTER_OTLP_HEADERS",
            format!("Authorization=Bearer {}", state.token),
        ),
        ("OTEL_METRICS_INCLUDE_ACCOUNT_UUID", "false".to_string()),
    ];
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// The decision, with its inputs given so it can be tested.
pub fn decide(
    mode_auto: bool,
    process_configured: bool,
    files_configured: bool,
    collector: Option<&State>,
) -> Vec<(String, String)> {
    if !mode_auto || process_configured || files_configured {
        return Vec::new();
    }
    match collector {
        Some(state) if running(state) => env_for(state),
        _ => Vec::new(),
    }
}

const LOCK_STALE_SECONDS: u64 = 10;
const START_WAIT: Duration = Duration::from_millis(1500);

/// Spawns `cmd`, retrying a few times while the program file is busy. A binary
/// that was just written or replaced (an update, or a script in a test) can
/// report ETXTBSY for a moment while a concurrent fork still holds its write
/// handle, and one failed spawn would leave the session without telemetry.
fn spawn_retrying_busy(cmd: &mut std::process::Command) -> bool {
    for attempt in 0..10 {
        match cmd.spawn() {
            Ok(_) => return true,
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 9 => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return false,
        }
    }
    false
}

/// Where the collector's messages go: appended to `log` when it can be opened,
/// else dropped. Never the session's terminal.
fn log_stdio(log: Option<&Path>) -> std::process::Stdio {
    log.and_then(|p| {
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        options.open(p).ok()
    })
    .map_or_else(std::process::Stdio::null, std::process::Stdio::from)
}

/// Starts `exe usage collect` detached and waits for it to record itself in
/// `state_file`. Returns the state when it is live. `started` is set only
/// when this call started the process, so the note is printed once.
fn start_collector(
    state_file: &Path,
    exe: &Path,
    started: &mut bool,
    wait: Duration,
    log: Option<&Path>,
) -> Option<State> {
    let dir = state_file.parent()?;
    let lock = dir.join("collector.spawn");
    let lock_is_fresh = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() < LOCK_STALE_SECONDS)
    };
    let mut owner = false;
    for _ in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
        {
            Ok(_) => {
                owner = true;
                break;
            }
            Err(_) if lock_is_fresh(&lock) => break,
            Err(_) => {
                let _ = std::fs::remove_file(&lock);
            }
        }
    }
    if owner {
        let mut cmd = std::process::Command::new(exe);
        cmd.args(["usage", "collect"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(log_stdio(log));
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        *started = spawn_retrying_busy(&mut cmd);
    }
    let mut found = None;
    let deadline = std::time::Instant::now() + wait;
    loop {
        if let Some(state) = read_state(state_file).filter(running) {
            found = Some(state);
            break;
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // A lock whose collector has not shown up yet stays, and goes stale by
    // itself, so a session that starts a moment later does not spawn a second
    // one while the first is still coming up.
    if owner && (found.is_some() || !*started) {
        let _ = std::fs::remove_file(&lock);
    }
    found
}

/// The variables to set for a session started in `cwd`, empty when none.
pub fn session_env(home: &Path, cwd: &Path) -> Vec<(String, String)> {
    let mode_auto = matches!(
        crate::config::resolve_valid("usage.telemetry", home, None),
        Ok((Value::String(v), _, _)) if v == "auto"
    );
    if !mode_auto {
        return Vec::new();
    }
    let mut paths = Paths::from_home(home);
    paths.db = crate::common::paths::usage_db_dir().join("usage.db");
    let state_file = state_path(&paths);
    let mut collector = read_state(&state_file);
    let kind = configured(home, cwd, collector.as_ref().map(|s| s.port));
    // Settings that init wrote already point the session at the collector,
    // so only the collector needs to be up; nothing more is set here.
    let wanted = matches!(kind, Configured::Nothing | Configured::Playbook);
    // A listener that is not a playbook collector (a reused port) is down.
    let down = !collector
        .as_ref()
        .is_some_and(|s| running(s) && probe(s.port) == Probe::Collector);
    if wanted && down && state_file.with_file_name("usage.db").exists() {
        if let Ok(exe) = std::env::current_exe() {
            let mut started = false;
            collector =
                start_collector(&state_file, &exe, &mut started, START_WAIT, None).or(collector);
            if started && collector.is_some() {
                eprintln!(
                    "playbook: started the usage collector for telemetry (it stops after 10 idle minutes, see playbook usage status)"
                );
            }
        }
    }
    decide(
        mode_auto,
        kind != Configured::Nothing,
        false,
        collector.as_ref(),
    )
}

/// How long the SessionStart hook waits for a new collector to record itself.
/// It stays short because the session start must not feel it.
const SESSION_START_WAIT: Duration = Duration::from_millis(1200);

/// What the SessionStart hook found and did about the collector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ensured {
    /// Telemetry is not set up to use the playbook collector, or is switched
    /// off, so nothing was started.
    NotConfigured,
    /// A collector was already answering.
    Running,
    /// This call started one.
    Started,
    /// It was down and did not come up in time (see `collector.log`).
    Down,
}

/// Makes sure the collector runs when the Claude Code settings point at it.
/// One collector at a time: a fresh spawn lock means another session is
/// already starting it. Bounded by `wait`, never panics, and its messages go
/// to `collector.log` next to the state file.
pub fn ensure_collector(home: &Path, cwd: &Path, exe: &Path, wait: Duration) -> Ensured {
    let mode_auto = matches!(
        crate::config::resolve_valid("usage.telemetry", home, None),
        Ok((Value::String(v), _, _)) if v == "auto"
    );
    if !mode_auto {
        return Ensured::NotConfigured;
    }
    let state_file = state_path(&Paths::from_home(home));
    let Some(state) = read_state(&state_file).filter(|s| s.port != 0 && !s.token.is_empty()) else {
        return Ensured::NotConfigured;
    };
    if configured(home, cwd, Some(state.port)) != Configured::Playbook {
        return Ensured::NotConfigured;
    }
    if running(&state) && probe(state.port) == Probe::Collector {
        return Ensured::Running;
    }
    // Something else listens on the recorded port: a new collector could not
    // bind it, so do not spawn one or make the session wait. Doctor says why.
    if crate::usage::collector::port_answers(state.port) {
        return Ensured::Down;
    }
    let log = state_file.with_file_name("collector.log");
    let mut started = false;
    match start_collector(&state_file, exe, &mut started, wait, Some(&log)) {
        Some(_) if started => Ensured::Started,
        Some(_) => Ensured::Running,
        None => Ensured::Down,
    }
}

/// `ensure_collector` for the SessionStart hook: the real binary and the real
/// wait.
pub fn ensure_collector_for_session(home: &Path, cwd: &Path) -> Ensured {
    let Ok(exe) = std::env::current_exe() else {
        return Ensured::NotConfigured;
    };
    ensure_collector(home, cwd, &exe, SESSION_START_WAIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::path::PathBuf;

    /// Generous: the wait ends as soon as the collector is found, so it only
    /// costs time when the host is too loaded to start a shell in 1.5 seconds.
    const TEST_WAIT: Duration = Duration::from_secs(20);

    fn live() -> State {
        // Our own pid is alive; a bound listener makes the port answer.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::mem::forget(listener);
        State {
            pid: std::process::id(),
            port,
            token: "tok".into(),
            ..State::default()
        }
    }

    fn get<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn a_live_collector_and_no_user_config_sets_the_session_env() {
        let state = live();
        let env = decide(true, false, false, Some(&state));
        assert_eq!(get(&env, "CLAUDE_CODE_ENABLE_TELEMETRY"), Some("1"));
        assert_eq!(get(&env, "OTEL_EXPORTER_OTLP_PROTOCOL"), Some("http/json"));
        assert_eq!(
            get(&env, "OTEL_EXPORTER_OTLP_ENDPOINT"),
            Some(format!("http://127.0.0.1:{}", state.port).as_str())
        );
        assert_eq!(
            get(&env, "OTEL_EXPORTER_OTLP_HEADERS"),
            Some("Authorization=Bearer tok")
        );
        assert_eq!(
            get(&env, "OTEL_METRICS_INCLUDE_ACCOUNT_UUID"),
            Some("false")
        );
    }

    #[test]
    fn content_logging_is_never_turned_on() {
        let env = env_for(&live());
        for name in [
            "OTEL_LOG_USER_PROMPTS",
            "OTEL_LOG_TOOL_DETAILS",
            "OTEL_LOG_TOOL_CONTENT",
            "OTEL_LOG_RAW_API_BODIES",
        ] {
            assert!(get(&env, name).is_none(), "{name}");
        }
    }

    #[test]
    fn off_user_config_or_no_live_collector_sets_nothing() {
        let state = live();
        assert!(decide(false, false, false, Some(&state)).is_empty());
        assert!(decide(true, true, false, Some(&state)).is_empty());
        assert!(decide(true, false, true, Some(&state)).is_empty());
        assert!(decide(true, false, false, None).is_empty());
        let dead = State {
            pid: 0,
            port: state.port,
            token: "tok".into(),
            ..State::default()
        };
        assert!(decide(true, false, false, Some(&dead)).is_empty());
    }

    /// A fake `playbook` that records a live state (this test process and a
    /// bound port) the way `usage collect` does, and counts its runs.
    #[cfg(unix)]
    fn fake_exe(dir: &Path, port: u16) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let exe = dir.join("fake-playbook");
        let script = format!(
            "#!/bin/sh\necho \"$@\" >> '{d}/runs'\nprintf '{{\"pid\":{pid},\"port\":{port},\"token\":\"t\"}}' > '{d}/collector.json'\n",
            d = dir.display(),
            pid = std::process::id(),
        );
        std::fs::write(&exe, script).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        exe
    }

    #[cfg(unix)]
    fn listening() -> (std::net::TcpListener, u16) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        (l, port)
    }

    #[test]
    #[cfg(unix)]
    fn a_down_collector_is_started_detached_and_the_lock_is_released() {
        let dir = scratch_dir("cc-start");
        std::fs::create_dir_all(&dir).unwrap();
        let (_l, port) = listening();
        let exe = fake_exe(&dir, port);
        let mut started = false;
        let state = start_collector(
            &dir.join("collector.json"),
            &exe,
            &mut started,
            TEST_WAIT,
            None,
        )
        .unwrap();
        assert!(started);
        assert_eq!((state.port, state.token.as_str()), (port, "t"));
        assert_eq!(
            std::fs::read_to_string(dir.join("runs")).unwrap().trim(),
            "usage collect"
        );
        assert!(!dir.join("collector.spawn").exists());
    }

    #[test]
    #[cfg(unix)]
    fn a_fresh_lock_means_another_cc_is_starting_one_so_nothing_is_spawned() {
        let dir = scratch_dir("cc-lock");
        std::fs::create_dir_all(&dir).unwrap();
        let (_l, port) = listening();
        let exe = fake_exe(&dir, port);
        std::fs::write(dir.join("collector.spawn"), "").unwrap();
        let mut started = false;
        assert!(start_collector(
            &dir.join("collector.json"),
            &exe,
            &mut started,
            Duration::from_millis(300),
            None
        )
        .is_none());
        assert!(!started);
        assert!(!dir.join("runs").exists());
        assert!(dir.join("collector.spawn").exists(), "not ours to remove");
    }

    #[test]
    #[cfg(unix)]
    fn a_stale_lock_is_replaced() {
        let dir = scratch_dir("cc-stale");
        std::fs::create_dir_all(&dir).unwrap();
        let (_l, port) = listening();
        let exe = fake_exe(&dir, port);
        let lock = std::fs::File::create(dir.join("collector.spawn")).unwrap();
        lock.set_modified(std::time::SystemTime::now() - Duration::from_secs(60))
            .unwrap();
        let mut started = false;
        assert!(start_collector(
            &dir.join("collector.json"),
            &exe,
            &mut started,
            TEST_WAIT,
            None
        )
        .is_some());
        assert!(started);
    }

    #[test]
    fn a_missing_program_leaves_the_session_without_telemetry() {
        let dir = scratch_dir("cc-missing");
        std::fs::create_dir_all(&dir).unwrap();
        let mut started = false;
        let state = start_collector(
            &dir.join("collector.json"),
            &dir.join("no-such-program"),
            &mut started,
            Duration::from_millis(100),
            None,
        );
        assert!(state.is_none() && !started);
        assert!(!dir.join("collector.spawn").exists());
    }

    /// A scratch home whose settings name the playbook collector on `port`,
    /// with a state file for it, plus the folder the state file lives in.
    #[cfg(unix)]
    fn configured_home(name: &str, port: u16, pid: u32) -> (PathBuf, PathBuf) {
        crate::usage_host::install();
        let home = scratch_dir(name);
        let state_file = state_path(&Paths::from_home(&home));
        let data = state_file.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(
            &state_file,
            format!("{{\"pid\":{pid},\"port\":{port},\"token\":\"t\"}}"),
        )
        .unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(
            home.join(".claude/settings.json"),
            format!(
                "{{\"env\":{{\"CLAUDE_CODE_ENABLE_TELEMETRY\":\"1\",\"OTEL_EXPORTER_OTLP_ENDPOINT\":\"http://127.0.0.1:{port}\"}}}}"
            ),
        )
        .unwrap();
        (home, data)
    }

    #[test]
    #[cfg(unix)]
    fn the_session_hook_starts_a_down_collector_once_the_settings_point_at_it() {
        let (_l, port) = listening();
        let unused = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        // The state names a port nobody listens on; the fake collector then
        // records the live one, as a real start would.
        let (home, data) = configured_home("cc-ensure-start", unused, 0);
        let exe = fake_exe(&data, port);

        let got = ensure_collector(&home, &home, &exe, TEST_WAIT);

        assert_eq!(got, Ensured::Started);
        assert_eq!(
            std::fs::read_to_string(data.join("runs")).unwrap().trim(),
            "usage collect"
        );
        assert!(data.join("collector.log").is_file());
    }

    #[test]
    #[cfg(unix)]
    fn a_second_session_at_the_same_moment_does_not_start_a_second_collector() {
        let (_l, port) = listening();
        let (home, data) = configured_home("cc-ensure-parallel", port, 0);
        let exe = fake_exe(&data, port);
        std::fs::write(data.join("collector.spawn"), "").unwrap();

        let got = ensure_collector(&home, &home, &exe, Duration::from_millis(300));

        assert_eq!(got, Ensured::Down);
        assert!(!data.join("runs").exists());
    }

    #[test]
    #[cfg(unix)]
    fn nothing_is_started_when_telemetry_is_not_pointed_at_the_collector() {
        let (_l, port) = listening();
        let (home, data) = configured_home("cc-ensure-other", port, 0);
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"env":{"CLAUDE_CODE_ENABLE_TELEMETRY":"1","OTEL_EXPORTER_OTLP_ENDPOINT":"https://mine.example"}}"#,
        )
        .unwrap();
        let exe = fake_exe(&data, port);

        let got = ensure_collector(&home, &home, &exe, Duration::from_millis(300));

        assert_eq!(got, Ensured::NotConfigured);
        assert!(!data.join("runs").exists());
    }

    #[test]
    #[cfg(unix)]
    fn nothing_is_started_before_init_has_recorded_a_collector() {
        crate::usage_host::install();
        let home = scratch_dir("cc-ensure-fresh");
        let exe = home.join("no-such-program");

        let got = ensure_collector(&home, &home, &exe, Duration::from_millis(300));

        assert_eq!(got, Ensured::NotConfigured);
    }

    #[test]
    #[cfg(unix)]
    fn a_program_that_cannot_start_is_reported_down_and_never_panics() {
        let (home, _data) = configured_home("cc-ensure-broken", 1, 0);
        let exe = home.join("no-such-program");

        let got = ensure_collector(&home, &home, &exe, Duration::from_millis(200));

        assert_eq!(got, Ensured::Down);
    }

    #[test]
    #[cfg(unix)]
    fn a_foreign_listener_on_the_recorded_port_spawns_nothing_and_does_not_wait() {
        let (_holder, port) = listening();
        let (home, data) = configured_home("cc-ensure-foreign", port, 0);
        let exe = fake_exe(&data, port);
        let began = std::time::Instant::now();

        let got = ensure_collector(&home, &home, &exe, Duration::from_secs(30));

        assert_eq!(got, Ensured::Down);
        assert!(!data.join("runs").exists());
        assert!(began.elapsed() < Duration::from_secs(5));
    }

    #[test]
    #[cfg(unix)]
    fn a_lock_stays_when_the_collector_has_not_shown_up_yet() {
        let dir = scratch_dir("cc-lock-stays");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("slow-playbook");
        std::fs::write(&exe, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&exe, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let mut started = false;

        let state = start_collector(
            &dir.join("collector.json"),
            &exe,
            &mut started,
            Duration::from_millis(200),
            None,
        );

        assert!(state.is_none() && started);
        assert!(dir.join("collector.spawn").exists());
    }
}
