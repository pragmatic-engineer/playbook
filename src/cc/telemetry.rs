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

use crate::usage::collector::{read_state, running, state_path, State};
use crate::usage::run::Paths;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A variable that means the user configured telemetry themselves.
fn is_telemetry_var(name: &str) -> bool {
    name == "CLAUDE_CODE_ENABLE_TELEMETRY" || name.starts_with("OTEL_")
}

/// Whether the `env` block of a settings document names a telemetry variable.
fn settings_configure_telemetry(settings: &Value) -> bool {
    settings
        .get("env")
        .and_then(Value::as_object)
        .is_some_and(|env| env.keys().any(|k| is_telemetry_var(k)))
}

/// The settings files that can carry an `env` block, lowest precedence first.
fn settings_files(home: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut files = vec![
        home.join(".claude/settings.json"),
        cwd.join(".claude/settings.json"),
        cwd.join(".claude/settings.local.json"),
    ];
    files.push(PathBuf::from(
        "/Library/Application Support/ClaudeCode/managed-settings.json",
    ));
    files.push(PathBuf::from("/etc/claude-code/managed-settings.json"));
    files
}

/// Whether telemetry is configured by the user in any file. A file that
/// exists but cannot be parsed counts as configured, so we never override
/// something we could not read.
fn configured_in_files(files: &[PathBuf]) -> bool {
    files.iter().any(|f| match std::fs::read(f) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
            .map(|v| settings_configure_telemetry(&v))
            .unwrap_or(true),
        Err(_) => false,
    })
}

/// What the user's own Claude Code configuration says about telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Configured {
    /// No telemetry variable anywhere.
    Nothing,
    /// `CLAUDE_CODE_ENABLE_TELEMETRY` is set to an off value.
    Disabled,
    /// Telemetry is on and points at the local collector port.
    Playbook,
    /// Telemetry is set up some other way (another endpoint, or unreadable).
    Other,
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Classifies the merged telemetry variables, `ours` being the collector port.
pub fn classify(
    vars: &std::collections::BTreeMap<String, String>,
    ours: Option<u16>,
) -> Configured {
    if vars.is_empty() {
        return Configured::Nothing;
    }
    let enable = vars.get("CLAUDE_CODE_ENABLE_TELEMETRY");
    if let Some(v) = enable {
        let v = v.trim().to_ascii_lowercase();
        if v.is_empty() || v == "0" || v == "false" {
            return Configured::Disabled;
        }
    }
    let endpoint = vars
        .get("OTEL_EXPORTER_OTLP_ENDPOINT")
        .map(|e| e.trim_end_matches('/').to_string());
    let local = ours.is_some_and(|port| {
        endpoint.as_deref().is_some_and(|e| {
            e == format!("http://127.0.0.1:{port}") || e == format!("http://localhost:{port}")
        })
    });
    if enable.is_some() && local {
        Configured::Playbook
    } else {
        Configured::Other
    }
}

/// The telemetry variables from the process environment, then each settings
/// file in turn (a later file wins). `Err(())` when a file cannot be parsed.
fn collect_vars(
    process: impl Iterator<Item = (String, String)>,
    files: &[PathBuf],
) -> Result<std::collections::BTreeMap<String, String>, ()> {
    let mut vars: std::collections::BTreeMap<String, String> =
        process.filter(|(k, _)| is_telemetry_var(k)).collect();
    for f in files {
        let Ok(bytes) = std::fs::read(f) else {
            continue;
        };
        let doc: Value = serde_json::from_slice(&bytes).map_err(|_| ())?;
        if let Some(env) = doc.get("env").and_then(Value::as_object) {
            for (k, v) in env.iter().filter(|(k, _)| is_telemetry_var(k)) {
                vars.insert(k.clone(), text_of(v));
            }
        }
    }
    Ok(vars)
}

/// What the user configured, reading the environment and the settings files.
pub fn configured(home: &Path, cwd: &Path, ours: Option<u16>) -> Configured {
    let process = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)));
    match collect_vars(process, &settings_files(home, cwd)) {
        Ok(vars) => classify(&vars, ours),
        Err(()) => Configured::Other,
    }
}

/// Whether the process environment names a telemetry variable.
pub fn process_configured() -> bool {
    std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .any(|k| is_telemetry_var(&k))
}

