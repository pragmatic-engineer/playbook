// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Breaking change notices on the first run after an upgrade (ADR 0035
//! decisions 4.3 and 4.6). The `state` table keeps `notices/binary` and
//! `notices/plugin`, the last versions whose block was shown, so each step
//! that runs the ADR 0034 migrations shows the block once.

use crate::breaking::{self, Entry};
use crate::update::resolve::compare;
use std::cmp::Ordering;
use std::path::Path;

const BINARY_KEY: &str = "notices/binary";
const PLUGIN_KEY: &str = "notices/plugin";

/// Set by the install script when it already printed the block.
const SHOWN_ENV: &str = "PLAYBOOK_BREAKING_SHOWN";

fn shown(root: &Path, key: &str) -> Option<String> {
    crate::state::list_read_only(root, key)
        .ok()?
        .into_iter()
        .find_map(|(k, v)| (k == key && !v.is_empty()).then_some(v))
}

/// Whether an older binary ran here: it left a migration record or old
/// state files, though 0.26 and earlier write no version record.
fn older_binary_ran(root: &Path) -> bool {
    crate::state::pending_legacy(root) > 0
        || crate::state::list_read_only(root, "migrations/").is_ok_and(|rows| !rows.is_empty())
}

/// Records `version` as shown under `key`, in its own transaction, unless
/// it is already there.
fn mark(root: &Path, key: &str, before: Option<&str>, version: &str) {
    if before != Some(version) {
        let _ = crate::state::set(root, key, version);
    }
}

/// The playbook entries this binary has not shown yet, oldest first. The
/// start is `notices/binary`, else `recorded` (`versions/binary`), else
/// `0.0.0` when an older binary ran here. A fresh install starts at this
/// binary and shows nothing. Moves `notices/binary` up to this binary, and
/// returns nothing when the install script already showed the block or
/// after a downgrade.
pub fn binary(root: &Path, recorded: Option<&str>) -> Vec<Entry> {
    let running = env!("CARGO_PKG_VERSION");
    let before = shown(root, BINARY_KEY);
    let from = match before.as_deref().or(recorded) {
        Some(v) => v.to_string(),
        None if older_binary_ran(root) => "0.0.0".to_string(),
        None => running.to_string(),
    };
    if compare(&from, running) == Ordering::Greater {
        return Vec::new();
    }
    let entries = breaking::entries_between(&from, running);
    mark(root, BINARY_KEY, before.as_deref(), running);
    if std::env::var(SHOWN_ENV).is_ok_and(|v| v == "1") {
        return Vec::new();
    }
    entries
}

/// The plugin entries the resolved plugin lists in `metadata.breakingChanges`
/// above `notices/plugin`, else above `recorded` (`versions/plugin`). The
/// first plugin seen shows nothing. Moves `notices/plugin` up to the
/// resolved plugin's version. An entry this binary cannot read is an error
/// that says to update the binary, and records nothing, so it shows again.
pub fn plugin(
    root: &Path,
    claude_home: &Path,
    recorded: Option<&str>,
) -> Result<Vec<Entry>, String> {
    let env = std::env::var("CLAUDE_PLUGIN_ROOT").ok();
    let Some(dir) = crate::plugin_root::find(env.as_deref(), claude_home) else {
        return Ok(Vec::new());
    };
    let Some(manifest) = std::fs::read_to_string(dir.join(".claude-plugin/plugin.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
    else {
        return Ok(Vec::new());
    };
    let Some(to) = manifest["version"].as_str() else {
        return Ok(Vec::new());
    };
    let before = shown(root, PLUGIN_KEY);
    let from = before.as_deref().or(recorded).unwrap_or(to).to_string();
    if compare(&from, to) == Ordering::Greater {
        return Ok(Vec::new());
    }
    let entries = breaking::plugin_entries(&manifest, &from, to)?;
    mark(root, PLUGIN_KEY, before.as_deref(), to);
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;

    fn root(tag: &str) -> std::path::PathBuf {
        let home = scratch_dir(tag);
        let root = crate::common::paths::playbook_root_from(&home);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_fresh_install_shows_nothing_and_records_this_binary() {
        // Arrange
        let root = root("notices-fresh");

        // Act
        let first = binary(&root, None);

        // Assert
        assert!(first.is_empty());
        assert_eq!(
            shown(&root, BINARY_KEY).as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn an_older_binary_without_a_record_starts_from_the_beginning_once() {
        // Arrange: a migration record with no version record, as 0.26 left.
        let root = root("notices-old");
        crate::state::set(&root, "migrations/shipped/system-prompt", "abc").unwrap();

        // Act
        let first = binary(&root, None);
        let again = binary(&root, None);

        // Assert
        assert_eq!(
            first,
            breaking::entries_between("0.0.0", env!("CARGO_PKG_VERSION"))
        );
        assert!(again.is_empty());
    }

    #[test]
    fn the_shown_version_wins_and_a_downgrade_shows_and_writes_nothing() {
        // Arrange
        let root = root("notices-order");
        crate::state::set(&root, BINARY_KEY, "999.0.0").unwrap();

        // Act
        let entries = binary(&root, Some("0.0.1"));

        // Assert
        assert!(entries.is_empty());
        assert_eq!(shown(&root, BINARY_KEY).as_deref(), Some("999.0.0"));
    }

    #[test]
    fn plugin_entries_come_from_the_manifest_above_the_shown_version() {
        // Arrange
        let home = scratch_dir("notices-plugin");
        let root = crate::common::paths::playbook_root_from(&home);
        fs::create_dir_all(&root).unwrap();
        let claude = home.join(".claude");
        let dir = claude.join("plugins/cache/pragmatic-engineer/playbook/0.40.0");
        fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        fs::create_dir_all(dir.join("skills")).unwrap();
        let manifest = serde_json::json!({
            "name": "playbook",
            "version": "0.40.0",
            "metadata": {"breakingChanges": [
                {"version": "0.35.0", "component": "plugin", "summary": "old", "action": "x"},
                {"version": "0.40.0", "component": "plugin", "summary": "new", "action": "y"},
                {"version": "0.40.0", "component": "future", "summary": "skip", "action": "z"}
            ]}
        });
        fs::write(dir.join(".claude-plugin/plugin.json"), manifest.to_string()).unwrap();

        // Act
        let first_sight = plugin(&root, &claude, None).unwrap();
        crate::state::set(&root, PLUGIN_KEY, "0.36.0").unwrap();
        let upgraded = plugin(&root, &claude, None).unwrap();
        let again = plugin(&root, &claude, None).unwrap();
        let mut unreadable = manifest.clone();
        unreadable["version"] = "0.41.0".into();
        unreadable["metadata"]["breakingChanges"][1]["summary"] = serde_json::Value::Null;
        fs::write(
            dir.join(".claude-plugin/plugin.json"),
            unreadable.to_string(),
        )
        .unwrap();
        let err = plugin(&root, &claude, None).unwrap_err();

        // Assert
        assert!(err.contains("run playbook update"), "{err}");
        assert_eq!(shown(&root, PLUGIN_KEY).as_deref(), Some("0.40.0"));
        assert!(first_sight.is_empty());
        let summaries: Vec<&str> = upgraded.iter().map(|e| e.summary.as_str()).collect();
        assert_eq!(summaries, ["new"]);
        assert!(again.is_empty());
    }
}
