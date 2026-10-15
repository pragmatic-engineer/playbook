// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Self healing after an upgrade (ADR 0034). A version record in the `state`
//! table says which binary last ran the `Owned` migrations cleanly. The
//! session hook, `doctor --fix` and `init` compare it with the running binary,
//! run what is safe, and report what needs the user. Nothing here writes a
//! Claude Code settings file.

use crate::init::migrate::{self, Ctx, OpenItem};
use crate::init::run::{StepReport, StepStatus};
use std::cmp::Ordering;
use std::path::Path;

const BINARY_KEY: &str = "versions/binary";
const SCHEMA_KEY: &str = "versions/schema";
const PLUGIN_KEY: &str = "versions/plugin";
const PENDING_KEY: &str = "versions/pending";
const FAILED_AT_KEY: &str = "versions/failed-at";

/// How long the hook waits before it retries after a failed run.
const RETRY_AFTER_SECS: u64 = 24 * 60 * 60;

/// The version record. Every field is empty on a machine that never ran it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    pub binary: Option<String>,
    pub schema: u32,
    pub plugin: Option<String>,
    pub pending: Vec<String>,
    pub failed_at: Option<u64>,
}

impl Record {
    pub fn read(root: &Path) -> Result<Record, crate::state::StateError> {
        let rows = crate::state::list(root, "versions/")?;
        let get = |key: &str| {
            rows.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .filter(|v| !v.is_empty())
        };
        Ok(Record {
            binary: get(BINARY_KEY),
            schema: get(SCHEMA_KEY).and_then(|v| v.parse().ok()).unwrap_or(0),
            plugin: get(PLUGIN_KEY),
            pending: get(PENDING_KEY)
                .map(|v| v.split(',').map(str::to_string).collect())
                .unwrap_or_default(),
            failed_at: get(FAILED_AT_KEY).and_then(|v| v.parse().ok()),
        })
    }

    fn write(&self, root: &Path) -> Result<(), crate::state::StateError> {
        crate::state::transaction(root, |tx| {
            tx.set(BINARY_KEY, self.binary.as_deref().unwrap_or(""))?;
            tx.set(SCHEMA_KEY, &self.schema.to_string())?;
            tx.set(PLUGIN_KEY, self.plugin.as_deref().unwrap_or(""))?;
            tx.set(PENDING_KEY, &self.pending.join(","))?;
            match self.failed_at {
                Some(at) => tx.set(FAILED_AT_KEY, &at.to_string())?,
                None => {
                    tx.delete(FAILED_AT_KEY)?;
                }
            }
            Ok(())
        })
    }

    /// How the record compares with this binary: `Less` when this binary is
    /// newer (or nothing ran yet), `Greater` after a downgrade.
    fn compare_with(&self, binary: &str, schema: u32) -> Ordering {
        let Some(recorded) = &self.binary else {
            return Ordering::Less;
        };
        crate::update::resolve::compare(recorded, binary).then(self.schema.cmp(&schema))
    }
}

/// Who runs the heal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The `session-init` hook: never waits for the lock, retries once a day.
    Hook,
    /// `playbook doctor --fix`: waits for the lock, retries at once.
    Fix,
    /// `playbook init`, after it ran every migration itself.
    Init { failed: bool },
}

/// What a heal did and what is left for the user.
#[derive(Default)]
pub struct Outcome {
    /// Changes the `Owned` migrations made.
    pub changed: Vec<StepReport>,
    /// Warnings the migrations printed.
    pub warnings: Vec<String>,
    /// Items that need the user.
    pub open: Vec<OpenItem>,
    /// Whether the record now names this binary.
    pub recorded: bool,
    /// Why nothing ran, when nothing did.
    pub skipped: Option<&'static str>,
}

impl Outcome {
    /// One line for the session message, or `None` when there is nothing to say.
    pub fn line(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.changed.is_empty() {
            parts.push(format!(
                "updated {} of its own files after the upgrade",
                self.changed.len()
            ));
        }
        let consent = self.open.iter().filter(|i| i.needs_init).count();
        if consent > 0 {
            parts.push(format!(
                "{consent} change(s) to your settings or shell files need `playbook init`"
            ));
        }
        let reports = self.open.len() - consent;
        if reports > 0 {
            parts.push(format!(
                "{reports} item(s) need you: run `playbook doctor --fix`"
            ));
        }
        (!parts.is_empty()).then(|| {
            format!(
                "playbook {}: {}.",
                env!("CARGO_PKG_VERSION"),
                parts.join("; ")
            )
        })
    }
}

/// The `doctor --fix` report: what changed, then one line per open item with
/// the command that fixes it.
pub fn render_fix(out: &Outcome) -> String {
    let mut text = String::new();
    if let Some(reason) = out.skipped {
        text.push_str(&format!("nothing ran: {reason}\n"));
    }
    for step in &out.changed {
        text.push_str(&format!("fixed: {}: {}\n", step.name, step.detail));
    }
    for warning in &out.warnings {
        text.push_str(&format!("note: {warning}\n"));
    }
    for item in &out.open {
        let command = if item.needs_init {
            "playbook init"
        } else {
            "see the message"
        };
        text.push_str(&format!("needs you: {} ({command})\n", item.message));
    }
    if text.is_empty() {
        text.push_str("nothing to fix\n");
    }
    text
}

