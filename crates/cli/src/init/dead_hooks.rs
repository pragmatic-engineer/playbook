// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Hook entries in `settings.json` whose command runs a script under
//! `~/.claude/hooks/` that no longer exists (ADR 0034 decision 9). Older
//! installs placed those scripts; the binary runs the hooks now, so a missing
//! script means the entry is dead and fails on every event. An entry whose
//! script still exists is the user's own and stays.

use serde_json::Value;
use std::path::{Component, Path, PathBuf};

const INTERPRETERS: [&str; 3] = ["bash", "sh", "python3"];

/// Characters that make a command more than one program run, so a missing
/// first script does not make the whole entry dead.
const SHELL_OPERATORS: [char; 6] = [';', '&', '|', '<', '>', '`'];

/// The script `command` runs, when it is a path under a Claude `hooks`
/// folder, optionally after `bash`, `sh` or `python3`. `None` for any other
/// command, including a quoted path that holds a space and a command with a
/// shell operator or `$(`.
fn hook_script(command: &str, home: &Path, claude_home: &Path) -> Option<PathBuf> {
    if command.contains(SHELL_OPERATORS) || command.contains("$(") {
        return None;
    }
    let mut tokens = command.split_whitespace();
    let mut path = tokens.next()?;
    if INTERPRETERS.contains(&path) {
        path = tokens.next()?;
    }
    let path = unquote(path)?;
    let home_str = home.to_string_lossy();
    let expanded = ["~/", "$HOME/", "${HOME}/"]
        .iter()
        .find_map(|prefix| path.strip_prefix(prefix))
        .map(|rest| format!("{home_str}/{rest}"))
        .unwrap_or_else(|| path.to_string());
    let script = PathBuf::from(expanded);
    let under_hooks = [claude_home.join("hooks"), home.join(".claude/hooks")]
        .iter()
        .any(|dir| script.starts_with(dir) && script != *dir);
    let plain = script.is_absolute()
        && script
            .components()
            .all(|c| !matches!(c, Component::ParentDir | Component::CurDir));
    (under_hooks && plain).then_some(script)
}

/// `token` without one pair of matching quotes. `None` when a quote opens
/// and does not close in the same token, so a path with a space is skipped.
fn unquote(token: &str) -> Option<&str> {
    for quote in ['"', '\''] {
        if let Some(rest) = token.strip_prefix(quote) {
            return rest.strip_suffix(quote);
        }
    }
    Some(token)
}

/// Whether `command` runs a script under a Claude `hooks` folder that is gone.
pub fn is_dead(command: &str, home: &Path, claude_home: &Path) -> bool {
    hook_script(command, home, claude_home).is_some_and(|script| !script.exists())
}

/// The dead hook commands in `settings`, each once, in file order.
pub fn dead_commands(settings: &Value, home: &Path, claude_home: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let events = settings.get("hooks").and_then(Value::as_object);
    for groups in events.into_iter().flat_map(|e| e.values()) {
        for group in groups.as_array().into_iter().flatten() {
            for entry in group
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(cmd) = entry.get("command").and_then(Value::as_str) else {
                    continue;
                };
                if is_dead(cmd, home, claude_home) && !out.iter().any(|c| c == cmd) {
                    out.push(cmd.to_string());
                }
            }
        }
    }
    out
}

fn read_settings(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// What removing the dead entries would change, without writing, or `None`.
pub fn pending(settings_path: &Path, home: &Path, claude_home: &Path) -> Option<String> {
    let dead = dead_commands(&read_settings(settings_path)?, home, claude_home);
    (!dead.is_empty()).then(|| {
        format!(
            "settings.json has {} hook(s) whose script is gone: {}",
            dead.len(),
            dead.join(", ")
        )
    })
}

/// Removes every hook entry `is_target` accepts, then any matcher group or
/// event that leaves empty. A group or event empty before stays.
pub(super) fn drop_entries(settings: &mut Value, is_target: impl Fn(&Value) -> bool) {
    if let Some(events) = settings.get_mut("hooks").and_then(Value::as_object_mut) {
        let mut emptied_events = Vec::new();
        for (event, groups) in events.iter_mut() {
            let Some(groups) = groups.as_array_mut() else {
                continue;
            };
            let before = groups.len();
            groups.retain_mut(|group| {
                let Some(entries) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                    return true;
                };
                let had = entries.len();
                entries.retain(|e| !is_target(e));
                entries.len() == had || !entries.is_empty()
            });
            if groups.is_empty() && before > 0 {
                emptied_events.push(event.clone());
            }
        }
        let emptied = !emptied_events.is_empty();
        for event in emptied_events {
            events.remove(&event);
        }
        if emptied && events.is_empty() {
            settings.as_object_mut().map(|root| root.remove("hooks"));
        }
    }
}

