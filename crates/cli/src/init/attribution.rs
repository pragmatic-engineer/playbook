// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `attribution-off` part of `playbook setup` (ADR 0042). With consent it
//! sets `attribution` in the user's `~/.claude/settings.json` so Claude Code
//! adds no `Co-Authored-By` trailer, no generated-by line and no claude.ai
//! session link to commits and pull requests.
//!
//! The value is `{"commit": "", "pr": "", "sessionUrl": false}`. The shorter
//! `"attribution": false` needs Claude Code 2.1.281 or later, and an older
//! version skips the whole settings file that holds it, so it is not used.
//!
//! Only a missing key is added. A key that holds a value of the user's own is
//! replaced only after a yes to the question, an explicit `--attribution-off`
//! or `--only attribution-off`; `--yes` alone keeps it.

use crate::init::run::StepReport;
use serde_json::{Map, Value};
use std::path::Path;

/// The part id, also the name of the step and the setup flag.
pub const PART: &str = "attribution-off";

/// The command that applies the part and nothing else.
pub const COMMAND: &str = "playbook setup --only attribution-off";

/// Each key under `attribution` and the value that hides that attribution.
fn wanted() -> [(&'static str, Value); 3] {
    [
        ("commit", Value::String(String::new())),
        ("pr", Value::String(String::new())),
        ("sessionUrl", Value::Bool(false)),
    ]
}

/// Where the user's `attribution` setting stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Every attribution is hidden, or `attribution` is `false`.
    Off,
    /// `missing` keys are not set. `custom` keys hold a value of the user's own.
    Open {
        missing: Vec<&'static str>,
        custom: Vec<&'static str>,
    },
    /// `attribution` is neither an object nor `false`, or settings.json is
    /// not a JSON object. Setup leaves it alone.
    Unusable(String),
}

impl State {
    /// Whether a key is missing, which is what setup adds without asking twice.
    pub fn missing_any(&self) -> bool {
        matches!(self, State::Open { missing, .. } if !missing.is_empty())
    }
}

/// Reads `settings_path` without writing. A missing file counts as every key
/// missing.
pub fn state(settings_path: &Path) -> State {
    let root = match crate::init::telemetry::load(settings_path) {
        Ok(root) => root,
        Err(err) => return State::Unusable(err),
    };
    state_of(root.get("attribution"))
}

fn state_of(value: Option<&Value>) -> State {
    let current = match value {
        None => Map::new(),
        Some(Value::Bool(false)) => return State::Off,
        Some(Value::Object(map)) => map.clone(),
        Some(_) => {
            return State::Unusable(
                "attribution in settings.json is not an object, left as it is".to_string(),
            )
        }
    };
    let mut missing = Vec::new();
    let mut custom = Vec::new();
    for (key, want) in wanted() {
        match current.get(key) {
            None => missing.push(key),
            Some(v) if *v == want => {}
            Some(_) => custom.push(key),
        }
    }
    if missing.is_empty() && custom.is_empty() {
        State::Off
    } else {
        State::Open { missing, custom }
    }
}

