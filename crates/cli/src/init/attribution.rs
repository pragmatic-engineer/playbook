// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `attribution` part of `playbook setup` (ADR 0042).
//! Attribution is what Claude Code adds to commits and pull requests: a
//! `Co-Authored-By` trailer, a generated-by line and a claude.ai session link.
//! The part has two states:
//!
//! - OFF, the default: playbook sets `attribution` in the user's
//!   `~/.claude/settings.json` so Claude Code adds none of it.
//! - ON: playbook removes the keys it wrote, each key that holds exactly the
//!   off value, so Claude Code adds its attribution again. Any other value
//!   stays.
//!
//! The off value is `{"commit": "", "pr": "", "sessionUrl": false}`. The
//! shorter `"attribution": false` needs Claude Code 2.1.281 or later, and an
//! older version skips the whole settings file that holds it, so it is not
//! used. A choice of ON is stored as `attribution.enabled`, so `--yes`, an
//! `--only` run and `playbook doctor` keep it. A no to the old question in
//! 0.36 or 0.37 counts as ON until a choice is stored.

use crate::init::run::StepReport;
use serde_json::{Map, Value};
use std::path::Path;

/// The part id, also the name of the step and the setup flag.
pub const PART: &str = "attribution";

/// The per-user key that stores the choice: true is ON.
pub const KEY: &str = "attribution.enabled";

/// The command that applies the stored choice, OFF unless ON was chosen.
pub const COMMAND: &str = "playbook setup --only attribution";

/// The command that turns attribution off and replaces the user's own values.
pub const OFF_COMMAND: &str = "playbook setup --only attribution --no-attribution";

/// The command that turns attribution on.
pub const ON_COMMAND: &str = "playbook setup --only attribution --attribution";

/// The state key 0.36 and 0.37 wrote for a no. Only read.
const DECLINED_KEY: &str = "attribution/declined";

/// How the wizard summary and the dry run name each state.
pub const ON_LABEL: &str = "ON: Claude Code adds its attribution";
/// See [`ON_LABEL`].
pub const OFF_LABEL: &str = "OFF (default): Claude Code adds no attribution";

/// What setup does with the part, decided from the flags, the answer and
/// the stored choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// No flag and no answer, as in a run with no terminal and no `--yes`,
    /// or an `--only` run that does not name the part: nothing changes.
    Skip,
    /// No flag and no answer under `--yes` or `--only attribution`: the
    /// stored choice, else OFF that keeps the user's own values.
    Default,
    /// Attribution OFF: add the missing keys. `replace` also sets a key that
    /// holds the user's own value, after `--no-attribution`.
    Off { replace: bool },
    /// Attribution ON: remove the keys that hold exactly the off value.
    On,
}

impl Choice {
    /// `Default` turned into the stored choice; any other choice as it is.
    pub fn resolve(self, home: &Path) -> Choice {
        match self {
            Choice::Default if enabled(home) => Choice::On,
            Choice::Default => Choice::Off { replace: false },
            other => other,
        }
    }
}

/// Whether the user chose ON: `KEY` when stored, else a no to the question
/// of 0.36 or 0.37.
pub fn enabled(home: &Path) -> bool {
    match crate::config::resolve(KEY, home, None) {
        Ok((Value::Bool(on), source)) if !matches!(source, crate::config::Source::Default) => on,
        _ => declined(home),
    }
}

/// Stores the choice in `KEY` when it differs from what is stored.
fn record(home: &Path, on: bool) -> Result<(), String> {
    if enabled(home) == on {
        return Ok(());
    }
    crate::config::write::set(
        crate::config::write::Tier::Global,
        KEY,
        Value::Bool(on),
        home,
        None,
    )
    .map(|_| ())
    .map_err(|err| format!("could not store {KEY}: {err}"))
}

/// Whether the user said no in 0.36 or 0.37, read without creating or
/// changing the store.
fn declined(home: &Path) -> bool {
    let root = crate::common::paths::playbook_root_from(home);
    crate::state::list_read_only(&root, DECLINED_KEY).is_ok_and(|rows| !rows.is_empty())
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

/// The keys under `attribution` that hold exactly the off value playbook
/// writes. Empty when the file cannot be read or `attribution` is `false`.
pub fn held_off(settings_path: &Path) -> Vec<&'static str> {
    match crate::init::telemetry::load(settings_path) {
        Ok(root) => match root.get("attribution") {
            Some(Value::Object(map)) => off_keys(map),
            _ => Vec::new(),
        },
        Err(_) => Vec::new(),
    }
}