/// Removes the dead entries from `settings_path` after a backup under
/// `epoch`, and drops a matcher group or event that this leaves empty. A
/// group or event that was empty before stays. Returns the removed commands.
/// A missing or invalid file is left alone.
pub fn remove(
    settings_path: &Path,
    home: &Path,
    claude_home: &Path,
    epoch: u64,
) -> std::io::Result<Vec<String>> {
    let Some(mut settings) = read_settings(settings_path) else {
        return Ok(Vec::new());
    };
    let dead = dead_commands(&settings, home, claude_home);
    if dead.is_empty() {
        return Ok(dead);
    }
    drop_entries(&mut settings, |e| {
        e.get("command")
            .and_then(Value::as_str)
            .is_some_and(|c| dead.iter().any(|d| d == c))
    });
    let rendered = format!("{settings:#}\n");
    crate::init::run::backup_then_write(settings_path, &rendered, &[], epoch)?;
    Ok(dead)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use serde_json::json;
    use std::fs;

    fn setup(tag: &str) -> (PathBuf, PathBuf) {
        let home = scratch_dir(tag);
        let claude_home = home.join(".claude");
        fs::create_dir_all(claude_home.join("hooks")).unwrap();
        (home, claude_home)
    }

    #[test]
    fn a_missing_script_under_the_hooks_folder_is_dead_in_every_spelling() {
        let (home, ch) = setup("dead-spellings");
        let abs = format!("{}/hooks/gone.sh", ch.display());
        for cmd in [
            "~/.claude/hooks/gone.sh",
            "bash ~/.claude/hooks/gone.sh",
            "sh $HOME/.claude/hooks/gone.sh --flag",
            "~/.claude/hooks/gone.sh 2",
            "python3 \"${HOME}/.claude/hooks/gone.py\"",
            abs.as_str(),
        ] {
            assert!(is_dead(cmd, &home, &ch), "{cmd}");
        }
    }

    #[test]
    fn a_live_script_or_any_other_command_is_kept() {
        let (home, ch) = setup("dead-kept");
        fs::write(ch.join("hooks/mine.sh"), "#!/bin/sh\n").unwrap();
        for cmd in [
            "bash ~/.claude/hooks/mine.sh",
            "playbook hook session-init",
            "node ~/.claude/hooks/gone.js",
            "bash ~/scripts/gone.sh",
            "bash \"~/.claude/hooks/my script.sh\"",
            "bash ~/.claude/hooks/../gone.sh",
            "bash ~/.claude/hooks/",
            "~/.claude/hooks/mine.sh; true",
            "bash ~/.claude/hooks/gone.sh || ~/bin/fallback.sh",
            "bash ~/.claude/hooks/gone.sh|jq .",
            "bash $(echo ~/.claude/hooks/gone.sh)",
        ] {
            assert!(!is_dead(cmd, &home, &ch), "{cmd}");
        }
    }

    #[test]
    fn removal_keeps_live_entries_backs_up_and_is_idempotent() {
        // Arrange
        let (home, ch) = setup("dead-remove");
        fs::write(ch.join("hooks/mine.sh"), "#!/bin/sh\n").unwrap();
        let settings = ch.join("settings.json");
        let original = json!({
            "model": "x",
            "hooks": {
                "Notification": [],
                "PreToolUse": [
                    {"matcher": "Bash", "hooks": [
                        {"type": "command", "command": "bash ~/.claude/hooks/gone.sh"},
                        {"type": "command", "command": "bash ~/.claude/hooks/mine.sh"}
                    ]}
                ],
                "Stop": [
                    {"hooks": [{"type": "command", "command": "~/.claude/hooks/old-stop.sh"}]}
                ]
            }
        });
        fs::write(&settings, format!("{original:#}\n")).unwrap();
        assert!(pending(&settings, &home, &ch)
            .unwrap()
            .contains("2 hook(s)"));

        // Act
        let removed = remove(&settings, &home, &ch, 1000).unwrap();
        let again = remove(&settings, &home, &ch, 2000).unwrap();

        // Assert
        assert_eq!(removed.len(), 2);
        assert!(again.is_empty());
        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(
            after,
            json!({
                "model": "x",
                "hooks": {"Notification": [], "PreToolUse": [{"matcher": "Bash", "hooks": [
                    {"type": "command", "command": "bash ~/.claude/hooks/mine.sh"}
                ]}]}
            })
        );
        let backup: Value =
            serde_json::from_str(&fs::read_to_string(ch.join("settings.json.bak.1000")).unwrap())
                .unwrap();
        assert_eq!(backup, original);
        assert!(!ch.join("settings.json.bak.2000").exists());
        assert!(pending(&settings, &home, &ch).is_none());
    }

    #[test]
    fn a_missing_or_invalid_settings_file_is_left_alone() {
        let (home, ch) = setup("dead-missing");
        let settings = ch.join("settings.json");
        assert_eq!(
            remove(&settings, &home, &ch, 1).unwrap(),
            Vec::<String>::new()
        );
        assert!(!settings.exists());
        fs::write(&settings, "{ not json").unwrap();
        assert!(remove(&settings, &home, &ch, 1).unwrap().is_empty());
        assert_eq!(fs::read_to_string(&settings).unwrap(), "{ not json");
        assert!(pending(&settings, &home, &ch).is_none());
    }
}
