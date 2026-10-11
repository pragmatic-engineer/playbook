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
//! Prompt, tool detail, tool content and raw body logging are never turned on
//! and the account uuid is left out of the metrics.

use crate::usage::collector::{read_state, running, state_path, State};
use crate::usage::run::Paths;
use serde_json::Value;
use std::path::{Path, PathBuf};

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

/// The variables to set for a session started in `cwd`, empty when none.
pub fn session_env(home: &Path, cwd: &Path) -> Vec<(String, String)> {
    let mode_auto = matches!(
        crate::config::resolve_valid("usage.telemetry", home, None),
        Ok((Value::String(v), _, _)) if v == "auto"
    );
    if !mode_auto {
        return Vec::new();
    }
    let process_configured = std::env::vars_os()
        .filter_map(|(k, _)| k.into_string().ok())
        .any(|k| is_telemetry_var(&k));
    let files_configured = configured_in_files(&settings_files(home, cwd));
    let mut paths = Paths::from_home(home);
    paths.db = crate::common::paths::usage_db_dir().join("usage.db");
    let collector = read_state(&state_path(&paths));
    decide(
        mode_auto,
        process_configured,
        files_configured,
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
}
