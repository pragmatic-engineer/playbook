// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `telemetry` part of `playbook init`: with consent, writes the
//! OpenTelemetry variables that point Claude Code at the local usage
//! collector into the `env` block of `settings.json`, so plain `claude`, the
//! IDE and the desktop app send telemetry too, not only `cc`.
//!
//! It never overrides: when the environment or any Claude Code settings file
//! already names a telemetry variable, nothing is written. What it writes is
//! recorded in `~/.config/playbook/telemetry-env.json`, and `uninstall`
//! removes exactly those keys, and only while they still hold the value
//! written. Prompt, tool detail, tool content and raw body logging are never
//! turned on, and the account uuid stays out of the metrics.

use crate::init::run::StepReport;
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

fn marker_path(home: &Path) -> PathBuf {
    crate::common::paths::playbook_root_from(home).join("telemetry-env.json")
}

fn load(path: &Path) -> Result<Map<String, Value>, String> {
    if !path.is_file() {
        return Ok(Map::new());
    }
    let text =
        fs::read_to_string(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(format!("{} is not a JSON object", path.display())),
    }
}

/// Copies `path` to `.bak.<epoch>` (kept when one exists) and writes `root`.
fn write_with_backup(path: &Path, root: Map<String, Value>, epoch: u64) -> Result<(), String> {
    if path.is_file() {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let backup = path.with_file_name(format!("{name}.bak.{epoch}"));
        if !backup.exists() {
            fs::copy(path, &backup)
                .map_err(|e| format!("could not back up {}: {e}", path.display()))?;
        }
    }
    crate::common::atomic::write_atomic(path, format!("{:#}\n", Value::Object(root)))
        .map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Whether the record exists and settings.json still holds every value in it.
fn already_written(home: &Path, settings_path: &Path) -> bool {
    let Ok(written) = load(&marker_path(home)) else {
        return false;
    };
    if written.is_empty() {
        return false;
    }
    let Ok(root) = load(settings_path) else {
        return false;
    };
    let env = root.get("env").and_then(Value::as_object);
    written
        .iter()
        .all(|(k, v)| env.is_some_and(|e| e.get(k) == Some(v)))
}

/// Whether Claude Code telemetry is on, off or not mentioned, as the process
/// environment and every Claude Code settings file read together say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Telemetry variables are set and not switched off.
    Enabled,
    /// `CLAUDE_CODE_ENABLE_TELEMETRY` is set to an off value.
    Disabled,
    /// No telemetry variable anywhere.
    Unset,
}

/// The current telemetry state. `cwd` is where project settings are looked up.
/// `with_process` also reads the process environment; tests pass false so the
/// machine they run on cannot change the answer.
pub fn state(home: &Path, cwd: &Path, with_process: bool) -> State {
    use crate::cc::telemetry::Configured;
    let found = if with_process {
        crate::cc::telemetry::configured(home, cwd, None)
    } else {
        crate::usage::telemetry_detect::configured_from_files(home, cwd, None)
    };
    match found {
        Configured::Nothing => State::Unset,
        Configured::Disabled => State::Disabled,
        Configured::Playbook | Configured::Other => State::Enabled,
    }
}

/// The one line init prints when telemetry is already on and so asks nothing.
pub const ENABLED_LINE: &str =
    "telemetry: already enabled in your environment or Claude Code settings, so nothing to ask";

/// Whether the only telemetry variable in `settings_path`'s env block is an
/// off `CLAUDE_CODE_ENABLE_TELEMETRY`, the one case where a yes may replace a
/// value the user set (it is the exact thing they were asked about).
fn only_an_off_switch(settings_path: &Path) -> bool {
    let Ok(root) = load(settings_path) else {
        return false;
    };
    let Some(env) = root.get("env").and_then(Value::as_object) else {
        return false;
    };
    let off = env.get("CLAUDE_CODE_ENABLE_TELEMETRY").is_some_and(|v| {
        matches!(
            v.as_str().map(|s| s.trim().to_ascii_lowercase()).as_deref(),
            Some("" | "0" | "false")
        )
    });
    off && env
        .keys()
        .filter(|k| *k == "CLAUDE_CODE_ENABLE_TELEMETRY" || k.starts_with("OTEL_"))
        .count()
        == 1
}

