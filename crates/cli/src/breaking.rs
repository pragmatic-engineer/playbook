// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Breaking change notices (ADR 0035). `breaking-changes.json` at the
//! repository root lists every release that needs a step from the user or
//! changes behavior. The binary embeds it, so every channel that runs the
//! new binary can print the entries between two versions.

use crate::update::resolve::compare;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::sync::OnceLock;

/// The embedded record, as committed at build time.
const EMBEDDED: &str = include_str!("../../../breaking-changes.json");

/// What an entry applies to. Each has its own version line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
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

/// The fields an entry may have.
const FIELDS: [&str; 5] = ["version", "component", "summary", "action", "migration"];

/// One breaking change.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Entry {
    /// The release that brings the change, as MAJOR.MINOR.PATCH.
    pub version: String,
    pub component: Component,
    /// One plain line that says what changed.
    pub summary: String,
    /// The command or step the user runs, or `AUTO_ACTION`.
    pub action: String,
    /// The id of the migration that handles it, when one does.
    pub migration: Option<String>,
}

impl Entry {
    /// True when the user has to run something, so an automatic update
    /// should stop and tell them.
    pub fn needs_user(&self) -> bool {
        self.action != AUTO_ACTION
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    entries: Vec<serde_json::Value>,
}

/// Parses and checks a record strictly: an unknown field or a bad entry is
/// an error. Used for the embedded file and the release guards.
pub fn parse(text: &str) -> Result<Vec<Entry>, String> {
    let file: File =
        serde_json::from_str(text).map_err(|e| format!("breaking-changes.json: {e}"))?;
    file.entries
        .into_iter()
        .enumerate()
        .map(|(i, value)| {
            let at = format!("breaking-changes.json entry {}", i + 1);
            if let Some(key) = value
                .as_object()
                .and_then(|o| o.keys().find(|k| !FIELDS.contains(&k.as_str())))
            {
                return Err(format!("{at}: unknown field '{key}'"));
            }
            let entry: Entry = serde_json::from_value(value).map_err(|e| format!("{at}: {e}"))?;
            validate(&entry).map_err(|e| format!("{at}: {e}"))?;
            Ok(entry)
        })
        .collect()
}

/// Reads entries the way an older binary must read a newer plugin's list:
/// unknown fields are ignored, and an entry that does not parse or check,
/// such as one with a component this binary does not know, is skipped.
pub fn lenient(values: &[serde_json::Value]) -> Vec<Entry> {
    values
        .iter()
        .filter_map(|v| serde_json::from_value::<Entry>(v.clone()).ok())
        .filter(|e| validate(e).is_ok())
        .collect()
}

/// Checks one entry. The version is MAJOR.MINOR.PATCH. The summary, action
/// and migration id are each one non empty line with no character that a
/// shell, a Ruby string or markdown would read as code. An action of
/// `AUTO_ACTION` names its migration, and a near miss of it is refused.
pub fn validate(e: &Entry) -> Result<(), String> {
    if !is_release_version(&e.version) {
        return Err(format!("version '{}' is not MAJOR.MINOR.PATCH", e.version));
    }
    let mut lines = vec![
        ("summary", e.summary.as_str()),
        ("action", e.action.as_str()),
    ];
    if let Some(id) = &e.migration {
        lines.push(("migration", id.as_str()));
    }
    for (field, value) in lines {
        if value.trim().is_empty() {
            return Err(format!("{field} must be one non empty line"));
        }
        if value.contains("#{")
            || value
                .chars()
                .any(|c| c.is_control() || matches!(c, '\\' | '"' | '`' | '$' | '<' | '>'))
        {
            return Err(format!(
                "{field} must be one line without control characters, \\, \", `, $, <, > or #{{"
            ));
        }
    }
    let loose = e.action.trim().trim_end_matches('.');
    if loose.eq_ignore_ascii_case(AUTO_ACTION) {
        if e.action != AUTO_ACTION {
            return Err(format!("action must be exactly '{AUTO_ACTION}'"));
        }
        if e.migration.is_none() {
            return Err(format!("action '{AUTO_ACTION}' needs a migration id"));
        }
    }
    Ok(())
}

/// MAJOR.MINOR.PATCH with no leading zeros and no pre-release, the only
/// shape a released version has.
fn is_release_version(v: &str) -> bool {
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

/// True for a version `select` accepts: MAJOR.MINOR.PATCH with an optional
/// leading `v` and an optional pre-release.
pub fn is_version(v: &str) -> bool {
    crate::plugin_root::is_plain_version(v.trim_start_matches('v'))
}

/// The entries as a JSON list, the shape `lenient` reads back. `playbook
/// breaking --since <version> --json` prints it, and an older binary's
/// `playbook update` reads it, so the argv and this shape stay stable.
pub fn to_json(entries: &[Entry]) -> String {
    serde_json::to_string(entries).unwrap_or_else(|_| "[]".into())
}

/// The entries of `component` in `entries` with `from < version <= to`,
/// oldest first. Nothing when `to` is not above `from`, or when either is
/// not a version (an optional leading `v` and a pre-release are fine), so
/// an empty or corrupt record never shows the whole history.
pub fn select(entries: &[Entry], component: Component, from: &str, to: &str) -> Vec<Entry> {
    let (from, to) = (from.trim_start_matches('v'), to.trim_start_matches('v'));
    if !is_version(from) || !is_version(to) || compare(to, from) != Ordering::Greater {
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

/// What `playbook breaking` prints: the playbook entries since `since` up to
/// this binary, or every entry, as text or as JSON.
pub fn command(since: Option<&str>, json: bool) -> Result<String, String> {
    let current = env!("CARGO_PKG_VERSION");
    let (entries, text) = match since {
        Some(v) if !is_version(v) => {
            return Err(format!("'{v}' is not a version such as 0.25.0"));
        }
        Some(v) => {
            let e = entries_between(v, current);
            let t = render(&e);
            (e, t)
        }
        None => (all().to_vec(), render_all(all())),
    };
    Ok(if json {
        format!("{}\n", to_json(&entries))
    } else {
        text
    })
}

/// The playbook entries an upgrade from `from` to `to` crosses, from the
/// embedded record.
pub fn entries_between(from: &str, to: &str) -> Vec<Entry> {
    select(all(), Component::Playbook, from, to)
}

/// The plugin entries an upgrade from plugin `from` to `to` crosses, read
/// from a plugin manifest's `metadata.breakingChanges` (ADR 0035 decision
/// 4.6). The list is read with `lenient`, since a newer plugin can carry
/// fields or components this binary does not know. A manifest without the
/// key has no entries. A plugin entry this binary cannot read is an error,
/// so the caller can say to update the binary instead of showing nothing.
pub fn plugin_entries(
    manifest: &serde_json::Value,
    from: &str,
    to: &str,
) -> Result<Vec<Entry>, String> {
    let values = manifest
        .pointer("/metadata/breakingChanges")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let entries = lenient(values);
    let plugin = |v: &serde_json::Value| v["component"].as_str() == Some(Component::Plugin.name());
    let raw = values.iter().filter(|v| plugin(v)).count();
    let read = entries
        .iter()
        .filter(|e| e.component == Component::Plugin)
        .count();
    if read != raw {
        return Err(
            "the plugin lists a breaking change this playbook version cannot read; run playbook update"
                .into(),
        );
    }
    Ok(select(&entries, Component::Plugin, from, to))
}

/// One short block for the user about an upgrade, or an empty string when
/// there is nothing.
pub fn render(entries: &[Entry]) -> String {
    render_titled("Breaking changes in this upgrade:", entries)
}

/// The same block for a full listing, which belongs to no upgrade.
pub fn render_all(entries: &[Entry]) -> String {
    render_titled("Breaking changes this binary knows about:", entries)
}

fn render_titled(title: &str, entries: &[Entry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = format!("{title}\n");
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

    fn record(
        version: &str,
        component: &str,
        summary: &str,
        action: &str,
        migration: &str,
    ) -> String {
        format!(
            r#"{{"entries":[{{"version":"{version}","component":"{component}","summary":"{summary}","action":"{action}","migration":{migration}}}]}}"#
        )
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
        assert!(first.needs_user());
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
    fn select_is_empty_when_a_version_is_missing_or_corrupt() {
        let entries = vec![entry("0.26.0", Component::Playbook)];
        for from in ["", "latest", "0.25", "0.25.0.1"] {
            assert!(
                select(&entries, Component::Playbook, from, "0.27.0").is_empty(),
                "{from:?}"
            );
        }
        assert!(select(&entries, Component::Playbook, "0.25.0", "").is_empty());
    }

    #[test]
    fn select_accepts_a_leading_v() {
        let entries = vec![entry("0.26.0", Component::Playbook)];
        assert_eq!(
            select(&entries, Component::Playbook, "v0.25.0", "v0.26.0").len(),
            1
        );
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
    fn parse_rejects_a_bad_version_and_an_unknown_component_or_field() {
        let err = |text: &str| parse(text).unwrap_err();
        assert!(err(&record("0.26", "playbook", "s", "x", "null")).contains("MAJOR.MINOR.PATCH"));
        assert!(parse(&record("0.26.01", "playbook", "s", "x", "null")).is_err());
        assert!(parse(&record("0.26.0", "code", "s", "x", "null")).is_err());
        assert!(err(r#"{"entries":[{"version":"0.26.0","component":"plugin","summary":"s","action":"x","migration":null,"extra":1}]}"#).contains("unknown field 'extra'"));
        assert!(parse(r#"{"entries":[],"extra":1}"#).is_err());
        assert_eq!(
            parse(&record("0.26.0", "telemetry", "s", "x", "null")).unwrap()[0].component,
            Component::Telemetry
        );
    }

    #[test]
    fn parse_checks_the_summary_the_action_and_the_migration_id() {
        let err = |text: String| parse(&text).unwrap_err();
        assert!(err(record("0.26.0", "playbook", " ", "x", "null")).contains("summary"));
        assert!(err(record("0.26.0", "playbook", "a\\nb", "x", "null")).contains("summary"));
        assert!(err(record("0.26.0", "playbook", "s", "", "null")).contains("action"));
        assert!(err(record("0.26.0", "playbook", "s", "a\\nb", "null")).contains("action"));
        assert!(err(record("0.26.0", "playbook", "s", "x", "\"\"")).contains("migration"));
        assert!(err(record("0.26.0", "playbook", "s", "x", "\"a\\nb\"")).contains("migration"));
    }

    #[test]
    fn parse_refuses_text_that_a_shell_ruby_or_markdown_would_run() {
        for bad in [
            "run $(id)",
            "#{system}",
            "a `b`",
            "x \\\" y",
            "<b>",
            "a\\\\b",
        ] {
            let text = record("0.26.0", "playbook", "s", bad, "null");
            assert!(parse(&text).unwrap_err().contains("action"), "{bad}");
        }
        assert!(parse(&record(
            "0.26.0",
            "playbook",
            "it's fine, really",
            "brew upgrade playbook",
            "null"
        ))
        .is_ok());
    }

    #[test]
    fn the_automatic_action_needs_a_migration_and_an_exact_spelling() {
        let auto =
            |action: &str, migration: &str| record("0.27.0", "playbook", "s", action, migration);
        assert!(parse(&auto(AUTO_ACTION, "null"))
            .unwrap_err()
            .contains("needs a migration"));
        assert!(parse(&auto(
            "None, migrated automatically.",
            "\"0001-memory-store-move\""
        ))
        .unwrap_err()
        .contains("exactly"));
        assert!(parse(&auto(AUTO_ACTION, "\"0001-memory-store-move\"")).is_ok());
    }

    #[test]
    fn to_json_keeps_the_shape_older_binaries_read() {
        // Older binaries parse this exact shape in `playbook update`.
        assert_eq!(
            to_json(&[entry("0.26.0", Component::Playbook)]),
            r#"[{"version":"0.26.0","component":"playbook","summary":"change in 0.26.0","action":"playbook init","migration":null}]"#
        );
    }

    #[test]
    fn command_lists_since_a_version_or_everything_and_refuses_a_bad_version() {
        assert_eq!(command(Some("0.26.0"), true).unwrap(), "[]\n");
        assert!(command(Some("0.25.0"), false)
            .unwrap()
            .starts_with("Breaking changes in this upgrade:\n"));
        assert!(command(None, false)
            .unwrap()
            .starts_with("Breaking changes this binary knows about:\n"));
        assert!(command(Some("latest"), false)
            .unwrap_err()
            .contains("not a version"));
    }

    #[test]
    fn to_json_round_trips_through_lenient() {
        let mut migrated = entry("0.27.0", Component::Plugin);
        migrated.action = AUTO_ACTION.into();
        migrated.migration = Some("0001-memory-store-move".into());
        let entries = vec![entry("0.26.0", Component::Playbook), migrated];

        let values: Vec<serde_json::Value> = serde_json::from_str(&to_json(&entries)).unwrap();

        assert_eq!(lenient(&values), entries);
        assert_eq!(to_json(&[]), "[]");
    }

    #[test]
    fn lenient_ignores_unknown_fields_and_skips_entries_it_cannot_read() {
        // Arrange
        let values: Vec<serde_json::Value> = serde_json::from_str(
            r#"[
              {"version":"0.32.0","component":"plugin","summary":"s","action":"x","migration":null,"since":"later"},
              {"version":"0.33.0","component":"future","summary":"s","action":"x","migration":null},
              {"version":"bad","component":"plugin","summary":"s","action":"x","migration":null}
            ]"#,
        )
        .unwrap();

        // Act
        let got = lenient(&values);

        // Assert
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].version, "0.32.0");
    }

    #[test]
    fn plugin_entries_reads_the_manifest_mirror_between_two_plugin_versions() {
        // Arrange
        let manifest: serde_json::Value = serde_json::from_str(
            r#"{"name":"playbook","metadata":{"breakingChanges":[
              {"version":"0.31.0","component":"plugin","summary":"old","action":"none needed","migration":null},
              {"version":"0.32.0","component":"plugin","summary":"new","action":"run /playbook:doctor","migration":null,"extra":1},
              {"version":"0.32.0","component":"playbook","summary":"binary","action":"playbook init","migration":null},
              {"version":"0.32.0","component":"desktop","summary":"later","action":"none needed","migration":null}
            ]}}"#,
        )
        .unwrap();

        // Act
        let got = plugin_entries(&manifest, "0.31.0", "0.32.0").unwrap();

        // Assert
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].summary, "new");
        assert_eq!(got[0].component, Component::Plugin);
    }

    #[test]
    fn plugin_entries_is_empty_for_a_manifest_without_the_mirror() {
        // Arrange
        let bare = serde_json::json!({"name": "playbook"});
        let wrong_type = serde_json::json!({"metadata": {"breakingChanges": "none"}});

        // Act
        let from_bare = plugin_entries(&bare, "0.1.0", "9.0.0").unwrap();
        let from_wrong_type = plugin_entries(&wrong_type, "0.1.0", "9.0.0").unwrap();

        // Assert
        assert!(from_bare.is_empty());
        assert!(from_wrong_type.is_empty());
    }

    #[test]
    fn plugin_entries_is_an_error_when_a_plugin_entry_cannot_be_read() {
        // Arrange
        let manifest = serde_json::json!({"metadata": {"breakingChanges": [
            {"version": "0.32.0", "component": "plugin", "summary": "new", "action": "none needed", "migration": null},
            {"version": "0.33", "component": "plugin", "summary": "odd", "action": "none needed", "migration": null}
        ]}});

        // Act
        let got = plugin_entries(&manifest, "0.31.0", "0.32.0");

        // Assert
        assert!(got.unwrap_err().contains("run playbook update"));
    }
}
