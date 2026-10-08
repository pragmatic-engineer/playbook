// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook effort`: playbook's own ceiling on effort.
//!
//! The config key `effort.max` (`auto`, `low`, `medium`, `high`, `xhigh`,
//! `max`) is playbook's setting. Claude Code has its own, `maxEffortLevel` in
//! `~/.claude/settings.json`, and it is the only thing that caps the effort a
//! command, skill or agent sets in its own frontmatter. The effective ceiling
//! is the lower of the two:
//!
//! - Claude Code's cap is already as low or lower: playbook writes nothing and
//!   Claude Code enforces its own.
//! - Playbook's is lower: playbook writes it as `maxEffortLevel` and remembers
//!   the value it replaced, so `auto` (or a later, higher level) puts it back.
//! - `auto` means playbook sets no ceiling.
//!
//! A `maxEffortLevel` the user set is never lost: it is restored, not
//! deleted. Claude Code 2.1.267 or later is needed for the cap to apply.

use crate::common::atomic::write_atomic;
use crate::common::paths::playbook_root_from;
use crate::config::{self, write};
use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The config key.
pub const KEY: &str = "effort.max";

/// Every accepted value, `auto` first.
pub const LEVELS: [&str; 6] = ["auto", "low", "medium", "high", "xhigh", "max"];

/// Position of `level` from lowest to highest, `None` for `auto` or unknown.
fn rank(level: &str) -> Option<usize> {
    LEVELS.iter().skip(1).position(|l| *l == level)
}

/// The lower of two levels. An unknown or missing side loses.
fn lower<'a>(a: Option<&'a str>, b: Option<&'a str>) -> Option<&'a str> {
    match (
        a.and_then(|l| rank(l).map(|r| (l, r))),
        b.and_then(|l| rank(l).map(|r| (l, r))),
    ) {
        (Some((la, ra)), Some((lb, rb))) => Some(if ra <= rb { la } else { lb }),
        (Some((l, _)), None) | (None, Some((l, _))) => Some(l),
        (None, None) => None,
    }
}

/// What `sync` did to `settings.json`.
#[derive(Debug, PartialEq, Eq)]
pub enum Synced {
    /// Playbook's cap was written.
    Set(String),
    /// Playbook's cap was removed and the user's earlier value put back.
    Cleared,
    /// The Claude Code cap is as low or lower, so it stays and wins.
    Deferred(String),
    /// Nothing needed to change.
    Unchanged,
}

/// What playbook wrote, and the user's value it replaced.
#[derive(Debug, Default, PartialEq, Eq)]
struct Owned {
    value: String,
    previous: Option<String>,
}

fn marker_path(home: &Path) -> PathBuf {
    playbook_root_from(home).join(".effort-owned")
}

