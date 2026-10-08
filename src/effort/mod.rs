// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook effort`: the user's ceiling on effort.
//!
//! The config key `effort.max` (`auto`, `low`, `medium`, `high`, `xhigh`,
//! `max`) is written to the `maxEffortLevel` setting in
//! `~/.claude/settings.json`. Claude Code applies that cap to every session,
//! including the effort a command, skill or agent sets in its own
//! frontmatter, so an `xhigh` agent runs at `medium` under a `medium` cap.
//! `auto` means playbook sets no cap and the shipped defaults apply. Claude
//! Code 2.1.267 or later is needed for the setting to take effect.
//!
//! Playbook removes the cap on `auto` only when it wrote the value itself. A
//! `maxEffortLevel` the user set by hand is never touched.

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

/// What `sync` did to `settings.json`.
#[derive(Debug, PartialEq, Eq)]
pub enum Synced {
    /// The cap was written or changed.
    Set(String),
    /// A cap that playbook wrote earlier was removed.
    Cleared,
    /// Nothing needed to change.
    Unchanged,
}

fn marker_path(home: &Path) -> PathBuf {
    playbook_root_from(home).join(".effort-owned")
}

/// The configured level, `auto` when no tier sets one.
pub fn configured(home: &Path) -> String {
    match config::resolve(KEY, home, None) {
        Ok((Value::String(level), _)) => level,
        _ => "auto".to_string(),
    }
}

/// Make `settings.json` agree with `level`.
pub fn sync(home: &Path, claude_home: &Path, level: &str) -> io::Result<Synced> {
    let settings_path = claude_home.join("settings.json");
    let marker = marker_path(home);
    let owned = fs::read_to_string(&marker)
        .ok()
        .map(|s| s.trim().to_string());

    let mut root = match fs::read_to_string(&settings_path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => map,
            _ => {
                return Err(io::Error::other(format!(
                    "{} is not a JSON object, left unchanged",
                    settings_path.display()
                )))
            }
        },
        Err(err) if err.kind() == io::ErrorKind::NotFound => Map::new(),
        Err(err) => return Err(err),
    };
    let current = root
        .get("maxEffortLevel")
        .and_then(Value::as_str)
        .map(str::to_string);

    if level == "auto" {
        let ours = owned.is_some() && owned == current;
        if owned.is_some() {
            let _ = fs::remove_file(&marker);
        }
        if !ours {
            return Ok(Synced::Unchanged);
        }
        root.remove("maxEffortLevel");
        write_settings(&settings_path, root)?;
        return Ok(Synced::Cleared);
    }

    if current.as_deref() == Some(level) {
        write_marker(&marker, level)?;
        return Ok(Synced::Unchanged);
    }
    root.insert(
        "maxEffortLevel".to_string(),
        Value::String(level.to_string()),
    );
    write_settings(&settings_path, root)?;
    write_marker(&marker, level)?;
    Ok(Synced::Set(level.to_string()))
}

fn write_marker(marker: &Path, level: &str) -> io::Result<()> {
    if let Some(dir) = marker.parent() {
        fs::create_dir_all(dir)?;
    }
    write_atomic(marker, &format!("{level}\n"))
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
        ("auto", Synced::Cleared) => "effort cap removed, playbook defaults apply".to_string(),
        ("auto", _) => "effort set to auto, playbook defaults apply".to_string(),
        (l, Synced::Set(_)) => format!(
            "effort capped at {l}: anything set higher runs at {l} (needs Claude Code 2.1.267 or later)"
        ),
        (l, _) => format!("effort already capped at {l}"),
    })
}

/// `playbook effort` with no level: what is configured and what is applied.
pub fn run_status(home: &Path, claude_home: &Path) -> String {
    let level = configured(home);
    let applied = fs::read_to_string(claude_home.join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| {
            v.get("maxEffortLevel")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let applied_text = applied.as_deref().unwrap_or("none");
    let mut out = format!("effort.max: {level}\nmaxEffortLevel in settings.json: {applied_text}");
    let in_sync = match (level.as_str(), applied.as_deref()) {
        ("auto", _) => true,
        (l, Some(a)) => l == a,
        _ => false,
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

    fn read_cap(claude: &Path) -> Option<String> {
        let text = fs::read_to_string(claude.join("settings.json")).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        v.get("maxEffortLevel")?.as_str().map(str::to_string)
    }

    #[test]
    fn a_level_is_written_and_other_settings_survive() {
        let home = scratch_dir("effort-set");
        let claude = home.join(".claude");
        fs::create_dir_all(&claude).unwrap();
        fs::write(claude.join("settings.json"), r#"{"theme":"dark"}"#).unwrap();

        let out = sync(&home, &claude, "medium").unwrap();

        assert_eq!(out, Synced::Set("medium".into()));
        assert_eq!(read_cap(&claude).as_deref(), Some("medium"));
        let v: Value =
            serde_json::from_str(&fs::read_to_string(claude.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(v["theme"], "dark");
    }

    #[test]
    fn the_same_level_twice_changes_nothing() {
        let home = scratch_dir("effort-twice");
        let claude = home.join(".claude");
        sync(&home, &claude, "high").unwrap();
        assert_eq!(sync(&home, &claude, "high").unwrap(), Synced::Unchanged);
    }

    #[test]
    fn auto_removes_a_cap_playbook_wrote() {
        let home = scratch_dir("effort-clear");
        let claude = home.join(".claude");
        sync(&home, &claude, "low").unwrap();
        assert_eq!(sync(&home, &claude, "auto").unwrap(), Synced::Cleared);
        assert_eq!(read_cap(&claude), None);
    }

    #[test]
    fn auto_keeps_a_cap_the_user_set_by_hand() {
        let home = scratch_dir("effort-hand");
        let claude = home.join(".claude");
        fs::create_dir_all(&claude).unwrap();
        fs::write(claude.join("settings.json"), r#"{"maxEffortLevel":"high"}"#).unwrap();
        assert_eq!(sync(&home, &claude, "auto").unwrap(), Synced::Unchanged);
        assert_eq!(read_cap(&claude).as_deref(), Some("high"));
    }

    #[test]
    fn auto_after_the_user_changed_our_value_keeps_theirs() {
        let home = scratch_dir("effort-edited");
        let claude = home.join(".claude");
        sync(&home, &claude, "low").unwrap();
        fs::write(claude.join("settings.json"), r#"{"maxEffortLevel":"max"}"#).unwrap();
        assert_eq!(sync(&home, &claude, "auto").unwrap(), Synced::Unchanged);
        assert_eq!(read_cap(&claude).as_deref(), Some("max"));
    }

    #[test]
    fn a_broken_settings_file_is_left_alone() {
        let home = scratch_dir("effort-broken");
        let claude = home.join(".claude");
        fs::create_dir_all(&claude).unwrap();
        fs::write(claude.join("settings.json"), "{not json").unwrap();
        assert!(sync(&home, &claude, "medium").is_err());
        assert_eq!(
            fs::read_to_string(claude.join("settings.json")).unwrap(),
            "{not json"
        );
    }

    #[test]
    fn an_unknown_level_is_rejected_before_any_write() {
        let home = scratch_dir("effort-unknown");
        let claude = home.join(".claude");
        assert!(run_set(&home, &claude, "turbo").is_err());
        assert!(!claude.join("settings.json").exists());
    }
}
