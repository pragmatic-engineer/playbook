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

use crate::init::run::StepReport;
use serde_json::{Map, Value};
use std::path::Path;

/// The part id, also the name of the step and the setup flag.
pub const PART: &str = "attribution-off";

/// The command that adds the missing keys and keeps the user's own values.
pub const COMMAND: &str = "playbook setup --only attribution-off";

/// The command that also replaces the user's own values.
pub const REPLACE_COMMAND: &str = "playbook setup --only attribution-off --attribution-off";

/// The state key that records a no, so the reminder of migration 0015 stops.
const DECLINED_KEY: &str = "attribution/declined";

/// What setup does with the part, decided from the flags and the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Not asked and no flag, as in a run with no terminal: nothing changes.
    Skip,
    /// The user said no: nothing changes and the reminder stops.
    Declined,
    /// Add the missing keys and keep the user's own values (`--yes`, `--only`).
    AddMissing,
    /// Also replace the user's own values (a yes to the question, `--attribution-off`).
    Replace,
}

/// Whether the user said no, read without creating or changing the store.
pub fn declined(home: &Path) -> bool {
    let root = crate::common::paths::playbook_root_from(home);
    crate::state::list_read_only(&root, DECLINED_KEY).is_ok_and(|rows| !rows.is_empty())
}

/// Records or clears the no. A failure only means the reminder may show again.
fn record_declined(home: &Path, no: bool) {
    let root = crate::common::paths::playbook_root_from(home);
    let _ = match no {
        true => crate::state::set(&root, DECLINED_KEY, "1"),
        false if declined(home) => crate::state::delete(&root, DECLINED_KEY).map(|_| ()),
        false => Ok(()),
    };
}

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
    /// `attribution` is neither an object nor `false`. Setup leaves it alone.
    OtherShape,
    /// settings.json cannot be read or is not a JSON object.
    Unreadable(String),
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
        Err(err) => return State::Unreadable(err),
    };
    state_of(root.get("attribution"))
}