fn read_marker(home: &Path) -> Option<Owned> {
    let text = fs::read_to_string(marker_path(home)).ok()?;
    let text = text.trim();
    // The first release wrote the bare level, with nothing to restore.
    if !text.starts_with('{') {
        return (!text.is_empty()).then(|| Owned {
            value: text.to_string(),
            previous: None,
        });
    }
    let v: Value = serde_json::from_str(text).ok()?;
    Some(Owned {
        value: v.get("owned")?.as_str()?.to_string(),
        previous: v
            .get("previous")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn write_marker(home: &Path, owned: &Owned) -> io::Result<()> {
    let marker = marker_path(home);
    if let Some(dir) = marker.parent() {
        fs::create_dir_all(dir)?;
    }
    let body = serde_json::json!({"owned": owned.value, "previous": owned.previous});
    write_atomic(&marker, &format!("{body}\n"))
}

fn clear_marker(home: &Path) {
    let _ = fs::remove_file(marker_path(home));
}

/// The configured level, `auto` when no tier sets one.
pub fn configured(home: &Path) -> String {
    match config::resolve(KEY, home, None) {
        Ok((Value::String(level), _)) => level,
        _ => "auto".to_string(),
    }
}

fn read_settings(path: &Path) -> io::Result<Map<String, Value>> {
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => Ok(map),
            _ => Err(io::Error::other(format!(
                "{} is not a JSON object, left unchanged",
                path.display()
            ))),
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Map::new()),
        Err(err) => Err(err),
    }
}

fn cap_of(root: &Map<String, Value>) -> Option<String> {
    root.get("maxEffortLevel")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// The Claude Code cap the user set, ignoring one playbook wrote itself.
pub fn claude_cap(home: &Path, claude_home: &Path) -> Option<String> {
    let root = read_settings(&claude_home.join("settings.json")).ok()?;
    let current = cap_of(&root);
    match read_marker(home) {
        Some(o) if current.as_deref() == Some(o.value.as_str()) => o.previous,
        _ => current,
    }
}

/// The ceiling that applies: the lower of playbook's and Claude Code's.
pub fn effective(configured: &str, claude: Option<&str>) -> Option<String> {
    lower(Some(configured), claude).map(str::to_string)
}

/// Make `settings.json` agree with `level`.
pub fn sync(home: &Path, claude_home: &Path, level: &str) -> io::Result<Synced> {
    let settings_path = claude_home.join("settings.json");
    let mut root = read_settings(&settings_path)?;
    let current = cap_of(&root);
    let marker = read_marker(home);
    let ours = marker
        .as_ref()
        .filter(|o| current.as_deref() == Some(o.value.as_str()));
    // The user's own cap: what was there before ours, or what is there now.
    let user_cap: Option<String> = match ours {
        Some(o) => o.previous.clone(),
        None => current.clone(),
    };

    let wants_ours = level != "auto"
        && match (rank(level), user_cap.as_deref().and_then(rank)) {
            (Some(mine), Some(theirs)) => mine < theirs,
            (Some(_), None) => true,
            _ => false,
        };

    if !wants_ours {
        // Put the user's value back, or leave theirs alone.
        if let Some(o) = ours {
            match &o.previous {
                Some(prev) => {
                    root.insert("maxEffortLevel".into(), Value::String(prev.clone()));
                }
                None => {
                    root.remove("maxEffortLevel");
                }
            }
            write_settings(&settings_path, root)?;
            clear_marker(home);
            return Ok(if level == "auto" {
                Synced::Cleared
            } else {
                Synced::Deferred(user_cap.unwrap_or_default())
            });
        }
        if marker.is_some() {
            clear_marker(home);
        }
        return Ok(match (level, user_cap) {
            ("auto", _) | (_, None) => Synced::Unchanged,
            (_, Some(cap)) => Synced::Deferred(cap),
        });
    }

    if ours.is_some() && current.as_deref() == Some(level) {
        return Ok(Synced::Unchanged);
    }
    root.insert("maxEffortLevel".into(), Value::String(level.to_string()));
    write_settings(&settings_path, root)?;
    write_marker(
        home,
        &Owned {
            value: level.to_string(),
            previous: user_cap,
        },
    )?;
    Ok(Synced::Set(level.to_string()))
}

fn write_settings(path: &Path, root: Map<String, Value>) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let body = serde_json::to_string_pretty(&Value::Object(root)).map_err(io::Error::other)?;
    // A symlinked settings.json (stow, chezmoi) stays a symlink.
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    write_atomic(&target, &format!("{body}\n"))
}

/// `playbook effort <level>`: store the level, then apply it.
pub fn run_set(home: &Path, claude_home: &Path, level: &str) -> Result<String, String> {
    if !LEVELS.contains(&level) {
        return Err(format!(
            "unknown effort level '{level}', use one of: {}",
            LEVELS.join(", ")
        ));
    }
    write::set(
        write::Tier::Global,
        KEY,
        Value::String(level.to_string()),
        home,
        None,
    )
    .map_err(|err| err.to_string())?;
    let synced = sync(home, claude_home, level).map_err(|err| err.to_string())?;
    Ok(match (level, synced) {
        ("auto", Synced::Cleared) => {
            "effort cap removed, your earlier Claude Code cap is back, playbook defaults apply"
                .to_string()
        }
        ("auto", _) => "effort set to auto, playbook defaults apply".to_string(),
        (l, Synced::Set(_)) => format!(
            "effort capped at {l}: anything set higher runs at {l} (needs Claude Code 2.1.267 or later)"
        ),
        (l, Synced::Deferred(cap)) => format!(
            "effort set to {l}, but Claude Code already caps at {cap}, which is lower or equal, so {cap} applies"
        ),
        (l, _) => format!("effort already capped at {l}"),
    })
}

/// `playbook effort` with no level: playbook's value, Claude Code's value and
/// the one that wins.
pub fn run_status(home: &Path, claude_home: &Path) -> String {
    let level = configured(home);
    let claude = claude_cap(home, claude_home);
    let applied = read_settings(&claude_home.join("settings.json"))
        .ok()
        .and_then(|r| cap_of(&r));
    let eff = effective(&level, claude.as_deref());
    let mut out = format!(
        "playbook effort.max: {level}\nClaude Code maxEffortLevel: {}\neffective ceiling: {}",
        claude.as_deref().unwrap_or("none"),
        eff.as_deref().unwrap_or("none, shipped defaults apply"),
    );
    let in_sync = match (eff.as_deref(), applied.as_deref()) {
        (None, _) => true,
        (Some(e), Some(a)) => e == a,
        (Some(_), None) => false,
    };
    if !in_sync {
        out.push_str("\nnot applied yet, run `playbook init` or `playbook effort <level>`");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    struct Env {
        home: PathBuf,
        claude: PathBuf,
    }

    fn env(tag: &str) -> Env {
        let home = scratch_dir(tag);
        let claude = home.join(".claude");
        fs::create_dir_all(&claude).unwrap();
        Env { home, claude }
    }

    impl Env {
        fn settings(&self, body: &str) {
            fs::write(self.claude.join("settings.json"), body).unwrap();
        }
        fn cap(&self) -> Option<String> {
            let v: Value =
                serde_json::from_str(&fs::read_to_string(self.claude.join("settings.json")).ok()?)
                    .ok()?;
            v.get("maxEffortLevel")?.as_str().map(str::to_string)
        }
        fn sync(&self, level: &str) -> Synced {
            sync(&self.home, &self.claude, level).unwrap()
        }
    }

    #[test]
    fn the_lower_of_two_levels_wins() {
        assert_eq!(lower(Some("xhigh"), Some("max")), Some("xhigh"));
        assert_eq!(lower(Some("xhigh"), Some("medium")), Some("medium"));
        assert_eq!(lower(Some("auto"), Some("high")), Some("high"));
        assert_eq!(lower(Some("low"), None), Some("low"));
        assert_eq!(lower(Some("auto"), None), None);
    }

    #[test]
    fn playbook_xhigh_under_claude_max_applies_xhigh() {
        let e = env("effort-xhigh-max");
        e.settings(r#"{"maxEffortLevel":"max","theme":"dark"}"#);
        assert_eq!(e.sync("xhigh"), Synced::Set("xhigh".into()));
        assert_eq!(e.cap().as_deref(), Some("xhigh"));
        assert_eq!(
            effective("xhigh", claude_cap(&e.home, &e.claude).as_deref()).as_deref(),
            Some("xhigh")
        );
    }

    #[test]
    fn playbook_xhigh_under_claude_medium_keeps_medium_and_writes_nothing() {
        let e = env("effort-xhigh-medium");
        e.settings(r#"{"maxEffortLevel":"medium"}"#);
        assert_eq!(e.sync("xhigh"), Synced::Deferred("medium".into()));
        assert_eq!(e.cap().as_deref(), Some("medium"));
        assert!(!marker_path(&e.home).exists());
        assert_eq!(
            effective("xhigh", claude_cap(&e.home, &e.claude).as_deref()).as_deref(),
            Some("medium")
        );
    }

    #[test]
    fn auto_restores_the_value_playbook_replaced() {
        let e = env("effort-restore");
        e.settings(r#"{"maxEffortLevel":"max"}"#);
        e.sync("low");
        assert_eq!(e.cap().as_deref(), Some("low"));
        assert_eq!(claude_cap(&e.home, &e.claude).as_deref(), Some("max"));
        assert_eq!(e.sync("auto"), Synced::Cleared);
        assert_eq!(e.cap().as_deref(), Some("max"));
        assert!(!marker_path(&e.home).exists());
    }

    #[test]
    fn auto_removes_a_cap_when_there_was_none_before() {
        let e = env("effort-none-before");
        e.sync("low");
        assert_eq!(e.sync("auto"), Synced::Cleared);
        assert_eq!(e.cap(), None);
    }

    #[test]
    fn raising_playbook_above_the_user_cap_gives_the_user_cap_back() {
        let e = env("effort-raise");
        e.settings(r#"{"maxEffortLevel":"high"}"#);
        e.sync("low");
        assert_eq!(e.cap().as_deref(), Some("low"));
        assert_eq!(e.sync("max"), Synced::Deferred("high".into()));
        assert_eq!(e.cap().as_deref(), Some("high"));
    }

    #[test]
    fn the_same_level_twice_changes_nothing() {
        let e = env("effort-twice");
        e.sync("high");
        assert_eq!(e.sync("high"), Synced::Unchanged);
    }

    #[test]
    fn auto_keeps_a_cap_the_user_set_by_hand() {
        let e = env("effort-hand");
        e.settings(r#"{"maxEffortLevel":"high"}"#);
        assert_eq!(e.sync("auto"), Synced::Unchanged);
        assert_eq!(e.cap().as_deref(), Some("high"));
    }

    #[test]
    fn the_user_changing_our_value_is_respected() {
        let e = env("effort-edited");
        e.sync("low");
        e.settings(r#"{"maxEffortLevel":"max"}"#);
        assert_eq!(e.sync("auto"), Synced::Unchanged);
        assert_eq!(e.cap().as_deref(), Some("max"));
    }

    #[test]
    fn an_old_bare_marker_still_works() {
        let e = env("effort-old-marker");
        e.settings(r#"{"maxEffortLevel":"low"}"#);
        fs::create_dir_all(marker_path(&e.home).parent().unwrap()).unwrap();
        fs::write(marker_path(&e.home), "low\n").unwrap();
        assert_eq!(e.sync("auto"), Synced::Cleared);
        assert_eq!(e.cap(), None);
    }

    #[test]
    fn a_broken_settings_file_is_left_alone() {
        let e = env("effort-broken");
        e.settings("{not json");
        assert!(sync(&e.home, &e.claude, "medium").is_err());
        assert_eq!(
            fs::read_to_string(e.claude.join("settings.json")).unwrap(),
            "{not json"
        );
    }

    #[test]
    fn status_shows_both_sides_and_the_winner() {
        let e = env("effort-status");
        e.settings(r#"{"maxEffortLevel":"medium"}"#);
        let out = run_set(&e.home, &e.claude, "xhigh").unwrap();
        assert!(out.contains("already caps at medium"), "{out}");
        let s = run_status(&e.home, &e.claude);
        assert!(s.contains("playbook effort.max: xhigh"), "{s}");
        assert!(s.contains("Claude Code maxEffortLevel: medium"), "{s}");
        assert!(s.contains("effective ceiling: medium"), "{s}");
    }

    #[test]
    fn an_unknown_level_is_rejected_before_any_write() {
        let e = env("effort-unknown");
        assert!(run_set(&e.home, &e.claude, "turbo").is_err());
        assert!(!e.claude.join("settings.json").exists());
    }
}