/// Runs the heal for `trigger` at `now` (epoch seconds).
pub fn run(ctx: &Ctx, trigger: Trigger, now: u64) -> Outcome {
    let root = crate::common::paths::playbook_root_from(&ctx.home);
    let binary = env!("CARGO_PKG_VERSION");
    let schema = migrate::schema();
    let Ok(record) = Record::read(&root) else {
        return Outcome {
            skipped: Some("the state table could not be read"),
            ..Outcome::default()
        };
    };
    let order = record.compare_with(binary, schema);
    if order == Ordering::Greater {
        return Outcome {
            skipped: Some("a newer playbook ran here; nothing runs after a downgrade"),
            ..Outcome::default()
        };
    }

    if trigger == Trigger::Hook && order == Ordering::Equal {
        if record.pending.is_empty() {
            return Outcome::default();
        }
        let open = migrate::open_items(ctx, Some(&record.pending));
        let pending = ids(&open);
        if pending != record.pending {
            let _ = Record {
                pending,
                ..record.clone()
            }
            .write(&root);
        }
        return Outcome {
            open,
            recorded: true,
            ..Outcome::default()
        };
    }

    let failed = match trigger {
        Trigger::Init { failed } => failed,
        Trigger::Hook | Trigger::Fix => {
            if trigger == Trigger::Hook
                && record
                    .failed_at
                    .is_some_and(|at| now.saturating_sub(at) < RETRY_AFTER_SECS)
            {
                return Outcome {
                    open: migrate::open_items(ctx, Some(&record.pending)),
                    skipped: Some("the last run failed; the hook retries once a day"),
                    ..Outcome::default()
                };
            }
            let retries = if trigger == Trigger::Hook { 0 } else { 50 };
            let owned = migrate::run_owned(ctx, retries);
            if !owned.locked {
                return Outcome {
                    skipped: Some("another playbook process holds the migration lock"),
                    ..Outcome::default()
                };
            }
            let mut out = Outcome {
                changed: owned
                    .report
                    .steps
                    .into_iter()
                    .filter(|s| matches!(s.status, StepStatus::Wired | StepStatus::Failed))
                    .collect(),
                warnings: owned.report.warnings,
                ..Outcome::default()
            };
            if owned.failed {
                let _ = Record {
                    failed_at: Some(now),
                    ..record
                }
                .write(&root);
                return out;
            }
            out.open = migrate::open_items(ctx, None);
            return record_clean(&root, out, binary, schema);
        }
    };
    let out = Outcome {
        open: migrate::open_items(ctx, None),
        ..Outcome::default()
    };
    if failed {
        return out;
    }
    record_clean(&root, out, binary, schema)
}

fn record_clean(root: &Path, mut out: Outcome, binary: &str, schema: u32) -> Outcome {
    let plugin = crate::plugin_root::resolve_here()
        .ok()
        .and_then(|r| crate::plugin_root::plugin_version(&r.root));
    out.recorded = Record {
        binary: Some(binary.to_string()),
        schema,
        plugin,
        pending: ids(&out.open),
        failed_at: None,
    }
    .write(root)
    .is_ok();
    out
}

fn ids(open: &[OpenItem]) -> Vec<String> {
    open.iter().map(|i| i.id.to_string()).collect()
}

