// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook effort`: playbook's own ceiling on effort.
//!
//! The config key `maxEffortLevel` (`low`, `medium`, `high`, `xhigh`, `max`)
//! is playbook's setting, with the same name and values as Claude Code's own
//! key. `max` is the default and means playbook sets no ceiling. Playbook only
//! ever reads Claude Code's key and never writes to its settings files.
//!
//! The effective ceiling is the lower of the two. When playbook's is lower,
//! the launcher (`ccc`, `ccd`) passes it to that one session with `--settings`,
//! which Claude Code applies to command, skill and agent effort alike. When
//! Claude Code's is as low or lower, or playbook is `max`, the launcher adds
//! nothing. A session started without the launcher gets no playbook ceiling.

use crate::config::{self, write};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// The config key.
pub const KEY: &str = "maxEffortLevel";

/// Every accepted value, lowest first. Same set as Claude Code's key.
pub const LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// Position of `level` from lowest to highest, `None` for an unknown level.
fn rank(level: &str) -> Option<usize> {
    LEVELS.iter().position(|l| *l == level)
}

/// Playbook's `max` sets no ceiling, so it counts as unset.
fn playbook_ceiling(level: &str) -> Option<&str> {
    (level != "max").then_some(level)
}

/// The lower of two levels. An unknown or missing side loses.
fn lower<'a>(a: Option<&'a str>, b: Option<&'a str>) -> Option<&'a str> {
    let ra = a.and_then(|l| rank(l).map(|r| (l, r)));
    let rb = b.and_then(|l| rank(l).map(|r| (l, r)));
    match (ra, rb) {
        (Some((la, ra)), Some((lb, rb))) => Some(if ra <= rb { la } else { lb }),
        (Some((l, _)), None) | (None, Some((l, _))) => Some(l),
        (None, None) => None,
    }
}

/// Who sets the effective ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Winner {
    Playbook,
    ClaudeCode,
    /// Neither sets one, so each command, skill and agent keeps its own effort.
    Neither,
}

/// Both ceilings and the one that applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ceiling {
    pub playbook: String,
    pub claude: Option<String>,
    pub effective: Option<String>,
    pub winner: Winner,
}

impl Ceiling {
    pub fn new(playbook: &str, claude: Option<&str>) -> Self {
        let own = playbook_ceiling(playbook);
        let effective = lower(own, claude).map(str::to_string);
        let winner = match (own.and_then(rank), claude.and_then(rank)) {
            (None, None) => Winner::Neither,
            (Some(_), None) => Winner::Playbook,
            (None, Some(_)) => Winner::ClaudeCode,
            (Some(p), Some(c)) if p < c => Winner::Playbook,
            _ => Winner::ClaudeCode,
        };
        Ceiling {
            playbook: playbook.to_string(),
            claude: claude.map(str::to_string),
            effective,
            winner,
        }
    }
}

/// The configured level, `max` (no ceiling) when no tier sets one.
pub fn configured(home: &Path) -> String {
    match config::resolve(KEY, home, None) {
        Ok((Value::String(level), _)) => level,
        _ => "max".to_string(),
    }
}

fn cap_in(path: &Path) -> Option<String> {
    let v: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    v.get("maxEffortLevel")?.as_str().map(str::to_string)
}

