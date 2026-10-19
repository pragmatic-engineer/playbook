// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Hook entries setup wrote for a hook that has since been removed. Only the
//! bare `playbook hook <name>` form is ours; a user's own command stays.

use serde_json::Value;
use std::path::Path;

/// Hooks removed from the binary's wiring. Each keeps a hidden no-op
/// subcommand for one release so an old entry does not print an error.
pub const RETIRED: [&str; 1] = ["no-slop-guard"];

fn is_retired_entry(entry: &Value) -> bool {
    entry
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|cmd| {
            RETIRED
                .iter()
                .any(|name| cmd == format!("{}{name}", super::wire::HOOK_COMMAND_PREFIX))
        })
}

fn count(settings: &Value) -> usize {
    let events = settings.get("hooks").and_then(Value::as_object);
    events
        .into_iter()
        .flat_map(|e| e.values())
        .flat_map(|groups| groups.as_array().into_iter().flatten())
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter(|e| is_retired_entry(e))
        .count()
}

/// Removes the retired entries after a backup under `epoch`. Returns how many
/// went. A missing or invalid file is left alone.
pub fn remove(settings_path: &Path, epoch: u64) -> std::io::Result<usize> {
    let Some(mut settings) = std::fs::read_to_string(settings_path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    else {
        return Ok(0);
    };
    let found = count(&settings);
    if found == 0 {
        return Ok(0);
    }
    super::dead_hooks::drop_entries(&mut settings, is_retired_entry);
    let rendered = format!("{settings:#}\n");
    crate::init::run::backup_then_write(settings_path, &rendered, &[], epoch)?;
    Ok(found)
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
                {"type": "command", "command": "playbook hook no-slop-guard"}]},
            {"matcher": "Write", "hooks": [
                {"type": "command", "command": "my-wrapper playbook hook no-slop-guard"}]}
        ]}});
        fs::write(&path, format!("{settings:#}\n")).unwrap();

        // Act
        let first = remove(&path, 100).unwrap();
        let second = remove(&path, 101).unwrap();

        // Assert
        assert_eq!((first, second), (2, 0));
        let after: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let text = after.to_string();
        assert!(text.contains("policy-guard"));
        assert!(text.contains("my-wrapper playbook hook no-slop-guard"));
        assert_eq!(count(&after), 0);
        assert_eq!(after["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
        assert!(dir.join("settings.json.bak.100").is_file());
    }

    #[test]
    fn a_missing_file_is_left_alone() {
        let dir = scratch_dir("retired-hooks-none");
        assert_eq!(remove(&dir.join("settings.json"), 1).unwrap(), 0);
    }
}
