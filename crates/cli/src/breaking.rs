// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Breaking change notices (ADR 0035). `breaking-changes.json` at the
//! repository root lists every release that needs a step from the user or
//! changes behavior. The binary embeds it, so every channel that runs the
//! new binary can print the entries between two versions.

use crate::update::resolve::compare;
use serde::Deserialize;
use std::cmp::Ordering;
use std::sync::OnceLock;

/// The embedded record, as committed at build time.
const EMBEDDED: &str = include_str!("../../../breaking-changes.json");

/// What an entry applies to. Each has its own version line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Component {
    Playbook,
    Plugin,
    Telemetry,
}

impl Component {
    pub fn name(self) -> &'static str {
        match self {
            Component::Playbook => "playbook",
            Component::Plugin => "plugin",
            Component::Telemetry => "telemetry",
        }
    }
}

/// The action of a change that a migration handles with no step from the user.
pub const AUTO_ACTION: &str = "none, migrated automatically";

/// One breaking change.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The release that brings the change, as MAJOR.MINOR.PATCH.
    pub version: String,
    pub component: Component,
    /// One plain line that says what changed.
    pub summary: String,
    /// The command or step the user runs, or "none, migrated automatically".
    pub action: String,
    /// The id of the migration that handles it, when one does.
    pub migration: Option<String>,
}

impl Entry {
    /// True when the user has to run something, so an automatic update
    /// should stop and tell them.
    pub fn needs_user(&self) -> bool {
        self.action.trim() != AUTO_ACTION
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    entries: Vec<Entry>,
}

/// Parses and checks a record. Every version is MAJOR.MINOR.PATCH, and the
/// summary, action and migration id are each one non empty line.
pub fn parse(text: &str) -> Result<Vec<Entry>, String> {
    let file: File =
        serde_json::from_str(text).map_err(|e| format!("breaking-changes.json: {e}"))?;
    for (i, e) in file.entries.iter().enumerate() {
        let at = format!("breaking-changes.json entry {}", i + 1);
        if !is_plain_version(&e.version) {
            return Err(format!(
                "{at}: version '{}' is not MAJOR.MINOR.PATCH",
                e.version
            ));
        }
        let mut lines = vec![
            ("summary", e.summary.as_str()),
            ("action", e.action.as_str()),
        ];
        if let Some(id) = &e.migration {
            lines.push(("migration", id.as_str()));
        }
        for (field, value) in lines {
            if value.trim().is_empty() || value.contains('\n') {
                return Err(format!("{at}: {field} must be one non empty line"));
            }
        }
    }
    Ok(file.entries)
}

fn is_plain_version(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        })
}

/// Every entry in the embedded record. A record that does not parse is a
/// build defect the unit tests catch, so at run time it reads as empty.
pub fn all() -> &'static [Entry] {
    static ALL: OnceLock<Vec<Entry>> = OnceLock::new();
    ALL.get_or_init(|| parse(EMBEDDED).unwrap_or_default())
}

/// The entries of `component` in `entries` with `from < version <= to`,
/// oldest first. Nothing when `to` is not above `from`.
pub fn select(entries: &[Entry], component: Component, from: &str, to: &str) -> Vec<Entry> {
    if compare(to, from) != Ordering::Greater {
        return Vec::new();
    }
    let mut found: Vec<Entry> = entries
        .iter()
        .filter(|e| {
            e.component == component
                && compare(&e.version, from) == Ordering::Greater
                && compare(&e.version, to) != Ordering::Greater
        })
        .cloned()
        .collect();
    found.sort_by(|a, b| compare(&a.version, &b.version));
    found
}

/// The playbook entries an upgrade from `from` to `to` crosses, from the
/// embedded record.
pub fn entries_between(from: &str, to: &str) -> Vec<Entry> {
    select(all(), Component::Playbook, from, to)
}