/// The init step. `cwd` is where project settings are looked up.
pub fn apply(
    home: &Path,
    claude_home: &Path,
    cwd: &Path,
    epoch: u64,
    process_configured: bool,
) -> StepReport {
    let off = matches!(
        crate::config::resolve_valid("usage.telemetry", home, None),
        Ok((Value::String(v), _, _)) if v == "off"
    );
    if off {
        return StepReport::skipped("telemetry", "usage.telemetry is off, so nothing is written");
    }
    if already_written(home, &claude_home.join("settings.json")) {
        return StepReport::already_correct(
            "telemetry",
            "the variables init wrote are still in place",
        );
    }
    // A yes to "turn it on" may replace an off switch that lives alone in the
    // user's own settings.json. Any other existing setup is left as it is.
    let switch_off_here = !process_configured
        && state(home, cwd, false) == State::Disabled
        && only_an_off_switch(&claude_home.join("settings.json"));
    if !switch_off_here && (process_configured || crate::cc::telemetry::files_configured(home, cwd))
    {
        return StepReport::skipped(
            "telemetry",
            "you already have telemetry configured, left as it is (`playbook usage status` shows how it is read)",
        );
    }
    let paths = crate::usage::run::Paths::from_home(home);
    let state = match crate::usage::collector::ensure_state(&paths) {
        Ok(s) => s,
        Err(e) => return StepReport::failed("telemetry", e),
    };
    let settings_path = claude_home.join("settings.json");
    let mut root = match load(&settings_path) {
        Ok(r) => r,
        Err(e) => return StepReport::failed("telemetry", e),
    };
    let vars = crate::cc::telemetry::env_for(&state);
    let env = root
        .entry("env")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(env) = env.as_object_mut() else {
        return StepReport::skipped(
            "telemetry",
            "the env entry in settings.json is not an object, left as it is",
        );
    };
    let mut written = Map::new();
    for (k, v) in vars {
        env.insert(k.clone(), Value::String(v.clone()));
        written.insert(k, Value::String(v));
    }
    if let Err(e) = write_with_backup(&settings_path, root, epoch) {
        return StepReport::failed("telemetry", e);
    }
    let marker = marker_path(home);
    if let Err(e) = crate::common::atomic::write_atomic_private(
        &marker,
        format!("{:#}\n", Value::Object(written)),
    ) {
        return StepReport::failed(
            "telemetry",
            format!("could not write {}: {e}", marker.display()),
        );
    }
    StepReport::wired(
        "telemetry",
        format!(
            "wrote the OpenTelemetry variables for the local collector (127.0.0.1:{}); no prompt text or tool content is sent",
            state.port
        ),
    )
}

/// What `remove` did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Removed {
    pub keys: usize,
    pub backup: Option<PathBuf>,
}