fn names(keys: &[&str]) -> String {
    keys.iter()
        .map(|k| format!("attribution.{k}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The `Consent` check of migration 0015: a line when a key is missing, else
/// `None`. A key with the user's own value is not reported.
pub fn pending(settings_path: &Path) -> Option<String> {
    match state(settings_path) {
        State::Open { missing, .. } if !missing.is_empty() => Some(format!(
            "{} not set in ~/.claude/settings.json, so Claude Code adds its attribution to commits and pull requests",
            names(&missing)
        )),
        _ => None,
    }
}

/// The setup step. Adds every missing key. `replace` also sets a key that
/// holds a value of the user's own. Keeps every other setting, the key order
/// of a sorted file, a symlinked file, and a backup next to it.
pub fn apply(settings_path: &Path, epoch: u64, replace: bool) -> StepReport {
    let (missing, custom) = match state(settings_path) {
        State::Off => {
            return StepReport::already_correct(
                PART,
                "commit, PR and session link attribution are off",
            )
        }
        State::Unusable(why) => return StepReport::skipped(PART, why),
        State::Open { missing, custom } => (missing, custom),
    };
    let keep: &[&str] = if replace { &[] } else { &custom };
    if missing.is_empty() && !keep.is_empty() {
        return StepReport::skipped(
            PART,
            format!(
                "{} holds your own value, left as it is; to replace it, run: {COMMAND}",
                names(keep)
            ),
        );
    }
    let mut root = match crate::init::telemetry::load(settings_path) {
        Ok(root) => root,
        Err(err) => return StepReport::failed(PART, err),
    };
    let was_sorted = root.keys().is_sorted();
    let mut attribution = match root.remove("attribution") {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    for (key, want) in wanted() {
        if !keep.contains(&key) {
            attribution.insert(key.to_string(), want);
        }
    }
    root.insert("attribution".to_string(), Value::Object(attribution));
    if was_sorted {
        let mut entries: Vec<(String, Value)> = std::mem::take(&mut root).into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        root.extend(entries);
    }
    // A symlinked settings.json (stow, chezmoi) stays a symlink.
    let target =
        std::fs::canonicalize(settings_path).unwrap_or_else(|_| settings_path.to_path_buf());
    if let Err(err) = crate::init::telemetry::write_with_backup(&target, root, epoch) {
        return StepReport::failed(PART, err);
    }
    let mut detail =
        "set attribution so commits and pull requests carry no Claude attribution or session link"
            .to_string();
    if !keep.is_empty() {
        detail.push_str(&format!("; {} keeps your own value", names(keep)));
    }
    StepReport::wired(PART, detail)
}

/// The attribution step of `playbook doctor --fix`. It writes only after a
/// yes at a terminal, and only when a key is missing; a value of the user's
/// own stays. Without a terminal it writes nothing, because the heal report
/// already names the command. `Err` names a write that failed.
pub fn fix_with_consent(
    settings_path: &Path,
    interactive: bool,
    ask: &mut dyn FnMut(&str, bool) -> bool,
) -> Result<Option<StepReport>, String> {
    if !interactive || !state(settings_path).missing_any() {
        return Ok(None);
    }
    if !ask(crate::init::catalog::question(PART), false) {
        return Ok(None);
    }
    let epoch = crate::common::time::now_epoch_secs();
    let step = apply(settings_path, epoch, false);
    if step.status == crate::init::run::StepStatus::Failed {
        return Err(step.render());
    }
    Ok(Some(step))
}

/// Asks `question` on the terminal. Only `y` or `yes` is a yes; an empty
/// answer or end of input gives `default`.
pub fn ask_on_terminal(question: &str, default: bool) -> bool {
    use std::io::{BufRead, Write};
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    print!("{question} {hint} ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).unwrap_or(0) == 0 {
        println!();
        return default;
    }
    match line.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use crate::init::run::StepStatus;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;

    fn settings(tag: &str, body: Option<&str>) -> PathBuf {
        let dir = scratch_dir(&format!("attribution-{tag}"));
        let path = dir.join(".claude").join("settings.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if let Some(body) = body {
            fs::write(&path, body).unwrap();
        }
        path
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn a_missing_key_is_added_and_every_other_setting_stays() {
        // Arrange
        let path = settings("missing", Some(r#"{"model":"opus","env":{"A":"1"}}"#));

        // Act
        let step = apply(&path, 7, false);

        // Assert
        assert_eq!(step.status, StepStatus::Wired, "{}", step.detail);
        assert_eq!(
            read(&path),
            json!({"model":"opus","env":{"A":"1"},"attribution":{"commit":"","pr":"","sessionUrl":false}})
        );
        assert!(path.with_file_name("settings.json.bak.7").is_file());
        assert!(fs::read_to_string(&path).unwrap().ends_with("}\n"));
        assert_eq!(state(&path), State::Off);
    }

    #[test]
    fn a_sorted_file_stays_sorted() {
        let path = settings("sorted", Some(r#"{"env":{},"model":"opus"}"#));

        apply(&path, 1, false);

        let keys: Vec<String> = read(&path).as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["attribution", "env", "model"]);
    }

    #[test]
    fn no_settings_file_gets_one_with_the_key() {
        let path = settings("nofile", None);

        let step = apply(&path, 1, false);

        assert_eq!(step.status, StepStatus::Wired);
        assert_eq!(state(&path), State::Off);
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let path = settings("again", Some("{}"));
        apply(&path, 1, false);
        let before = fs::read_to_string(&path).unwrap();

        let step = apply(&path, 2, false);

        assert_eq!(step.status, StepStatus::AlreadyCorrect);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert!(!path.with_file_name("settings.json.bak.2").exists());
    }

    #[test]
    fn a_user_value_is_kept_without_consent_and_the_missing_keys_are_added() {
        // Arrange
        let path = settings("custom", Some(r#"{"attribution":{"commit":"Made by me"}}"#));

        // Act
        let step = apply(&path, 1, false);

        // Assert
        assert_eq!(step.status, StepStatus::Wired);
        assert!(step
            .detail
            .contains("attribution.commit keeps your own value"));
        assert_eq!(
            read(&path)["attribution"],
            json!({"commit":"Made by me","pr":"","sessionUrl":false})
        );
        assert_eq!(pending(&path), None);
    }

    #[test]
    fn a_user_value_with_nothing_missing_is_left_and_names_the_command() {
        let body = r#"{"attribution":{"commit":"x","pr":"","sessionUrl":true}}"#;
        let path = settings("custom-only", Some(body));

        let step = apply(&path, 1, false);

        assert_eq!(step.status, StepStatus::Skipped);
        assert!(step.detail.contains(COMMAND), "{}", step.detail);
        assert_eq!(fs::read_to_string(&path).unwrap(), body);
    }

    #[test]
    fn consent_replaces_a_user_value() {
        let path = settings(
            "replace",
            Some(r#"{"attribution":{"commit":"x","sessionUrl":true}}"#),
        );

        apply(&path, 1, true);

        assert_eq!(state(&path), State::Off);
    }

    #[test]
    fn attribution_false_counts_as_off_and_another_shape_is_left_alone() {
        let off = settings("false", Some(r#"{"attribution":false}"#));
        let odd = settings("odd", Some(r#"{"attribution":"none"}"#));

        assert_eq!(state(&off), State::Off);
        assert_eq!(apply(&odd, 1, true).status, StepStatus::Skipped);
        assert_eq!(pending(&odd), None);
    }

    #[test]
    fn pending_names_only_the_missing_keys() {
        let path = settings("pending", Some(r#"{"attribution":{"commit":""}}"#));

        let line = pending(&path).expect("pr and sessionUrl are missing");

        assert!(
            line.starts_with("attribution.pr, attribution.sessionUrl not set"),
            "{line}"
        );
    }

    #[test]
    fn doctor_fix_writes_only_after_a_yes_at_a_terminal() {
        // Arrange
        let path = settings("fix", Some("{}"));
        let mut asked = 0;
        let mut no = |_: &str, _: bool| {
            asked += 1;
            false
        };

        // Act
        let unattended = fix_with_consent(&path, false, &mut |_, _| panic!("asked"));
        let declined = fix_with_consent(&path, true, &mut no);
        let after_no = fs::read_to_string(&path).unwrap();
        let accepted = fix_with_consent(&path, true, &mut |_, default| !default);

        // Assert
        assert!(matches!(unattended, Ok(None)));
        assert!(matches!(declined, Ok(None)));
        assert_eq!(asked, 1);
        assert_eq!(after_no, "{}");
        assert_eq!(accepted.unwrap().unwrap().status, StepStatus::Wired);
        assert_eq!(state(&path), State::Off);
        assert!(matches!(
            fix_with_consent(&path, true, &mut |_, _| panic!("asked")),
            Ok(None)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_settings_file_stays_a_symlink() {
        let path = settings("link", None);
        let real = path.with_file_name("real.json");
        fs::write(&real, "{}").unwrap();
        std::os::unix::fs::symlink(&real, &path).unwrap();

        apply(&path, 1, false);

        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(state(&real), State::Off);
    }
}