/// One short block for stderr, or an empty string when there is nothing.
pub fn render(entries: &[Entry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = String::from("Breaking changes in this upgrade:\n");
    for e in entries {
        out.push_str(&format!(
            "  {} {}: {}\n    Action: {}\n",
            e.component.name(),
            e.version,
            e.summary,
            e.action
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(version: &str, component: Component) -> Entry {
        Entry {
            version: version.into(),
            component,
            summary: format!("change in {version}"),
            action: "playbook init".into(),
            migration: None,
        }
    }

    #[test]
    fn the_embedded_record_parses_and_names_only_registered_migrations() {
        // Arrange
        let ids: Vec<&str> = crate::init::migrate::registry()
            .iter()
            .map(|m| m.id)
            .collect();

        // Act
        let entries = parse(EMBEDDED).expect("breaking-changes.json must parse");

        // Assert
        assert!(!entries.is_empty());
        assert_eq!(all(), entries.as_slice());
        for e in &entries {
            if let Some(id) = &e.migration {
                assert!(ids.contains(&id.as_str()), "unknown migration id {id}");
            }
        }
    }

    #[test]
    fn the_first_entry_is_the_plugin_path_change_in_0_26_0() {
        let first = &all()[0];
        assert_eq!(
            (first.version.as_str(), first.component),
            ("0.26.0", Component::Playbook)
        );
        assert_eq!(first.action, "brew upgrade playbook or playbook update");
    }

    #[test]
    fn select_keeps_entries_above_from_up_to_and_including_to() {
        // Arrange
        let entries = vec![
            entry("0.27.0", Component::Playbook),
            entry("0.25.0", Component::Playbook),
            entry("0.26.0", Component::Playbook),
            entry("0.26.0", Component::Plugin),
            entry("0.28.0", Component::Playbook),
        ];

        // Act
        let got = select(&entries, Component::Playbook, "0.25.0", "0.27.0");

        // Assert
        let versions: Vec<&str> = got.iter().map(|e| e.version.as_str()).collect();
        assert_eq!(versions, vec!["0.26.0", "0.27.0"]);
    }

    #[test]
    fn select_is_empty_for_the_same_version_and_for_a_downgrade() {
        let entries = vec![entry("0.26.0", Component::Playbook)];
        assert!(select(&entries, Component::Playbook, "0.26.0", "0.26.0").is_empty());
        assert!(select(&entries, Component::Playbook, "0.27.0", "0.25.0").is_empty());
    }

    #[test]
    fn select_counts_a_pre_release_of_the_entry_version_as_below_it() {
        let entries = vec![entry("0.27.0", Component::Playbook)];
        let got = select(&entries, Component::Playbook, "0.27.0-rc.1", "0.27.0");
        assert_eq!(got.len(), 1);
        assert!(select(&entries, Component::Playbook, "0.26.0", "0.27.0-rc.1").is_empty());
    }

    #[test]
    fn entries_between_reads_the_embedded_playbook_entries() {
        let got = entries_between("0.25.0", "0.26.0");
        assert!(got.iter().any(|e| e.version == "0.26.0"));
        assert!(got.iter().all(|e| e.component == Component::Playbook));
        assert!(entries_between("0.26.0", "0.26.0").is_empty());
    }

    #[test]
    fn render_prints_one_block_with_each_summary_and_action() {
        // Arrange
        let mut migrated = entry("0.27.0", Component::Plugin);
        migrated.action = AUTO_ACTION.into();

        // Act
        let text = render(&[entry("0.26.0", Component::Playbook), migrated]);

        // Assert
        assert_eq!(
            text,
            "Breaking changes in this upgrade:\n  playbook 0.26.0: change in 0.26.0\n    Action: playbook init\n  plugin 0.27.0: change in 0.27.0\n    Action: none, migrated automatically\n"
        );
        assert_eq!(render(&[]), "");
    }

    #[test]
    fn needs_user_is_false_only_for_the_automatic_action() {
        let mut e = entry("0.27.0", Component::Playbook);
        assert!(e.needs_user());
        e.action = AUTO_ACTION.into();
        assert!(!e.needs_user());
    }

    #[test]
    fn parse_rejects_a_bad_version_an_empty_line_and_an_unknown_component() {
        let base = |version: &str, summary: &str, component: &str| {
            format!(
                r#"{{"entries":[{{"version":"{version}","component":"{component}","summary":"{summary}","action":"x","migration":null}}]}}"#
            )
        };
        assert!(parse(&base("0.26", "s", "playbook"))
            .unwrap_err()
            .contains("MAJOR.MINOR.PATCH"));
        assert!(parse(&base("0.26.01", "s", "playbook")).is_err());
        assert!(parse(&base("0.26.0", " ", "playbook"))
            .unwrap_err()
            .contains("summary"));
        assert!(parse(&base("0.26.0", "a\\nb", "playbook"))
            .unwrap_err()
            .contains("summary"));
        assert!(parse(&base("0.26.0", "s", "code")).is_err());
        assert!(parse(r#"{"entries":[],"extra":1}"#).is_err());
        assert_eq!(
            parse(&base("0.26.0", "s", "telemetry")).unwrap()[0].component,
            Component::Telemetry
        );
    }
}