fn state_of(value: Option<&Value>) -> State {
    let current = match value {
        None => Map::new(),
        Some(Value::Bool(false)) => return State::Off,
        Some(Value::Object(map)) => map.clone(),
        Some(_) => return State::OtherShape,
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

/// Why setup leaves an `attribution` of another shape alone.
pub const OTHER_SHAPE: &str =
    "attribution in settings.json is neither an object nor false, left as it is";

fn names(keys: &[&str]) -> String {
    keys.iter()
        .map(|k| format!("attribution.{k}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The `Consent` check of migration 0015: a line when a key is missing, else
/// `None`. A key with the user's own value is not reported, and nothing is
/// after the user said no.
pub fn pending(home: &Path, settings_path: &Path) -> Option<String> {
    if declined(home) {
        return None;
    }
    match state(settings_path) {
        State::Open { missing, .. } if !missing.is_empty() => Some(format!(
            "{} not set in ~/.claude/settings.json, so Claude Code adds its attribution to commits and pull requests",
            names(&missing)
        )),
        _ => None,
    }
}

/// The setup step for `choice`. A no is recorded, and a yes clears it.
pub fn step(home: &Path, settings_path: &Path, epoch: u64, choice: Choice) -> StepReport {
    let replace = match choice {
        Choice::Skip => {
            return StepReport::skipped(PART, "not requested; pass --attribution-off to opt in")
        }
        Choice::Declined => {
            record_declined(home, true);
            return StepReport::skipped(
                PART,
                format!("you said no, so playbook stops reminding you; to turn it off later, run: {COMMAND}"),
            );
        }
        Choice::AddMissing => false,
        Choice::Replace => true,
    };
    let report = apply(settings_path, epoch, replace);
    if report.status != crate::init::run::StepStatus::Failed {
        record_declined(home, false);
    }
    report
}

/// Adds every missing key. `replace` also sets a key that
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
        State::OtherShape => return StepReport::skipped(PART, OTHER_SHAPE),
        State::Unreadable(why) => return StepReport::failed(PART, why),
        State::Open { missing, custom } => (missing, custom),
    };
    let keep: &[&str] = if replace { &[] } else { &custom };
    if missing.is_empty() && !keep.is_empty() {
        return StepReport::skipped(
            PART,
            format!(
                "{} holds your own value, left as it is; to replace it, run: {REPLACE_COMMAND}",
                names(keep)
            ),
        );
    }
    let mut root = match crate::init::telemetry::load(settings_path) {
        Ok(root) => root,
        Err(err) => return StepReport::failed(PART, err),
    };
    let was_sorted = root.keys().is_sorted();
    // Edited in place, so no other key moves in a file that is not sorted.
    let attribution = root
        .entry("attribution")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(attribution) = attribution.as_object_mut() else {
        return StepReport::skipped(PART, OTHER_SHAPE);
    };
    for (key, want) in wanted() {
        if !keep.contains(&key) {
            attribution.insert(key.to_string(), want);
        }
    }
    // A symlinked settings.json (stow, chezmoi) stays a symlink.
    let target =
        std::fs::canonicalize(settings_path).unwrap_or_else(|_| settings_path.to_path_buf());
    if let Err(err) = crate::init::telemetry::write_keeping_order(&target, root, was_sorted, epoch)
    {
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

/// The attribution step of `playbook doctor --fix`, run before the heal so
/// its report and record see the result. It asks only at a terminal, only
/// when a key is missing and the user has not said no before. A yes adds the
/// missing keys and keeps the user's own values; a no is recorded. Without a
/// terminal it writes nothing, because the heal report names the command.
/// `Err` names a write that failed.
pub fn fix_with_consent(
    home: &Path,
    settings_path: &Path,
    interactive: bool,
    ask: &mut dyn FnMut(&str, bool) -> bool,
) -> Result<Option<StepReport>, String> {
    if !interactive || declined(home) || !state(settings_path).missing_any() {
        return Ok(None);
    }
    let choice = if ask(crate::init::catalog::question(PART), false) {
        Choice::AddMissing
    } else {
        Choice::Declined
    };
    let report = step(
        home,
        settings_path,
        crate::common::time::now_epoch_secs(),
        choice,
    );
    if report.status == crate::init::run::StepStatus::Failed {
        return Err(report.render());
    }
    Ok(Some(report))
}

/// Asks `question` on the terminal with the wizard's answer rules. Only a
/// yes counts; any other answer, or end of input, gives `default`.
pub fn ask_on_terminal(question: &str, default: bool) -> bool {
    use crate::ui::wizard::{parse_answer, Answer};
    use std::io::{BufRead, Write};
    print!("{question} {} ", if default { "[Y/n]" } else { "[y/N]" });
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    match parse_answer(&line, default) {
        Answer::Yes => true,
        Answer::No => false,
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

    /// The scratch home that holds `path` as `.claude/settings.json`.
    fn home_of(path: &Path) -> &Path {
        path.parent().and_then(Path::parent).unwrap()
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    fn keys(path: &Path) -> Vec<String> {
        read(path).as_object().unwrap().keys().cloned().collect()
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
        assert_eq!(keys(&path), ["model", "env", "attribution"]);
        assert!(path.with_file_name("settings.json.bak.7").is_file());
        assert!(fs::read_to_string(&path).unwrap().ends_with("}\n"));
    }

    #[test]
    fn a_sorted_file_stays_sorted_and_an_unsorted_one_keeps_its_order() {
        let sorted = settings("sorted", Some(r#"{"env":{},"model":"opus"}"#));
        let unsorted = settings(
            "unsorted",
            Some(r#"{"model":"opus","attribution":{"commit":""},"env":{}}"#),
        );

        apply(&sorted, 1, false);
        apply(&unsorted, 1, false);

        assert_eq!(keys(&sorted), ["attribution", "env", "model"]);
        assert_eq!(keys(&unsorted), ["model", "attribution", "env"]);
        assert_eq!(state(&unsorted), State::Off);
    }

    #[test]
    fn a_missing_file_is_created_and_a_second_run_changes_nothing() {
        let path = settings("again", None);
        assert_eq!(apply(&path, 1, false).status, StepStatus::Wired);
        let before = fs::read_to_string(&path).unwrap();

        let step = apply(&path, 2, false);

        assert_eq!(step.status, StepStatus::AlreadyCorrect);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert!(!path.with_file_name("settings.json.bak.2").exists());
    }

    #[test]
    fn a_user_value_with_nothing_missing_is_left_and_names_the_command() {
        let body = r#"{"attribution":{"commit":"x","pr":"","sessionUrl":true}}"#;
        let path = settings("custom-only", Some(body));

        let step = apply(&path, 1, false);

        assert_eq!(step.status, StepStatus::Skipped);
        assert!(step.detail.contains(REPLACE_COMMAND), "{}", step.detail);
        assert_eq!(fs::read_to_string(&path).unwrap(), body);
        assert_eq!(pending(home_of(&path), &path), None);
    }

    #[test]
    fn attribution_false_counts_as_off_and_another_shape_is_left_alone() {
        let off = settings("false", Some(r#"{"attribution":false}"#));
        let odd = settings("odd", Some(r#"{"attribution":true}"#));

        assert_eq!(state(&off), State::Off);
        assert_eq!(state(&odd), State::OtherShape);
        assert_eq!(apply(&odd, 1, true).status, StepStatus::Skipped);
        assert_eq!(pending(home_of(&odd), &odd), None);
    }

    #[test]
    fn pending_names_only_the_missing_keys() {
        let path = settings("pending", Some(r#"{"attribution":{"commit":""}}"#));

        let line = pending(home_of(&path), &path).expect("pr and sessionUrl are missing");

        assert!(
            line.starts_with("attribution.pr, attribution.sessionUrl not set"),
            "{line}"
        );
    }

    #[test]
    fn doctor_fix_asks_only_at_a_terminal_and_a_no_stops_the_reminder() {
        // Arrange
        let path = settings("declined", Some("{}"));
        let home = home_of(&path);

        // Act
        let unattended = fix_with_consent(home, &path, false, &mut |_, _| panic!("asked"));
        let no = fix_with_consent(home, &path, true, &mut |_, _| false);
        let after_no = fs::read_to_string(&path).unwrap();
        let asked_again = fix_with_consent(home, &path, true, &mut |_, _| panic!("asked"));
        let pending_after_no = pending(home, &path);
        let yes = step(home, &path, 1, Choice::AddMissing);

        // Assert
        assert!(matches!(unattended, Ok(None)));
        assert_eq!(no.unwrap().unwrap().status, StepStatus::Skipped);
        assert_eq!(after_no, "{}");
        assert!(matches!(asked_again, Ok(None)));
        assert_eq!(pending_after_no, None);
        assert_eq!(yes.status, StepStatus::Wired);
        assert!(!declined(home));
    }

    #[test]
    fn doctor_fix_adds_the_missing_keys_after_a_yes() {
        let path = settings("fix", Some(r#"{"attribution":{"commit":"mine"}}"#));

        let fixed = fix_with_consent(home_of(&path), &path, true, &mut |_, d| !d);

        assert_eq!(fixed.unwrap().unwrap().status, StepStatus::Wired);
        assert_eq!(read(&path)["attribution"]["commit"], "mine");
        assert!(!state(&path).missing_any());
    }

    #[test]
    fn skip_writes_nothing_and_records_nothing() {
        let path = settings("skip", Some("{}"));

        let report = step(home_of(&path), &path, 1, Choice::Skip);

        assert_eq!(report.status, StepStatus::Skipped);
        assert_eq!(fs::read_to_string(&path).unwrap(), "{}");
        assert!(pending(home_of(&path), &path).is_some());
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
