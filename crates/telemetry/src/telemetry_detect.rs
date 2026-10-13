// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Reads what the user's own Claude Code configuration says about telemetry
//! (process environment and settings files). Read only.

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

/// The merged telemetry variables, `None` when a settings file cannot be
/// parsed. `with_process` includes the process environment.
pub fn merged_vars(
    home: &Path,
    cwd: &Path,
    with_process: bool,
) -> Option<std::collections::BTreeMap<String, String>> {
    let process: Vec<(String, String)> = if with_process {
        std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .collect()
    } else {
        Vec::new()
    };
    collect_vars(process.into_iter(), &settings_files(home, cwd)).ok()
}

/// Whether any settings file other than `except` names a telemetry variable
/// (an unreadable file counts as naming one).
pub fn files_configured_except(home: &Path, cwd: &Path, except: &Path) -> bool {
    let files: Vec<PathBuf> = settings_files(home, cwd)
        .into_iter()
        .filter(|f| f != except)
        .collect();
    configured_in_files(&files)
}

/// Like [`configured`] but reading only the settings files, not the process
/// environment.
pub fn configured_from_files(home: &Path, cwd: &Path, ours: Option<u16>) -> Configured {
    match collect_vars(std::iter::empty(), &settings_files(home, cwd)) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::scratch_dir;
    use serde_json::json;

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
}
