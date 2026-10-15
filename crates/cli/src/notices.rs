// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Breaking change notices on the first run after an upgrade (ADR 0035
//! decisions 4.3 and 4.6). The `state` table keeps `notices/binary` and
//! `notices/plugin`, the last versions whose block was shown, so each step
//! that runs the ADR 0034 migrations shows the block once.
//!
//! A step reads what to show with `prepare`, runs the heal, then calls
//! `Pending::commit`, which moves each key in one transaction and keeps only
//! the entries whose key this process moved. Two processes that start
//! together show the block once between them.

use crate::breaking::{self, Entry};
use crate::update::resolve::compare;
use std::cmp::Ordering;
use std::path::Path;

const BINARY_KEY: &str = "notices/binary";
const PLUGIN_KEY: &str = "notices/plugin";

/// What a step shows: the block, and a line when the plugin lists an entry
/// this binary cannot read.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Notices {
    pub entries: Vec<Entry>,
    pub error: Option<String>,
}

impl Notices {
    /// The block and the error line, or an empty string.
    pub fn render(&self) -> String {
        let mut text = breaking::render(&self.entries);
        if let Some(err) = &self.error {
            text.push_str(&format!("playbook: {err}\n"));
        }
        text
    }
}

/// One key to move from `before` to `to`, and the entries it unlocks.
#[derive(Debug)]
struct Move {
    key: &'static str,
    before: Option<String>,
    to: String,
    entries: Vec<Entry>,
}

/// The notices one step will show, read before its heal moves the record.
#[derive(Debug, Default)]
pub struct Pending {
    moves: Vec<Move>,
    error: Option<String>,
}

/// The value of `key`. `Err` when the store cannot be read, so the caller
/// shows and records nothing and the next step tries again.
fn shown(root: &Path, key: &str) -> Result<Option<String>, ()> {
    Ok(crate::state::list_read_only(root, key)
        .map_err(|_| ())?
        .into_iter()
        .find_map(|(k, v)| (k == key && !v.is_empty()).then_some(v)))
}

/// Whether an older binary ran here: it left a migration record or old
/// state files, though 0.26 and earlier write no version record. Call it
/// before anything opens the store for writing, since that imports and
/// renames the old files.
pub fn older_binary_ran(root: &Path) -> bool {
    crate::state::pending_legacy(root) > 0
        || crate::state::list_read_only(root, "migrations/").is_ok_and(|rows| !rows.is_empty())
}

/// What the step will show, read without writing.
///
/// The playbook entries start at `notices/binary`, else `binary`
/// (`versions/binary`), else `0.0.0` when `older` says an older binary ran.
/// A fresh install starts at this binary and shows nothing.
///
/// The plugin entries come from the resolved plugin's
/// `metadata.breakingChanges`, starting at `notices/plugin`, else `plugin`
/// (`versions/plugin`). The first plugin seen shows nothing. An entry this
/// binary cannot read is an error that says to update the binary, and moves
/// nothing, so it shows again.
pub fn prepare(
    root: &Path,
    claude_home: &Path,
    binary: Option<&str>,
    plugin: Option<&str>,
    older: bool,
) -> Pending {
    let mut pending = Pending::default();
    let running = env!("CARGO_PKG_VERSION");
    if let Ok(before) = shown(root, BINARY_KEY) {
        let from = match before.as_deref().or(binary) {
            Some(v) => v,
            None if older => "0.0.0",
            None => running,
        };
        if before.as_deref() != Some(running) && compare(from, running) != Ordering::Greater {
            pending.moves.push(Move {
                key: BINARY_KEY,
                before: before.clone(),
                to: running.to_string(),
                entries: breaking::entries_between(from, running),
            });
        }
    }
    let env = std::env::var("CLAUDE_PLUGIN_ROOT").ok();
    let manifest = crate::plugin_root::find(env.as_deref(), claude_home)
        .and_then(|dir| crate::plugin_root::manifest(&dir));
    let to = manifest
        .as_ref()
        .and_then(|m| m["version"].as_str())
        .filter(|v| crate::plugin_root::is_plain_version(v));
    if let (Some(manifest), Some(to), Ok(before)) = (&manifest, to, shown(root, PLUGIN_KEY)) {
        let from = before.as_deref().or(plugin).unwrap_or(to);
        if before.as_deref() != Some(to) && compare(from, to) != Ordering::Greater {
            match breaking::plugin_entries(manifest, from, to) {
                Ok(entries) => pending.moves.push(Move {
                    key: PLUGIN_KEY,
                    before,
                    to: to.to_string(),
                    entries,
                }),
                Err(err) => pending.error = Some(err),
            }
        }
    }
    pending
}

