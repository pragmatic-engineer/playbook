// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The list of what still needs the user after an upgrade, and the one
//! renderer the session hook, `playbook migrate`, `migrate --status`,
//! `doctor` and `doctor --fix` share (ADR 0034, 2026-10-19 amendment).
//!
//! Each item is one bullet: what it is, why it matters, and the exact
//! command that fixes it.

use crate::init::migrate::{self, OpenItem};
use crate::update::resolve::compare;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// The slug of the item for a plugin that needs a newer binary.
pub const PLUGIN_NEEDS_BINARY: &str = "plugin-needs-newer-binary";
/// The slug of the item for a session that loaded an old plugin folder.
pub const PLUGIN_STALE_SESSION: &str = "plugin-stale-session";
/// The slug the plugin range check shares with its migration.
pub const PLUGIN_RANGE: &str = "plugin-version-range";

/// What kind of item it is, which says when it stops showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A change to a file the user owns. It closes when setup applies it or
    /// the user says no.
    Consent,
    /// A finding playbook cannot fix by itself.
    Report,
}

/// One thing that needs the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub slug: String,
    pub kind: Kind,
    /// What it is, in plain words.
    pub what: String,
    /// Why it matters.
    pub why: &'static str,
    /// The exact command, or the plain action, that fixes it.
    pub fix: String,
    /// Whether `playbook migrate` fixes it on its own.
    pub by_migrate: bool,
}

/// Why an item matters, by slug.
fn why(slug: &str) -> &'static str {
    match slug {
        migrate::STATUSLINE_MIGRATION => {
            "the status line still runs a script that newer playbook no longer ships"
        }
        "shell-init-rc-line" | migrate::RC_LINE_MIGRATION => {
            "every new shell loads a playbook launcher that is retired"
        }
        migrate::DEAD_HOOKS_MIGRATION => {
            "Claude Code runs these hooks on every event and each one fails"
        }
        migrate::COLLECTOR_SERVICE_MIGRATION => {
            "the usage collector runs only while a session keeps it alive, so usage data has gaps"
        }
        migrate::PROMPT_DEFAULT_MIGRATION => {
            "pb cc sessions start without the playbook system prompt until it is installed or turned off"
        }
        "other-playbook-plugins" => "hooks and skills run twice in every session",
        PLUGIN_RANGE => "skills and commands may call binary features that work differently in this version",
        PLUGIN_NEEDS_BINARY => "skills from this plugin call binary features this version does not have",
        PLUGIN_STALE_SESSION => "this session runs the old skills and commands until Claude Code starts again",
        _ => "playbook found it while checking this machine after the upgrade",
    }
}

/// `message` split at its last `Run: ` into the text and the command.
fn split_command(message: &str) -> (String, Option<String>) {
    match message.rsplit_once("Run: ") {
        Some((text, command)) if !command.trim().is_empty() => (
            text.trim_end().trim_end_matches('.').to_string(),
            Some(command.trim().to_string()),
        ),
        _ => (message.trim_end_matches('.').to_string(), None),
    }
}

/// `what` without a trailing `, so <reason>` clause, since the bullet gives
/// the reason on its own line. A list after a colon in the clause stays.
fn without_reason(what: String) -> String {
    match what.rsplit_once(", so ") {
        Some((text, reason)) => match reason.split_once(": ") {
            Some((_, list)) => format!("{text}: {list}"),
            None => text.to_string(),
        },
        None => what,
    }
}

/// Items for the open `Consent` and `Report` migrations.
pub fn from_open(open: &[OpenItem]) -> Vec<Item> {
    open.iter()
        .map(|o| {
            let (what, run) = split_command(&o.message);
            let (kind, fix) = match o.command {
                Some(command) => (Kind::Consent, command.to_string()),
                None => (
                    Kind::Report,
                    run.unwrap_or_else(|| "playbook migrate --status".to_string()),
                ),
            };
            Item {
                slug: o.id.to_string(),
                kind,
                what: without_reason(what),
                why: why(o.id),
                fix,
                by_migrate: false,
            }
        })
        .collect()
}