/// Removes the keys `apply` wrote, those still holding the value written.
/// `dry_run` only counts.
pub fn remove(
    home: &Path,
    settings_path: &Path,
    epoch: u64,
    dry_run: bool,
) -> Result<Removed, String> {
    let marker = marker_path(home);
    let written = load(&marker)?;
    if written.is_empty() {
        return Ok(Removed::default());
    }
    let mut root = load(settings_path)?;
    let mut keys = 0;
    let mut emptied = false;
    if let Some(env) = root.get_mut("env").and_then(Value::as_object_mut) {
        for (k, v) in &written {
            if env.get(k) == Some(v) {
                env.remove(k);
                keys += 1;
            }
        }
        emptied = keys > 0 && env.is_empty();
    }
    if emptied {
        root.remove("env");
    }
    let mut out = Removed { keys, backup: None };
    if dry_run {
        return Ok(out);
    }
    if keys > 0 {
        write_with_backup(settings_path, root, epoch)?;
        let name = settings_path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        out.backup = Some(settings_path.with_file_name(format!("{name}.bak.{epoch}")));
    }
    let _ = fs::remove_file(&marker);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use serde_json::json;

    fn setup(name: &str, settings: &str) -> (PathBuf, PathBuf, PathBuf) {
        crate::usage_host::install();
        let home = scratch_dir(name);
        let claude = home.join(".claude");
        fs::create_dir_all(&claude).unwrap();
        fs::write(claude.join("settings.json"), settings).unwrap();
        let cwd = home.join("project");
        fs::create_dir_all(&cwd).unwrap();
        (home, claude, cwd)
    }

    fn env_of(claude: &Path) -> Map<String, Value> {
        let doc: Value =
            serde_json::from_str(&fs::read_to_string(claude.join("settings.json")).unwrap())
                .unwrap();
        doc.get("env")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn with_no_telemetry_configured_the_env_keys_are_written_and_recorded() {
        let (home, claude, cwd) = setup("init-tel-write", r#"{"model":"opus","env":{"FOO":"1"}}"#);
        let step = apply(&home, &claude, &cwd, 100, false);
        let env = env_of(&claude);
        assert_eq!(step.status, crate::init::run::StepStatus::Wired);
        assert_eq!(env["CLAUDE_CODE_ENABLE_TELEMETRY"], "1");
        assert_eq!(env["OTEL_METRICS_INCLUDE_ACCOUNT_UUID"], "false");
        assert_eq!(env["FOO"], "1");
        assert!(marker_path(&home).is_file());
        assert!(claude.join("settings.json.bak.100").is_file());
    }

    #[test]
    fn state_reads_unset_disabled_and_enabled_from_the_settings_files() {
        let (home, _claude, cwd) = setup("init-tel-state-unset", "{}");
        assert_eq!(state(&home, &cwd, false), State::Unset);
        let (home, _claude, cwd) = setup(
            "init-tel-state-off",
            r#"{"env":{"CLAUDE_CODE_ENABLE_TELEMETRY":"0"}}"#,
        );
        assert_eq!(state(&home, &cwd, false), State::Disabled);
        let (home, _claude, cwd) = setup(
            "init-tel-state-on",
            r#"{"env":{"CLAUDE_CODE_ENABLE_TELEMETRY":"1"}}"#,
        );
        assert_eq!(state(&home, &cwd, false), State::Enabled);
    }

    #[test]
    fn a_yes_replaces_an_off_switch_that_stands_alone_in_user_settings() {
        let (home, claude, cwd) = setup(
            "init-tel-flip",
            r#"{"env":{"CLAUDE_CODE_ENABLE_TELEMETRY":"0","FOO":"1"}}"#,
        );
        let step = apply(&home, &claude, &cwd, 5, false);
        let env = env_of(&claude);
        assert_eq!(step.status, crate::init::run::StepStatus::Wired);
        assert_eq!(env["CLAUDE_CODE_ENABLE_TELEMETRY"], "1");
        assert_eq!(env["FOO"], "1");
    }

    #[test]
    fn a_second_run_reports_no_change_instead_of_calling_it_your_setup() {
        let (home, claude, cwd) = setup("init-tel-twice", "{}");
        apply(&home, &claude, &cwd, 1, false);
        let again = apply(&home, &claude, &cwd, 2, false);
        assert_eq!(again.status, crate::init::run::StepStatus::AlreadyCorrect);
    }

    #[test]
    fn the_config_key_off_writes_nothing() {
        let (home, claude, cwd) = setup("init-tel-cfgoff", "{}");
        let cfg = home.join(".config/playbook");
        fs::create_dir_all(&cfg).unwrap();
        fs::write(cfg.join("config.json"), r#"{"usage":{"telemetry":"off"}}"#).unwrap();
        let step = apply(&home, &claude, &cwd, 1, false);
        assert_eq!(step.status, crate::init::run::StepStatus::Skipped);
        assert!(env_of(&claude).is_empty());
    }

    #[test]
    fn content_logging_variables_are_never_written() {
        let (home, claude, cwd) = setup("init-tel-privacy", "{}");
        apply(&home, &claude, &cwd, 1, false);
        let env = env_of(&claude);
        for name in [
            "OTEL_LOG_USER_PROMPTS",
            "OTEL_LOG_TOOL_DETAILS",
            "OTEL_LOG_TOOL_CONTENT",
            "OTEL_LOG_RAW_API_BODIES",
        ] {
            assert!(!env.contains_key(name), "{name}");
        }
    }

    #[test]
    fn an_existing_telemetry_setting_is_never_overridden() {
        let original = r#"{"env":{"CLAUDE_CODE_ENABLE_TELEMETRY":"0","OTEL_EXPORTER_OTLP_ENDPOINT":"https://x"}}"#;
        let (home, claude, cwd) = setup("init-tel-keep", original);
        let step = apply(&home, &claude, &cwd, 1, false);
        assert_eq!(step.status, crate::init::run::StepStatus::Skipped);
        assert_eq!(
            fs::read_to_string(claude.join("settings.json")).unwrap(),
            original
        );
        assert!(!marker_path(&home).exists());
    }

    #[test]
    fn a_telemetry_variable_in_the_environment_counts_as_configured() {
        let (home, claude, cwd) = setup("init-tel-envvar", "{}");
        let step = apply(&home, &claude, &cwd, 1, true);
        assert_eq!(step.status, crate::init::run::StepStatus::Skipped);
        assert!(env_of(&claude).is_empty());
    }

    #[test]
    fn a_project_setting_counts_as_configured() {
        let (home, claude, cwd) = setup("init-tel-project", "{}");
        fs::create_dir_all(cwd.join(".claude")).unwrap();
        fs::write(
            cwd.join(".claude/settings.json"),
            r#"{"env":{"OTEL_LOGS_EXPORTER":"console"}}"#,
        )
        .unwrap();
        let step = apply(&home, &claude, &cwd, 1, false);
        assert_eq!(step.status, crate::init::run::StepStatus::Skipped);
        assert!(env_of(&claude).is_empty());
    }

    #[test]
    fn remove_takes_out_exactly_what_was_written_and_keeps_the_rest() {
        let (home, claude, cwd) = setup("init-tel-remove", r#"{"env":{"FOO":"1"}}"#);
        apply(&home, &claude, &cwd, 1, false);
        let out = remove(&home, &claude.join("settings.json"), 2, false).unwrap();
        assert_eq!(out.keys, 8);
        assert_eq!(
            env_of(&claude),
            json!({"FOO":"1"}).as_object().unwrap().clone()
        );
        assert!(!marker_path(&home).exists());
    }

    #[test]
    fn remove_keeps_a_key_the_user_changed_since() {
        let (home, claude, cwd) = setup("init-tel-changed", "{}");
        apply(&home, &claude, &cwd, 1, false);
        let path = claude.join("settings.json");
        let mut doc: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        doc["env"]["OTEL_LOGS_EXPORTER"] = json!("console");
        fs::write(&path, doc.to_string()).unwrap();
        let out = remove(&home, &path, 2, false).unwrap();
        assert_eq!(out.keys, 7);
        assert_eq!(env_of(&claude)["OTEL_LOGS_EXPORTER"], "console");
    }

    #[test]
    fn remove_without_a_record_changes_nothing() {
        let original = r#"{"env":{"CLAUDE_CODE_ENABLE_TELEMETRY":"1"}}"#;
        let (home, claude, _cwd) = setup("init-tel-norecord", original);
        let out = remove(&home, &claude.join("settings.json"), 2, false).unwrap();
        assert_eq!(out, Removed::default());
        assert_eq!(
            fs::read_to_string(claude.join("settings.json")).unwrap(),
            original
        );
    }

    #[test]
    fn a_dry_run_remove_counts_and_writes_nothing() {
        let (home, claude, cwd) = setup("init-tel-dry", "{}");
        apply(&home, &claude, &cwd, 1, false);
        let before = fs::read_to_string(claude.join("settings.json")).unwrap();
        let out = remove(&home, &claude.join("settings.json"), 2, true).unwrap();
        assert_eq!(out.keys, 8);
        assert_eq!(
            fs::read_to_string(claude.join("settings.json")).unwrap(),
            before
        );
        assert!(marker_path(&home).exists());
    }
}