/// Whether any Claude Code settings file (user, project, local, managed)
/// names a telemetry variable. An unreadable file counts as configured.
pub fn files_configured(home: &Path, cwd: &Path) -> bool {
    configured_in_files(&settings_files(home, cwd))
}

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
const START_WAIT_MS: u64 = 1500;

/// Starts `exe usage collect` detached and waits for it to record itself in
/// `state_file`. Returns the state when it is live. `started` is set only
/// when this call started the process, so the note is printed once.
fn start_collector(state_file: &Path, exe: &Path, started: &mut bool) -> Option<State> {
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
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        *started = cmd.spawn().is_ok();
    }
    let mut found = None;
    for _ in 0..(START_WAIT_MS / 50) {
        if let Some(state) = read_state(state_file).filter(running) {
            found = Some(state);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if owner {
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
    let down = !collector.as_ref().is_some_and(running);
    if wanted && down && state_file.with_file_name("usage.db").exists() {
        if let Ok(exe) = std::env::current_exe() {
            let mut started = false;
            collector = start_collector(&state_file, &exe, &mut started).or(collector);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use serde_json::json;

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

    fn vars(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn classify_tells_disabled_ours_and_someone_elses_apart() {
        assert_eq!(classify(&vars(&[]), Some(4318)), Configured::Nothing);
        let off = vars(&[("CLAUDE_CODE_ENABLE_TELEMETRY", "0")]);
        assert_eq!(classify(&off, Some(4318)), Configured::Disabled);
        let ours = vars(&[
            ("CLAUDE_CODE_ENABLE_TELEMETRY", "1"),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:4318"),
        ]);
        assert_eq!(classify(&ours, Some(4318)), Configured::Playbook);
        assert_eq!(classify(&ours, Some(5000)), Configured::Other);
        let theirs = vars(&[
            ("CLAUDE_CODE_ENABLE_TELEMETRY", "1"),
            (
                "OTEL_EXPORTER_OTLP_ENDPOINT",
                "https://collector.example.test",
            ),
        ]);
        assert_eq!(classify(&theirs, Some(4318)), Configured::Other);
    }

    #[test]
    fn an_unparsable_settings_file_is_treated_as_the_users_own_setup() {
        let dir = scratch_dir("cc-telemetry-broken-kind");
        std::fs::create_dir_all(&dir).unwrap();
        let broken = dir.join("settings.json");
        std::fs::write(&broken, "{").unwrap();
        assert!(collect_vars(std::iter::empty(), &[broken]).is_err());
    }

    #[test]
    fn an_env_block_naming_a_telemetry_variable_counts_as_configured() {
        for key in ["CLAUDE_CODE_ENABLE_TELEMETRY", "OTEL_LOGS_EXPORTER"] {
            let doc = json!({ "env": { key: "0" } });
            assert!(settings_configure_telemetry(&doc), "{key}");
        }
        assert!(!settings_configure_telemetry(
            &json!({ "env": { "FOO": "1" } })
        ));
        assert!(!settings_configure_telemetry(&json!({ "model": "opus" })));
    }

    #[test]
    fn files_are_read_never_written_and_a_broken_file_counts_as_configured() {
        let dir = scratch_dir("cc-telemetry-files");
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good.json");
        let pinned = dir.join("pinned.json");
        let broken = dir.join("broken.json");
        std::fs::write(&good, r#"{"env":{"FOO":"1"}}"#).unwrap();
        std::fs::write(&pinned, r#"{"env":{"CLAUDE_CODE_ENABLE_TELEMETRY":"0"}}"#).unwrap();
        std::fs::write(&broken, "{").unwrap();
        let missing = dir.join("missing.json");
        assert!(!configured_in_files(&[good.clone(), missing]));
        assert!(configured_in_files(&[good.clone(), pinned.clone()]));
        assert!(configured_in_files(&[broken]));
        assert_eq!(
            std::fs::read_to_string(&pinned).unwrap(),
            r#"{"env":{"CLAUDE_CODE_ENABLE_TELEMETRY":"0"}}"#
        );
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
        let state = start_collector(&dir.join("collector.json"), &exe, &mut started).unwrap();
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
        assert!(start_collector(&dir.join("collector.json"), &exe, &mut started).is_none());
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
        assert!(start_collector(&dir.join("collector.json"), &exe, &mut started).is_some());
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
        );
        assert!(state.is_none() && !started);
        assert!(!dir.join("collector.spawn").exists());
    }
}