/// Records `version` as shown, for a caller that printed the notes of that
/// release itself (ADR 0037): `playbook update`. Best effort.
pub fn record(root: &Path, version: &str) {
    let _ = crate::state::set(root, BINARY_KEY, version);
}

impl Pending {
    /// Moves each key in one transaction, if no other process moved it
    /// first, and returns the entries of the keys this process moved.
    ///
    /// A caller that already printed the playbook block, such as the install
    /// script or `playbook update`, sets `breaking::SHOWN_ENV` to the version
    /// that block started from. Only the playbook entries at or below it are
    /// returned then, since the caller showed the rest. An empty or unset
    /// value means nothing was shown.
    pub fn commit(self, root: &Path) -> Notices {
        let since = std::env::var(breaking::SHOWN_ENV).ok();
        self.commit_shown(root, since.as_deref())
    }

    fn commit_shown(self, root: &Path, shown_since: Option<&str>) -> Notices {
        // The same rule `playbook breaking --since` reads its version with.
        let shown_since = shown_since
            .map(|v| v.trim_start_matches('v'))
            .filter(|v| breaking::is_version(v));
        let mut notices = Notices {
            entries: Vec::new(),
            error: self.error,
        };
        if self.moves.is_empty() {
            return notices;
        }
        let moved = crate::state::transaction(root, |tx| {
            let mut moved = Vec::new();
            for m in &self.moves {
                if tx.get(m.key)?.filter(|v| !v.is_empty()) == m.before {
                    tx.set(m.key, &m.to)?;
                    moved.push(m.key);
                }
            }
            Ok(moved)
        })
        .unwrap_or_default();
        for m in self.moves {
            if !moved.contains(&m.key) {
                continue;
            }
            let shown = |e: &Entry| {
                m.key == BINARY_KEY
                    && shown_since.is_some_and(|v| compare(&e.version, v) == Ordering::Greater)
            };
            notices
                .entries
                .extend(m.entries.into_iter().filter(|e| !shown(e)));
        }
        notices
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;
    use std::path::PathBuf;

    const RUNNING: &str = env!("CARGO_PKG_VERSION");

    fn home(tag: &str) -> (PathBuf, PathBuf) {
        let home = scratch_dir(tag);
        let root = crate::common::paths::playbook_root_from(&home);
        fs::create_dir_all(&root).unwrap();
        (home.join(".claude"), root)
    }

    fn binary(root: &Path, claude: &Path, recorded: Option<&str>, older: bool) -> Vec<Entry> {
        prepare(root, claude, recorded, None, older)
            .commit(root)
            .entries
    }

    #[test]
    fn a_fresh_install_shows_nothing_and_records_this_binary() {
        // Arrange
        let (claude, root) = home("notices-fresh");

        // Act
        let first = binary(&root, &claude, None, false);

        // Assert
        assert!(first.is_empty());
        assert_eq!(shown(&root, BINARY_KEY), Ok(Some(RUNNING.into())));
    }

    #[test]
    fn the_recorded_version_or_an_older_binary_sets_the_start_once() {
        // Arrange
        let (claude, root) = home("notices-start");
        let (claude_old, root_old) = home("notices-old");

        // Act
        let upgraded = binary(&root, &claude, Some("0.25.0"), false);
        let again = binary(&root, &claude, Some("0.25.0"), false);
        let old = binary(&root_old, &claude_old, None, true);

        // Assert
        assert_eq!(upgraded, breaking::entries_between("0.25.0", RUNNING));
        assert!(!upgraded.is_empty());
        assert!(again.is_empty());
        assert_eq!(old, breaking::entries_between("0.0.0", RUNNING));
        assert_eq!(shown(&root, BINARY_KEY), Ok(Some(RUNNING.into())));
    }

    #[test]
    fn the_shown_version_wins_and_a_downgrade_shows_and_writes_nothing() {
        // Arrange
        let (claude, root) = home("notices-order");
        crate::state::set(&root, BINARY_KEY, "999.0.0").unwrap();

        // Act
        let entries = binary(&root, &claude, Some("0.0.1"), true);

        // Assert
        assert!(entries.is_empty());
        assert_eq!(shown(&root, BINARY_KEY), Ok(Some("999.0.0".into())));
    }

    #[test]
    fn a_caller_that_showed_the_block_leaves_only_the_entries_at_or_below_its_start() {
        // Arrange: each case moves both keys in its own home. The playbook
        // block spans 0.20.0 and 0.30.0; the plugin entry is above both.
        let entry = |version: &str, component| Entry {
            version: version.into(),
            component,
            summary: format!("change in {version}"),
            action: "run something".into(),
            migration: None,
        };
        let older = entry("0.20.0", breaking::Component::Playbook);
        let newer = entry("0.30.0", breaking::Component::Playbook);
        let plugin = entry("0.40.0", breaking::Component::Plugin);
        let commit_with = |tag: &str, shown_since: Option<&str>| {
            let (_, root) = home(tag);
            let pending = Pending {
                moves: vec![
                    Move {
                        key: BINARY_KEY,
                        before: None,
                        to: RUNNING.into(),
                        entries: vec![older.clone(), newer.clone()],
                    },
                    Move {
                        key: PLUGIN_KEY,
                        before: None,
                        to: "0.40.0".into(),
                        entries: vec![plugin.clone()],
                    },
                ],
                error: None,
            };
            let notices = pending.commit_shown(&root, shown_since).entries;
            (notices, shown(&root, BINARY_KEY))
        };

        // Act
        let shown_from_zero = commit_with("notices-shown-zero", Some("0.0.0"));
        let shown_from_older = commit_with("notices-shown-older", Some("0.20.0"));
        let with_a_v = commit_with("notices-shown-v", Some("v0.20.0"));
        let nothing_shown = commit_with("notices-shown-empty", Some(""));
        let not_a_version = commit_with("notices-shown-bad", Some("1"));

        // Assert: the caller's start keeps only the playbook entries at or
        // below it, and the plugin entry always shows.
        assert_eq!(shown_from_zero.0, std::slice::from_ref(&plugin));
        assert_eq!(shown_from_older.0, [older.clone(), plugin.clone()]);
        assert_eq!(with_a_v.0, shown_from_older.0);
        let everything = [older, newer, plugin];
        assert_eq!(nothing_shown.0, everything);
        assert_eq!(not_a_version.0, everything);
        for (_, recorded) in [
            shown_from_zero,
            shown_from_older,
            with_a_v,
            nothing_shown,
            not_a_version,
        ] {
            assert_eq!(recorded, Ok(Some(RUNNING.into())));
        }
    }

    #[test]
    fn only_the_process_that_moves_the_key_shows_the_block() {
        // Arrange: two steps read the same start before either commits.
        let (claude, root) = home("notices-race");
        let first = prepare(&root, &claude, Some("0.25.0"), None, false);
        let second = prepare(&root, &claude, Some("0.25.0"), None, false);

        // Act
        let shown_first = first.commit(&root);
        let shown_second = second.commit(&root);

        // Assert
        assert!(!shown_first.entries.is_empty());
        assert!(shown_second.entries.is_empty());
    }

    #[test]
    fn plugin_entries_come_from_the_manifest_above_the_shown_version() {
        // Arrange
        let (claude, root) = home("notices-plugin");
        let dir = claude.join("plugins/cache/pragmatic-engineer/playbook/0.40.0");
        fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        fs::create_dir_all(dir.join("skills")).unwrap();
        let mut manifest = serde_json::json!({
            "name": "playbook",
            "version": "0.40.0",
            "metadata": {"breakingChanges": [
                {"version": "0.35.0", "component": "plugin", "summary": "old", "action": "x"},
                {"version": "0.40.0", "component": "plugin", "summary": "new", "action": "y"},
                {"version": "0.40.0", "component": "future", "summary": "skip", "action": "z"}
            ]}
        });
        let write = |m: &serde_json::Value| {
            fs::write(dir.join(".claude-plugin/plugin.json"), m.to_string()).unwrap();
        };
        write(&manifest);
        let plugin =
            |recorded: Option<&str>| prepare(&root, &claude, None, recorded, false).commit(&root);

        // Act
        let first_sight = plugin(None);
        crate::state::delete(&root, PLUGIN_KEY).unwrap();
        let upgraded = plugin(Some("0.36.0"));
        let again = plugin(Some("0.36.0"));
        manifest["version"] = "0.41.0".into();
        manifest["metadata"]["breakingChanges"][1]["summary"] = serde_json::Value::Null;
        write(&manifest);
        let unreadable = plugin(None);

        // Assert
        assert!(first_sight.entries.is_empty());
        let summaries: Vec<&str> = upgraded
            .entries
            .iter()
            .map(|e| e.summary.as_str())
            .collect();
        assert_eq!(summaries, ["new"]);
        assert!(again.entries.is_empty());
        assert!(
            unreadable.render().contains("run playbook update"),
            "{unreadable:?}"
        );
        assert_eq!(shown(&root, PLUGIN_KEY), Ok(Some("0.40.0".into())));
    }
}