/// The project root Claude Code would use for `cwd`: the nearest ancestor
/// holding `.git`, else `cwd` itself.
fn project_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .find(|d| d.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

/// Claude Code's own `maxEffortLevel`, read from the user, project and local
/// settings files. The lowest one wins, as it does inside Claude Code.
/// Managed settings are not read here: Claude Code applies those itself.
pub fn claude_cap(claude_home: &Path, cwd: &Path) -> Option<String> {
    let root = project_root(cwd);
    let found = [
        claude_home.join("settings.json"),
        root.join(".claude").join("settings.json"),
        root.join(".claude").join("settings.local.json"),
    ]
    .iter()
    .filter_map(|p| cap_in(p))
    .collect::<Vec<_>>();
    found
        .iter()
        .filter_map(|l| rank(l).map(|r| (l, r)))
        .min_by_key(|(_, r)| *r)
        .map(|(l, _)| l.clone())
}

/// Both ceilings for a session started in `cwd`.
pub fn ceiling(home: &Path, claude_home: &Path, cwd: &Path) -> Ceiling {
    Ceiling::new(&configured(home), claude_cap(claude_home, cwd).as_deref())
}

/// The inline `--settings` value the launcher passes, or `None` when it has
/// nothing to add: playbook is `max`, or Claude Code's ceiling is as low or
/// lower.
pub fn launcher_settings(home: &Path, claude_home: &Path, cwd: &Path) -> Option<String> {
    let c = ceiling(home, claude_home, cwd);
    if c.winner != Winner::Playbook {
        return None;
    }
    Some(serde_json::json!({ "maxEffortLevel": c.playbook }).to_string())
}

/// `playbook effort <level>`: store playbook's level. Nothing else is written.
pub fn run_set(home: &Path, level: &str) -> Result<String, String> {
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
    Ok(if level == "max" {
        "maxEffortLevel set to max, playbook sets no ceiling".to_string()
    } else {
        format!(
            "playbook maxEffortLevel set to {level}. Sessions started with ccc or ccd stay at or below it, \
             and never above your Claude Code maxEffortLevel. Claude Code's settings are not changed."
        )
    })
}

/// `playbook effort` with no level: both ceilings and the one that wins.
pub fn run_status(home: &Path, claude_home: &Path, cwd: &Path, json: bool) -> String {
    let c = ceiling(home, claude_home, cwd);
    let winner = match c.winner {
        Winner::Playbook => "playbook",
        Winner::ClaudeCode => "claude-code",
        Winner::Neither => "neither",
    };
    if json {
        return serde_json::json!({
            "playbook": c.playbook,
            "claudeCode": c.claude,
            "effective": c.effective,
            "winner": winner,
        })
        .to_string();
    }
    format!(
        "playbook maxEffortLevel: {}\nClaude Code maxEffortLevel: {}\neffective ceiling: {} ({winner})",
        c.playbook,
        c.claude.as_deref().unwrap_or("none"),
        c.effective
            .as_deref()
            .unwrap_or("none, shipped defaults apply"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    fn setup(tag: &str) -> (PathBuf, PathBuf) {
        let home = scratch_dir(tag);
        let claude = home.join(".claude");
        fs::create_dir_all(&claude).unwrap();
        (home, claude)
    }

    #[test]
    fn the_lower_of_two_levels_wins() {
        assert_eq!(lower(Some("xhigh"), Some("max")), Some("xhigh"));
        assert_eq!(lower(Some("xhigh"), Some("medium")), Some("medium"));
        assert_eq!(lower(Some("low"), None), Some("low"));
        assert_eq!(lower(None, None), None);
    }

    #[test]
    fn xhigh_is_below_max() {
        assert!(rank("xhigh") < rank("max"));
        let c = Ceiling::new("xhigh", Some("max"));
        assert_eq!(c.effective.as_deref(), Some("xhigh"));
        assert_eq!(c.winner, Winner::Playbook);
    }

    #[test]
    fn claude_code_medium_beats_playbook_xhigh() {
        let c = Ceiling::new("xhigh", Some("medium"));
        assert_eq!(c.effective.as_deref(), Some("medium"));
        assert_eq!(c.winner, Winner::ClaudeCode);
    }

    #[test]
    fn equal_levels_leave_it_to_claude_code() {
        assert_eq!(
            Ceiling::new("high", Some("high")).winner,
            Winner::ClaudeCode
        );
    }

    #[test]
    fn max_defers_to_claude_code_or_to_nobody() {
        assert_eq!(Ceiling::new("max", Some("low")).winner, Winner::ClaudeCode);
        assert_eq!(Ceiling::new("max", None).winner, Winner::Neither);
        assert_eq!(Ceiling::new("max", None).effective, None);
    }

    #[test]
    fn the_launcher_adds_settings_only_when_playbook_is_lower() {
        let (home, claude) = setup("effort-launcher");
        let cwd = home.join("proj");
        fs::create_dir_all(&cwd).unwrap();
        run_set(&home, "xhigh").unwrap();

        // No Claude Code cap: playbook's applies.
        assert_eq!(
            launcher_settings(&home, &claude, &cwd).as_deref(),
            Some(r#"{"maxEffortLevel":"xhigh"}"#)
        );
        // Claude Code at max: still playbook's.
        fs::write(claude.join("settings.json"), r#"{"maxEffortLevel":"max"}"#).unwrap();
        assert!(launcher_settings(&home, &claude, &cwd).is_some());
        // Claude Code at medium: nothing to add.
        fs::write(
            claude.join("settings.json"),
            r#"{"maxEffortLevel":"medium"}"#,
        )
        .unwrap();
        assert_eq!(launcher_settings(&home, &claude, &cwd), None);
    }

    #[test]
    fn playbook_max_adds_nothing() {
        let (home, claude) = setup("effort-max");
        assert_eq!(launcher_settings(&home, &claude, &home), None);
    }

    #[test]
    fn the_lowest_claude_code_scope_counts() {
        let (home, claude) = setup("effort-scopes");
        let proj = home.join("proj");
        fs::create_dir_all(proj.join(".claude")).unwrap();
        fs::create_dir_all(proj.join(".git")).unwrap();
        fs::write(claude.join("settings.json"), r#"{"maxEffortLevel":"max"}"#).unwrap();
        fs::write(
            proj.join(".claude/settings.local.json"),
            r#"{"maxEffortLevel":"low"}"#,
        )
        .unwrap();
        let sub = proj.join("src");
        fs::create_dir_all(&sub).unwrap();
        assert_eq!(claude_cap(&claude, &sub).as_deref(), Some("low"));
    }

    #[test]
    fn setting_a_level_never_touches_claude_code_settings() {
        let (home, claude) = setup("effort-no-write");
        let body = r#"{"maxEffortLevel":"high","theme":"dark"}"#;
        fs::write(claude.join("settings.json"), body).unwrap();
        run_set(&home, "low").unwrap();
        run_set(&home, "max").unwrap();
        assert_eq!(
            fs::read_to_string(claude.join("settings.json")).unwrap(),
            body
        );
    }

    #[test]
    fn status_shows_both_sides_and_the_winner() {
        let (home, claude) = setup("effort-status");
        fs::write(
            claude.join("settings.json"),
            r#"{"maxEffortLevel":"medium"}"#,
        )
        .unwrap();
        run_set(&home, "xhigh").unwrap();
        let s = run_status(&home, &claude, &home, false);
        assert!(s.contains("playbook maxEffortLevel: xhigh"), "{s}");
        assert!(s.contains("Claude Code maxEffortLevel: medium"), "{s}");
        assert!(s.contains("effective ceiling: medium (claude-code)"), "{s}");
        let j: Value = serde_json::from_str(&run_status(&home, &claude, &home, true)).unwrap();
        assert_eq!(j["winner"], "claude-code");
        assert_eq!(j["effective"], "medium");
    }

    #[test]
    fn an_unknown_level_is_rejected_before_any_write() {
        let (home, _) = setup("effort-unknown");
        assert!(run_set(&home, "turbo").is_err());
    }
}