/// The `session-init` entry point: fails open and returns the one line to
/// show, if any.
pub fn session_line(home: &Path) -> Option<String> {
    let ctx = Ctx {
        home: home.to_path_buf(),
        claude_home: home.join(".claude"),
        self_root: None,
        repo: None,
    };
    let now = crate::common::time::now_epoch_secs();
    std::panic::catch_unwind(|| run(&ctx, Trigger::Hook, now))
        .ok()?
        .line()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;

    fn ctx(home: &Path) -> Ctx {
        Ctx {
            home: home.to_path_buf(),
            claude_home: home.join(".claude"),
            self_root: None,
            repo: None,
        }
    }

    fn root(home: &Path) -> std::path::PathBuf {
        crate::common::paths::playbook_root_from(home)
    }

    fn legacy_statusline(home: &Path) {
        fs::create_dir_all(home.join(".claude")).unwrap();
        let cmd = crate::init::statusline::playbook_statusline_path(home);
        fs::write(
            home.join(".claude/settings.json"),
            format!(
                r#"{{"statusLine":{{"type":"command","command":"bash {}"}}}}"#,
                cmd.display()
            ),
        )
        .unwrap();
    }

    #[test]
    fn a_fresh_machine_records_this_binary_and_writes_nothing_else() {
        // Arrange
        let home = scratch_dir("heal-fresh");

        // Act
        let out = run(&ctx(&home), Trigger::Hook, 1000);

        // Assert
        assert!(out.recorded);
        assert!(out.line().is_none(), "{:?}", out.line());
        let rec = Record::read(&root(&home)).unwrap();
        assert_eq!(rec.binary.as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(rec.schema, migrate::schema());
        assert!(!home.join(".claude").exists());
    }

    #[test]
    fn a_second_session_on_the_same_binary_does_nothing() {
        // Arrange
        let home = scratch_dir("heal-same");
        run(&ctx(&home), Trigger::Hook, 1000);
        let before = Record::read(&root(&home)).unwrap();

        // Act
        let out = run(&ctx(&home), Trigger::Hook, 2000);

        // Assert
        assert!(out.line().is_none());
        assert_eq!(Record::read(&root(&home)).unwrap(), before);
    }

    #[test]
    fn an_older_record_runs_the_owned_migrations_and_moves_the_record_up() {
        // Arrange: an old config-hash script left by an earlier install.
        let home = scratch_dir("heal-upgrade");
        Record {
            binary: Some("0.25.0".into()),
            schema: 1,
            ..Record::default()
        }
        .write(&root(&home))
        .unwrap();
        let script = root(&home).join("hooks/lib/config-hash.sh");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(&script, "x").unwrap();

        // Act
        let out = run(&ctx(&home), Trigger::Hook, 1000);

        // Assert
        assert!(!script.exists());
        assert_eq!(out.changed.len(), 1, "{:?}", out.line());
        assert!(out.line().unwrap().contains("updated 1 of its own files"));
        assert_eq!(
            Record::read(&root(&home)).unwrap().binary.as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn a_downgrade_runs_nothing_and_keeps_the_newer_record() {
        // Arrange
        let home = scratch_dir("heal-downgrade");
        let newer = Record {
            binary: Some("999.0.0".into()),
            schema: 999,
            ..Record::default()
        };
        newer.write(&root(&home)).unwrap();

        // Act
        let out = run(&ctx(&home), Trigger::Hook, 1000);

        // Assert
        assert!(out.skipped.is_some());
        assert_eq!(Record::read(&root(&home)).unwrap(), newer);
    }

    #[test]
    fn a_consent_item_is_reported_each_session_and_settings_stay_untouched() {
        // Arrange
        let home = scratch_dir("heal-consent");
        legacy_statusline(&home);
        let settings = fs::read(home.join(".claude/settings.json")).unwrap();

        // Act
        let first = run(&ctx(&home), Trigger::Hook, 1000);
        let second = run(&ctx(&home), Trigger::Hook, 2000);

        // Assert
        for out in [&first, &second] {
            assert!(
                out.line().unwrap().contains("need `playbook init`"),
                "{:?}",
                out.line()
            );
        }
        assert_eq!(
            Record::read(&root(&home)).unwrap().pending,
            vec!["0006-statusline-rust-command".to_string()]
        );
        assert_eq!(
            fs::read(home.join(".claude/settings.json")).unwrap(),
            settings
        );
    }

    #[test]
    fn a_fixed_consent_item_drops_out_of_the_pending_list() {
        // Arrange
        let home = scratch_dir("heal-consent-fixed");
        legacy_statusline(&home);
        run(&ctx(&home), Trigger::Hook, 1000);
        fs::write(
            home.join(".claude/settings.json"),
            r#"{"statusLine":{"type":"command","command":"playbook statusline"}}"#,
        )
        .unwrap();

        // Act
        let out = run(&ctx(&home), Trigger::Hook, 2000);

        // Assert
        assert!(out.line().is_none());
        assert!(Record::read(&root(&home)).unwrap().pending.is_empty());
    }

    #[test]
    fn a_held_lock_leaves_the_record_unchanged() {
        // Arrange
        let home = scratch_dir("heal-locked");
        fs::create_dir_all(root(&home).join("migrations.lock")).unwrap();

        // Act
        let out = run(&ctx(&home), Trigger::Hook, 1000);

        // Assert
        assert!(out.skipped.is_some());
        assert_eq!(Record::read(&root(&home)).unwrap().binary, None);
    }

    #[test]
    fn a_recent_failure_waits_a_day_in_the_hook() {
        // Arrange
        let home = scratch_dir("heal-failed");
        Record {
            binary: Some("0.25.0".into()),
            failed_at: Some(1000),
            ..Record::default()
        }
        .write(&root(&home))
        .unwrap();

        // Act
        let soon = run(&ctx(&home), Trigger::Hook, 1000 + 60);
        let later = run(&ctx(&home), Trigger::Hook, 1000 + RETRY_AFTER_SECS + 1);

        // Assert
        assert!(soon.skipped.is_some());
        assert!(later.recorded);
    }

    #[test]
    fn init_records_only_when_its_migrations_did_not_fail() {
        let home = scratch_dir("heal-init");
        assert!(!run(&ctx(&home), Trigger::Init { failed: true }, 1000).recorded);
        assert!(run(&ctx(&home), Trigger::Init { failed: false }, 1000).recorded);
    }
}
