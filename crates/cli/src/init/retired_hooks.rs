// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Hook entries setup wrote for a hook that has since been removed. Only the
//! bare `playbook hook <name>` form is ours; a user's own command stays.

use serde_json::Value;
use std::path::Path;

/// Hook names setup no longer writes. `no-slop-guard` keeps a hidden no-op
/// subcommand for one release; `no-dash-guard` (its old name) has none.
const RETIRED: [&str; 2] = ["no-slop-guard", "no-dash-guard"];

/// Whether `command` is the bare entry setup wrote for a retired hook.
pub fn is_retired_command(command: &str) -> bool {
    RETIRED
        .iter()
        .any(|name| command == super::wire::bare_command(name))
}

fn is_retired_entry(entry: &Value) -> bool {
    entry
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(is_retired_command)
}

/// A line naming the retired entries still in `settings_path`, or `None`.
pub fn pending(settings_path: &Path) -> Option<String> {
    let mut settings = super::dead_hooks::read_settings(settings_path)?;
    let n = super::dead_hooks::drop_entries(&mut settings, is_retired_entry);
    (n > 0).then(|| {
        format!("{n} hook entries run a retired playbook hook (no-slop-guard or no-dash-guard); setup removes them")
    })
}

/// Removes the retired entries after a backup under `epoch`. Returns how many
/// went. A missing or invalid file is left alone.
pub fn remove(settings_path: &Path, epoch: u64) -> std::io::Result<usize> {
    let Some(mut settings) = super::dead_hooks::read_settings(settings_path) else {
        return Ok(0);
    };
    let removed = super::dead_hooks::drop_entries(&mut settings, is_retired_entry);
    if removed == 0 {
        return Ok(0);
    }
    let rendered = format!("{settings:#}\n");
    crate::init::run::backup_then_write(settings_path, &rendered, &[], epoch)?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use serde_json::json;
    use std::fs;

    #[test]
    fn removal_drops_only_the_bare_entry_and_is_idempotent() {
        // Arrange
        let dir = scratch_dir("retired-hooks");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let settings = json!({"hooks": {"PreToolUse": [
            {"matcher": "Bash", "hooks": [
                {"type": "command", "command": "playbook hook no-slop-guard"},
                {"type": "command", "command": "playbook hook policy-guard"}]},
            {"matcher": "Edit|Write", "hooks": [
                {"type": "command", "command": "playbook hook no-slop-guard"},
                {"type": "command", "command": "playbook hook no-dash-guard"}]},
            {"matcher": "Write", "hooks": [
                {"type": "command", "command": "my-wrapper playbook hook no-slop-guard"}]}
        ]}});
        fs::write(&path, format!("{settings:#}\n")).unwrap();

        // Act
        let first = remove(&path, 100).unwrap();
        let second = remove(&path, 101).unwrap();

        // Assert
        assert_eq!((first, second), (3, 0));
        let after: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let text = after.to_string();
        assert!(text.contains("policy-guard"));
        assert!(text.contains("my-wrapper playbook hook no-slop-guard"));
        assert!(!text.contains("hook no-dash-guard"));
        assert_eq!(after["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
        assert!(dir.join("settings.json.bak.100").is_file());
    }

    #[test]
    fn pending_counts_retired_entries_and_leaves_the_file_alone() {
        // Arrange
        let dir = scratch_dir("retired-hooks-pending");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let settings = json!({"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [
            {"type": "command", "command": "playbook hook no-slop-guard"}]}]}});
        let text = format!("{settings:#}\n");
        fs::write(&path, &text).unwrap();

        // Act
        let line = pending(&path);

        // Assert
        assert!(line.unwrap().starts_with("1 hook entries"));
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert_eq!(pending(&dir.join("none.json")), None);
    }

    #[test]
    fn a_missing_file_is_left_alone() {
        let dir = scratch_dir("retired-hooks-none");
        assert_eq!(remove(&dir.join("settings.json"), 1).unwrap(), 0);
    }
}