/// The keys of the `attribution` object `map` that hold the off value.
fn off_keys(map: &Map<String, Value>) -> Vec<&'static str> {
    wanted()
        .into_iter()
        .filter(|(key, want)| map.get(*key) == Some(want))
        .map(|(key, _)| key)
        .collect()
}

/// Why setup leaves an `attribution` of another shape alone.
pub const OTHER_SHAPE: &str =
    "attribution in settings.json is neither an object nor false, left as it is";

/// Names the keys as `attribution.<key>`, comma separated.
pub fn names(keys: &[&str]) -> String {
    keys.iter()
        .map(|k| format!("attribution.{k}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The setup step for `choice`. After the edit, OFF or ON is stored in
/// `KEY`, so a later `--yes` keeps the choice.
pub fn step(home: &Path, settings_path: &Path, epoch: u64, choice: Choice) -> StepReport {
    let (on, report) = match choice.resolve(home) {
        Choice::Skip => {
            return StepReport::skipped(
                PART,
                "not requested; pass --no-attribution for attribution off (the default) or --attribution for on",
            )
        }
        Choice::Off { replace } => (false, apply(settings_path, epoch, replace)),
        Choice::Default => (false, apply(settings_path, epoch, false)),
        Choice::On => (true, remove(settings_path, epoch)),
    };
    if report.status == crate::init::run::StepStatus::Failed {
        return report;
    }
    match record(home, on) {
        Ok(()) => report,
        Err(err) => StepReport::failed(PART, format!("{}; {err}", report.detail)),
    }
}

/// Attribution OFF: adds every missing key. `replace` also sets a key that
/// holds a value of the user's own. Keeps every other setting, the key order
/// of a sorted file, a symlinked file, and a backup next to it.
pub fn apply(settings_path: &Path, epoch: u64, replace: bool) -> StepReport {
    let (missing, custom) =
        match state(settings_path) {
            State::Off => return StepReport::already_correct(
                PART,
                "attribution off: Claude Code adds no trailer, generated-by line or session link",
            ),
            State::OtherShape => return StepReport::skipped(PART, OTHER_SHAPE),
            State::Unreadable(why) => return StepReport::failed(PART, why),
            State::Open { missing, custom } => (missing, custom),
        };
    let keep: &[&str] = if replace { &[] } else { &custom };
    if missing.is_empty() && !keep.is_empty() {
        return StepReport::skipped(
            PART,
            format!(
                "{} holds your own value, left as it is; to replace it, run: {OFF_COMMAND}",
                names(keep)
            ),
        );
    }
    let detail = {
        let mut d = "attribution off: set attribution so Claude Code adds no trailer, generated-by line or session link".to_string();
        if !keep.is_empty() {
            d.push_str(&format!("; {} keeps your own value", names(keep)));
        }
        d
    };
    edit(settings_path, epoch, detail, |root| {
        let attribution = root
            .entry("attribution")
            .or_insert_with(|| Value::Object(Map::new()));
        let Some(attribution) = attribution.as_object_mut() else {
            return false;
        };
        for (key, want) in wanted() {
            if !keep.contains(&key) {
                attribution.insert(key.to_string(), want);
            }
        }
        true
    })
}

/// Attribution ON: removes each key that holds exactly the off value, and
/// `attribution` itself when nothing is left in it. Any other value stays.
pub fn remove(settings_path: &Path, epoch: u64) -> StepReport {
    let root = match crate::init::telemetry::load(settings_path) {
        Ok(root) => root,
        Err(err) => return StepReport::failed(PART, err),
    };
    let ours = match root.get("attribution") {
        None => {
            return StepReport::already_correct(
                PART,
                "attribution on: no attribution key is set, so Claude Code adds its attribution",
            )
        }
        Some(Value::Object(map)) => off_keys(map),
        Some(Value::Bool(false)) => {
            return StepReport::skipped(
                PART,
                "attribution in settings.json is false, which playbook does not write, left as it is; delete it to get Claude Code's attribution back",
            )
        }
        Some(_) => return StepReport::skipped(PART, OTHER_SHAPE),
    };
    if ours.is_empty() {
        return StepReport::already_correct(
            PART,
            "attribution on: no key holds the value playbook writes, so your settings stay as they are",
        );
    }
    let detail = format!(
        "attribution on: removed {} so Claude Code adds its attribution again",
        names(&ours)
    );
    edit(settings_path, epoch, detail, |root| {
        let Some(Value::Object(attribution)) = root.get_mut("attribution") else {
            return false;
        };
        for key in &ours {
            attribution.remove(*key);
        }
        if attribution.is_empty() {
            root.remove("attribution");
        }
        true
    })
}

/// Loads settings.json, runs `change` on it and writes it back with a backup,
/// keeping the key order of a sorted file and a symlinked file. `change`
/// returns false when `attribution` has another shape.
fn edit(
    settings_path: &Path,
    epoch: u64,
    detail: String,
    change: impl FnOnce(&mut Map<String, Value>) -> bool,
) -> StepReport {
    let mut root = match crate::init::telemetry::load(settings_path) {
        Ok(root) => root,
        Err(err) => return StepReport::failed(PART, err),
    };
    let was_sorted = root.keys().is_sorted();
    // Edited in place, so no other key moves in a file that is not sorted.
    if !change(&mut root) {
        return StepReport::skipped(PART, OTHER_SHAPE);
    }
    // A symlinked settings.json (stow, chezmoi) stays a symlink.
    let target =
        std::fs::canonicalize(settings_path).unwrap_or_else(|_| settings_path.to_path_buf());
    if let Err(err) = crate::init::telemetry::write_keeping_order(&target, root, was_sorted, epoch)
    {
        return StepReport::failed(PART, err);
    }
    StepReport::wired(PART, detail)
}

/// The question `playbook doctor --fix` asks before it adds a missing key.
pub const FIX_QUESTION: &str = "Turn attribution off, the playbook default, so Claude Code adds no Co-Authored-By trailer, generated-by line or claude.ai session link? This adds the missing `attribution` keys to ~/.claude/settings.json and keeps your own values.";

/// The attribution step of `playbook doctor --fix`, run before the heal so
/// its report and record see the result. It asks only at a terminal, only
/// when a key is missing and the user has not chosen ON. A yes turns
/// attribution off and keeps the user's own values. A no stores ON, so
/// doctor stops asking. Without a terminal it writes nothing, because the
/// doctor row names the command. `Err` names a write that failed.
pub fn fix_with_consent(
    home: &Path,
    settings_path: &Path,
    interactive: bool,
    ask: &mut dyn FnMut(&str, bool) -> bool,
) -> Result<Option<StepReport>, String> {
    if !interactive || enabled(home) || !state(settings_path).missing_any() {
        return Ok(None);
    }
    if ask(FIX_QUESTION, false) {
        let report = step(
            home,
            settings_path,
            crate::common::time::now_epoch_secs(),
            Choice::Off { replace: false },
        );
        if report.status == crate::init::run::StepStatus::Failed {
            return Err(report.render());
        }
        return Ok(Some(report));
    }
    record(home, true)?;
    Ok(Some(StepReport::skipped(
        PART,
        format!("you said no, so playbook adds no attribution keys and stores {KEY} as true; to turn attribution off later, run: {OFF_COMMAND}"),
    )))
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
        assert!(step.detail.contains(OFF_COMMAND), "{}", step.detail);
        assert_eq!(fs::read_to_string(&path).unwrap(), body);
    }

    #[test]
    fn attribution_false_counts_as_off_and_another_shape_is_left_alone() {
        let off = settings("false", Some(r#"{"attribution":false}"#));
        let odd = settings("odd", Some(r#"{"attribution":true}"#));

        assert_eq!(state(&off), State::Off);
        assert_eq!(state(&odd), State::OtherShape);
        assert_eq!(apply(&odd, 1, true).status, StepStatus::Skipped);
        assert_eq!(remove(&odd, 1).status, StepStatus::Skipped);
        assert_eq!(remove(&off, 1).status, StepStatus::Skipped);
        assert_eq!(read(&off), json!({"attribution": false}));
    }

    #[test]
    fn on_removes_only_the_off_values_and_keeps_a_user_value_and_the_key_order() {
        // Arrange
        let path = settings(
            "on",
            Some(
                r#"{"model":"opus","attribution":{"commit":"custom","pr":"","sessionUrl":false},"env":{}}"#,
            ),
        );

        // Act
        let step = remove(&path, 3);

        // Assert
        assert_eq!(step.status, StepStatus::Wired, "{}", step.detail);
        assert!(step
            .detail
            .contains("attribution.pr, attribution.sessionUrl"));
        assert_eq!(
            read(&path),
            json!({"model":"opus","attribution":{"commit":"custom"},"env":{}})
        );
        assert_eq!(keys(&path), ["model", "attribution", "env"]);
        assert!(path.with_file_name("settings.json.bak.3").is_file());
    }

    #[test]
    fn on_drops_the_attribution_key_when_only_the_off_values_were_in_it() {
        let path = settings(
            "on-all",
            Some(r#"{"attribution":{"commit":"","pr":"","sessionUrl":false},"model":"opus"}"#),
        );

        let first = remove(&path, 1);
        let again = remove(&path, 2);

        assert_eq!(first.status, StepStatus::Wired);
        assert_eq!(read(&path), json!({"model":"opus"}));
        assert_eq!(again.status, StepStatus::AlreadyCorrect);
        assert!(!path.with_file_name("settings.json.bak.2").exists());
        let mine = r#"{"attribution":{"commit":"custom","sessionUrl":true}}"#;
        let custom = settings("on-custom", Some(mine));
        assert_eq!(remove(&custom, 1).status, StepStatus::AlreadyCorrect);
        assert_eq!(fs::read_to_string(&custom).unwrap(), mine);
    }

    #[test]
    fn the_step_stores_on_and_off_and_off_clears_a_stored_on() {
        let path = settings("store", Some("{}"));
        let home = home_of(&path);

        let on = step(home, &path, 1, Choice::On);
        let on_stored = enabled(home);
        let off = step(home, &path, 2, Choice::Off { replace: true });

        assert_eq!(on.status, StepStatus::AlreadyCorrect, "{}", on.detail);
        assert!(on_stored);
        assert_eq!(off.status, StepStatus::Wired, "{}", off.detail);
        assert!(!enabled(home));
        assert_eq!(state(&path), State::Off);
    }

    #[test]
    fn doctor_fix_asks_only_at_a_terminal_and_a_no_stores_on() {
        // Arrange
        let path = settings("declined", Some("{}"));
        let home = home_of(&path);

        // Act
        let unattended = fix_with_consent(home, &path, false, &mut |_, _| panic!("asked"));
        let no = fix_with_consent(home, &path, true, &mut |_, _| false);
        let after_no = fs::read_to_string(&path).unwrap();
        let asked_again = fix_with_consent(home, &path, true, &mut |_, _| panic!("asked"));

        // Assert
        assert!(matches!(unattended, Ok(None)));
        assert_eq!(no.unwrap().unwrap().status, StepStatus::Skipped);
        assert_eq!(after_no, "{}");
        assert!(enabled(home));
        assert!(matches!(asked_again, Ok(None)));
    }

    #[test]
    fn a_no_from_0_37_counts_as_on_until_a_choice_is_stored() {
        let path = settings("legacy-no", Some("{}"));
        let home = home_of(&path);
        let root = crate::common::paths::playbook_root_from(home);
        crate::state::set(&root, DECLINED_KEY, "1").unwrap();

        let asked = fix_with_consent(home, &path, true, &mut |_, _| panic!("asked"));
        let default = Choice::Default.resolve(home);
        let off = step(home, &path, 1, Choice::Off { replace: true });

        assert!(matches!(asked, Ok(None)));
        assert_eq!(default, Choice::On);
        assert_eq!(off.status, StepStatus::Wired);
        assert!(!enabled(home), "a stored OFF wins over the old no");
        assert_eq!(
            Choice::Default.resolve(home),
            Choice::Off { replace: false }
        );
    }

    #[test]
    fn doctor_fix_adds_the_missing_keys_after_a_yes() {
        let path = settings("fix", Some(r#"{"attribution":{"commit":"mine"}}"#));

        let fixed = fix_with_consent(home_of(&path), &path, true, &mut |q, d| {
            assert_eq!(q, FIX_QUESTION);
            !d
        });

        assert_eq!(fixed.unwrap().unwrap().status, StepStatus::Wired);
        assert_eq!(read(&path)["attribution"]["commit"], "mine");
        assert!(!state(&path).missing_any());
        assert!(!enabled(home_of(&path)));
    }

    #[test]
    fn skip_writes_nothing_and_stores_nothing() {
        let path = settings("skip", Some("{}"));

        let report = step(home_of(&path), &path, 1, Choice::Skip);

        assert_eq!(report.status, StepStatus::Skipped);
        assert_eq!(fs::read_to_string(&path).unwrap(), "{}");
        assert!(!enabled(home_of(&path)));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_settings_file_stays_a_symlink() {
        let path = settings("link", None);
        let real = path.with_file_name("real.json");
        fs::write(&real, "{}").unwrap();
        std::os::unix::fs::symlink(&real, &path).unwrap();

        apply(&path, 1, false);
        let off = state(&real);
        remove(&path, 2);

        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(off, State::Off);
        assert_eq!(read(&real), json!({}));
    }
}