/// What the plugin checks read, passed in so tests never touch the real home.
#[derive(Debug, Clone)]
pub struct PluginProbe {
    pub claude_home: PathBuf,
    /// The plugin folder this session runs from (`CLAUDE_PLUGIN_ROOT`).
    pub session_root: Option<PathBuf>,
    pub binary: String,
    pub homebrew: bool,
}

impl PluginProbe {
    /// The probe for this process and `home`.
    pub fn for_home(home: &Path) -> PluginProbe {
        let exe = std::env::current_exe().unwrap_or_default();
        // A Homebrew binary is often reached through a symlink in /usr/local/bin.
        let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
        PluginProbe {
            claude_home: home.join(".claude"),
            session_root: std::env::var_os("CLAUDE_PLUGIN_ROOT")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            binary: env!("CARGO_PKG_VERSION").to_string(),
            homebrew: crate::update::is_homebrew(&exe),
        }
    }
}

/// The plugin items: a plugin that needs a newer binary, a plugin outside
/// the minor line of the binary, and a session that loaded an older plugin
/// folder than the one Claude Code installed. A version problem counts only
/// when both the installed plugin and the folder this session runs from have
/// it, so it goes away once either one is current.
pub fn plugin_items(probe: &PluginProbe) -> Vec<Item> {
    use crate::plugin_root::{
        binary_too_old, find, is_plugin_dir, plugin_out_of_range, plugin_version,
    };
    let Some(installed) = find(None, &probe.claude_home) else {
        return Vec::new();
    };
    let session = probe
        .session_root
        .clone()
        .filter(|r| is_plugin_dir(r) && *r != installed);
    let roots: Vec<&Path> = std::iter::once(installed.as_path())
        .chain(session.as_deref())
        .collect();
    let mut items = Vec::new();
    let too_old: Vec<Option<String>> = roots
        .iter()
        .map(|r| binary_too_old(r, &probe.binary, probe.homebrew))
        .collect();
    let range: Vec<Option<String>> = roots
        .iter()
        .map(|r| {
            plugin_version(r).and_then(|v| plugin_out_of_range(&v, &probe.binary, probe.homebrew))
        })
        .collect();
    if let Some(Some(first)) = too_old
        .first()
        .filter(|_| too_old.iter().all(Option::is_some))
    {
        items.push(report(PLUGIN_NEEDS_BINARY, first));
    } else if let Some(Some(first)) = range.first().filter(|_| range.iter().all(Option::is_some)) {
        items.push(report(PLUGIN_RANGE, first));
    }
    let versions = (
        plugin_version(&installed),
        session.as_deref().and_then(plugin_version),
    );
    if let (Some(new), Some(old)) = versions {
        if compare(&old, &new) == Ordering::Less {
            items.push(Item {
                slug: PLUGIN_STALE_SESSION.to_string(),
                kind: Kind::Report,
                what: format!(
                    "This session loaded the playbook plugin {old} from an old folder, and Claude Code has {new} installed"
                ),
                why: why(PLUGIN_STALE_SESSION),
                fix: "exit this session and start Claude Code again".to_string(),
                by_migrate: false,
            });
        }
    }
    items
}

fn report(slug: &'static str, message: &str) -> Item {
    let (what, run) = split_command(message);
    Item {
        slug: slug.to_string(),
        kind: Kind::Report,
        what,
        why: why(slug),
        fix: run.unwrap_or_else(|| "playbook doctor".to_string()),
        by_migrate: false,
    }
}

/// The migration items and the plugin items as one list, each slug once.
/// The plugin checks read the plugin folders now, so they replace the
/// migration's copy of the range check.
pub fn merge(open: &[OpenItem], plugin: Vec<Item>) -> Vec<Item> {
    let mut items: Vec<Item> = from_open(open)
        .into_iter()
        .filter(|i| i.slug != PLUGIN_RANGE)
        .collect();
    items.extend(plugin);
    items
}

/// Every open item for `home`, read without writing: the migration checks
/// and the plugin checks. `None` when the migration record cannot be read.
pub fn collect_read_only(home: &Path) -> Option<Vec<Item>> {
    let ctx = migrate::Ctx::for_home(home);
    let open: Vec<OpenItem> = migrate::plan(&ctx)?
        .into_iter()
        .filter(|p| p.scope != "owned")
        .filter_map(|p| match p.state {
            migrate::State::Open { message, command } => Some(OpenItem {
                id: p.id,
                message,
                command,
            }),
            _ => None,
        })
        .collect();
    Some(merge(&open, plugin_items(&PluginProbe::for_home(home))))
}

/// `what` with its first letter in upper case, unless the first word is a
/// key, a path or a name that keeps its case.
fn sentence(what: &str) -> String {
    let first = what.split(' ').next().unwrap_or_default();
    if first.is_empty() || !first.bytes().all(|b| b.is_ascii_lowercase()) {
        return what.to_string();
    }
    let mut chars = what.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// One bullet per item: what, why, and the fix.
pub fn bullets(items: &[Item]) -> String {
    let mut text = String::new();
    for item in items {
        text.push_str(&format!(
            "  - {}.\n    Why: {}.\n    Fix: {}\n",
            sentence(&item.what),
            item.why,
            item.fix
        ));
    }
    text
}

/// The last line: the one command that fixes what can be fixed without the
/// user, else the one that asks about every setup change, else the one that
/// shows the list again.
pub fn final_line(items: &[Item]) -> String {
    if items.iter().any(|i| i.by_migrate) {
        "To fix what playbook can fix on its own, run: playbook migrate".to_string()
    } else if items.iter().any(|i| i.kind == Kind::Consent) {
        "To go through every setup change at once, run: playbook setup".to_string()
    } else {
        "To see this list again, run: playbook migrate --status".to_string()
    }
}

/// How many items need the user, in words.
fn count(n: usize) -> String {
    if n == 1 {
        "1 item needs you".to_string()
    } else {
        format!("{n} items need you")
    }
}

/// The full list under a heading, or an empty string for no items.
pub fn render(items: &[Item]) -> String {
    if items.is_empty() {
        return String::new();
    }
    format!(
        "{}:\n{}{}\n",
        count(items.len()),
        bullets(items),
        final_line(items)
    )
}

/// The list as the session message shows it, or `None` for no items.
pub fn session_text(items: &[Item]) -> Option<String> {
    (!items.is_empty()).then(|| {
        format!(
            "playbook {}: {}",
            env!("CARGO_PKG_VERSION"),
            render(items).trim_end()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;

    fn item(slug: &str, kind: Kind, what: &str) -> Item {
        Item {
            slug: slug.into(),
            kind,
            what: what.into(),
            why: why(slug),
            fix: "playbook setup --only x".into(),
            by_migrate: false,
        }
    }

    fn plugin(home: &Path, version: &str, min: &str) -> PathBuf {
        let dir = home.join(format!(
            ".claude/plugins/cache/pragmatic-engineer/playbook/{version}"
        ));
        fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        fs::create_dir_all(dir.join("skills")).unwrap();
        fs::write(
            dir.join(".claude-plugin/plugin.json"),
            format!(r#"{{"name":"playbook","version":"{version}","metadata":{{"minBinaryVersion":"{min}"}}}}"#),
        )
        .unwrap();
        dir
    }

    fn install(home: &Path, dir: &Path, version: &str) {
        fs::write(
            home.join(".claude/plugins/installed_plugins.json"),
            format!(
                r#"{{"version":2,"plugins":{{"playbook@pragmatic-engineer":[{{"scope":"user","installPath":"{}","version":"{version}"}}]}}}}"#,
                dir.display()
            ),
        )
        .unwrap();
    }

    fn probe(home: &Path, session: Option<PathBuf>, binary: &str) -> PluginProbe {
        PluginProbe {
            claude_home: home.join(".claude"),
            session_root: session,
            binary: binary.into(),
            homebrew: false,
        }
    }

    #[test]
    fn each_item_is_one_bullet_with_what_why_and_the_fix() {
        // Arrange
        let open = [
            OpenItem {
                id: migrate::STATUSLINE_MIGRATION,
                message: "statusLine.command in settings.json still runs the old script, so the status line breaks".into(),
                command: Some("playbook setup --statusline"),
            },
            OpenItem {
                id: "other-playbook-plugins",
                message: "installed next to playbook@pragmatic-engineer, so hooks or skills run twice: playbook-hooks@pragmatic-engineer. Run: claude plugin uninstall playbook-hooks@pragmatic-engineer".into(),
                command: None,
            },
        ];

        // Act
        let text = render(&from_open(&open));

        // Assert
        assert_eq!(
            text,
            "2 items need you:\n\
             \x20 - statusLine.command in settings.json still runs the old script.\n\
             \x20   Why: the status line still runs a script that newer playbook no longer ships.\n\
             \x20   Fix: playbook setup --statusline\n\
             \x20 - Installed next to playbook@pragmatic-engineer: playbook-hooks@pragmatic-engineer.\n\
             \x20   Why: hooks and skills run twice in every session.\n\
             \x20   Fix: claude plugin uninstall playbook-hooks@pragmatic-engineer\n\
             To go through every setup change at once, run: playbook setup\n"
        );
    }

    #[test]
    fn the_final_line_names_migrate_only_when_migrate_fixes_an_item() {
        let mut report = item("x", Kind::Report, "a");
        assert!(final_line(&[report.clone()]).ends_with("playbook migrate --status"));
        report.by_migrate = true;
        assert!(final_line(&[report]).ends_with("run: playbook migrate"));
    }

    #[test]
    fn an_old_plugin_is_one_item_with_the_plugin_update_command_and_clears_once_current() {
        // Arrange
        let home = scratch_dir("pending-plugin-old");
        let old = plugin(&home, "0.36.0", "0.36.0");
        install(&home, &old, "0.36.0");

        // Act
        let before = plugin_items(&probe(&home, None, "0.37.0"));
        let new = plugin(&home, "0.37.0", "0.37.0");
        install(&home, &new, "0.37.0");
        let after = plugin_items(&probe(&home, None, "0.37.0"));

        // Assert
        assert_eq!(before.len(), 1, "{before:?}");
        assert_eq!(before[0].slug, PLUGIN_RANGE);
        assert_eq!(
            before[0].fix,
            "claude plugin update playbook@pragmatic-engineer"
        );
        assert!(after.is_empty(), "{after:?}");
    }

    #[test]
    fn a_plugin_that_needs_a_newer_binary_is_one_item_with_the_update_command() {
        // Arrange
        let home = scratch_dir("pending-plugin-new");
        let dir = plugin(&home, "0.38.0", "0.38.0");
        install(&home, &dir, "0.38.0");

        // Act
        let items = plugin_items(&probe(&home, None, "0.37.0"));

        // Assert
        let slugs: Vec<&str> = items.iter().map(|i| i.slug.as_str()).collect();
        assert_eq!(slugs, [PLUGIN_NEEDS_BINARY]);
        assert_eq!(items[0].fix, "playbook update");
    }

    #[test]
    fn a_session_from_a_stale_plugin_folder_is_recognised() {
        // Arrange: Claude Code installed 0.37.0, the session still runs 0.36.0.
        let home = scratch_dir("pending-plugin-stale");
        let old = plugin(&home, "0.36.0", "0.36.0");
        let new = plugin(&home, "0.37.0", "0.37.0");
        install(&home, &new, "0.37.0");

        // Act
        let items = plugin_items(&probe(&home, Some(old), "0.37.0"));
        let current = plugin_items(&probe(&home, Some(new), "0.37.0"));

        // Assert
        let slugs: Vec<&str> = items.iter().map(|i| i.slug.as_str()).collect();
        assert_eq!(
            slugs,
            [PLUGIN_STALE_SESSION],
            "the installed plugin is current"
        );
        assert!(items[0].what.contains("0.36.0") && items[0].what.contains("0.37.0"));
        assert!(current.is_empty());
    }

    #[test]
    fn the_plugin_check_replaces_the_migration_copy_of_the_range_item() {
        let open = [OpenItem {
            id: PLUGIN_RANGE,
            message: "old. Run: x".into(),
            command: None,
        }];
        assert!(merge(&open, Vec::new()).is_empty());
    }
}
