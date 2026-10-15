// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Self healing after an upgrade (ADR 0034). A version record in the `state`
//! table says which binary last ran the `Owned` migrations cleanly. The
//! first-run step, the session hook, `migrate` and `init` compare it with the
//! running binary, run what is safe, and report what needs the user. Nothing
//! here writes a Claude Code settings file.

use crate::init::migrate::{self, Ctx, OpenItem};
use crate::init::run::{StepReport, StepStatus};
use std::cmp::Ordering;
use std::path::Path;
use std::time::{Duration, Instant};

const BINARY_KEY: &str = "versions/binary";
const SCHEMA_KEY: &str = "versions/schema";
const PLUGIN_KEY: &str = "versions/plugin";
const PENDING_KEY: &str = "versions/pending";
const FAILED_AT_KEY: &str = "versions/failed-at";

/// How long the hook waits before it retries after a failed run.
const RETRY_AFTER_SECS: u64 = 24 * 60 * 60;

/// The note for a run the deadline stopped. It records nothing, so the next
/// run starts the rest.
const TIME_LIMIT_NOTE: &str = "stopped at the time limit; the rest runs next time";

/// How long the first-run step may start new migrations before the command
/// goes on. The rest runs on the next command.
pub const START_BUDGET: Duration = Duration::from_secs(2);

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
        Ok(Record::from_rows(&crate::state::list(root, "versions/")?))
    }

    /// The record read without changing anything on disk, for `doctor` and
    /// the dry runs. A missing database reads as an empty record.
    pub fn read_only(root: &Path) -> Result<Record, crate::state::StateError> {
        Ok(Record::from_rows(&crate::state::list_read_only(
            root,
            "versions/",
        )?))
    }

    fn from_rows(rows: &[(String, String)]) -> Record {
        let get = |key: &str| {
            rows.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .filter(|v| !v.is_empty())
        };
        Record {
            binary: get(BINARY_KEY),
            schema: get(SCHEMA_KEY).and_then(|v| v.parse().ok()).unwrap_or(0),
            plugin: get(PLUGIN_KEY),
            pending: get(PENDING_KEY)
                .map(|v| v.split(',').map(str::to_string).collect())
                .unwrap_or_default(),
            failed_at: get(FAILED_AT_KEY).and_then(|v| v.parse().ok()),
        }
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
    /// How the record compares with `binary` at `schema`: `Less` when a heal
    /// is due, `Greater` after a downgrade.
    pub fn compare_with(&self, binary: &str, schema: u32) -> Ordering {
        let Some(recorded) = &self.binary else {
            return Ordering::Less;
        };
        crate::update::resolve::compare(recorded, binary).then(self.schema.cmp(&schema))
    }
}

/// Writes one key of the record, so a run never copies back fields another
/// process changed since this one read them.
fn set_key(root: &Path, key: &str, value: &str) -> bool {
    crate::state::transaction(root, |tx| tx.set(key, value)).is_ok()
}

/// Who runs the heal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The `session-init` hook: never waits for the lock, retries once a day.
    Hook,
    /// `playbook migrate` and `doctor --fix`: waits for the lock, retries at once.
    Fix,
    /// The first-run step before a command: never waits for the lock, starts
    /// no migration after `deadline`, and retries after a failure once a day.
    Start { deadline: Instant },
    /// `playbook setup`, after it ran every migration itself. `statusline` is
    /// false when the user declined the status line, so its item is dropped.
    /// The dead hook item stays even when hooks were left off, because those
    /// entries fail on every event.
    Init { failed: bool, statusline: bool },
}

/// Why a heal ran nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    StateUnreadable,
    Downgrade,
    WaitingAfterFailure,
    LockBusy,
}

impl Skip {
    /// The stable token for JSON output.
    pub fn code(self) -> &'static str {
        match self {
            Skip::StateUnreadable => "state_unreadable",
            Skip::Downgrade => "downgrade",
            Skip::WaitingAfterFailure => "waiting_after_failure",
            Skip::LockBusy => "lock_busy",
        }
    }

    pub fn reason(self) -> &'static str {
        match self {
            Skip::StateUnreadable => "the state table could not be read",
            Skip::Downgrade => "a newer playbook ran here; nothing runs after a downgrade",
            Skip::WaitingAfterFailure => "the last run failed; the hook retries once a day",
            Skip::LockBusy => "another playbook process holds the migration lock",
        }
    }
}

/// What a heal did and what is left for the user.
#[derive(Default)]
pub struct Outcome {
    /// Changes the `Owned` migrations made.
    pub changed: Vec<StepReport>,
    /// `Owned` migrations that failed. The record stays on the old binary.
    pub failed: Vec<StepReport>,
    /// Warnings and notes the migrations printed.
    pub warnings: Vec<String>,
    /// Items that need the user.
    pub open: Vec<OpenItem>,
    /// Whether the record now names this binary.
    pub recorded: bool,
    /// Whether the deadline stopped the run before every migration started.
    pub timed_out: bool,
    /// The breaking changes not shown yet (ADR 0035), already recorded as
    /// shown. The caller shows them.
    pub notices: crate::notices::Notices,
    /// Why nothing ran, when nothing did.
    pub skipped: Option<Skip>,
}

impl Outcome {
    fn skipped(skip: Skip) -> Outcome {
        Outcome {
            skipped: Some(skip),
            ..Outcome::default()
        }
    }

    /// The `doctor --fix` exit code: 1 when a migration failed, could not
    /// run, or the record was not written, so `playbook update` reports it.
    /// A downgrade is not a failure.
    pub fn exit_code(&self) -> i32 {
        let blocked = matches!(self.skipped, Some(Skip::StateUnreadable | Skip::LockBusy));
        let unrecorded = self.skipped.is_none() && !self.recorded;
        i32::from(blocked || unrecorded || !self.failed.is_empty())
    }

    /// One line for the session message, or `None` when there is nothing to say.
    pub fn line(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.changed.is_empty() {
            parts.push(format!(
                "updated {} of its own files after the upgrade",
                self.changed.len()
            ));
        }
        if !self.failed.is_empty() {
            parts.push(format!(
                "{} migration(s) failed: run `playbook migrate`",
                self.failed.len()
            ));
        }
        let mut commands: Vec<&str> = self.open.iter().filter_map(|i| i.command).collect();
        let consent = commands.len();
        commands.sort_unstable();
        commands.dedup();
        if consent > 0 {
            let run = commands
                .iter()
                .map(|c| format!("`{c}`"))
                .collect::<Vec<_>>()
                .join(" and ");
            parts.push(format!(
                "{consent} change(s) to your settings or shell files need {run}"
            ));
        }
        let reports = self.open.len() - consent;
        if reports > 0 {
            parts.push(format!(
                "{reports} item(s) need you: run `playbook migrate`"
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

/// The `doctor --fix` report: what changed, what failed, then one line per
/// open item with the command that fixes it.
pub fn render_fix(out: &Outcome) -> String {
    let mut text = String::new();
    if let Some(skip) = out.skipped {
        text.push_str(&format!("nothing ran: {}\n", skip.reason()));
    }
    for step in &out.changed {
        text.push_str(&format!("fixed: {}: {}\n", step.name, step.detail));
    }
    for step in &out.failed {
        text.push_str(&format!("failed: {}: {}\n", step.name, step.detail));
    }
    for warning in &out.warnings {
        text.push_str(&format!("note: {warning}\n"));
    }
    for item in &out.open {
        text.push_str(&needs_you(&item.message, item.command));
    }
    if text.is_empty() {
        text.push_str("nothing to fix\n");
    }
    text
}

/// The report line for an item that needs the user, with the command that
/// applies it when there is one.
pub fn needs_you(message: &str, command: Option<&str>) -> String {
    match command {
        Some(command) => format!("needs you: {message} (run `{command}`)\n"),
        None => format!("needs you: {message}\n"),
    }
}

/// Runs the heal for `trigger` at `now` (epoch seconds).
pub fn run(ctx: &Ctx, trigger: Trigger, now: u64) -> Outcome {
    let root = crate::common::paths::playbook_root_from(&ctx.home);
    // Before the store opens for writing, which imports the old files.
    let older = crate::notices::older_binary_ran(&root);
    let Ok(record) = Record::read(&root) else {
        return Outcome::skipped(Skip::StateUnreadable);
    };
    let order = record.compare_with(env!("CARGO_PKG_VERSION"), migrate::schema());
    if order == Ordering::Greater {
        return Outcome::skipped(Skip::Downgrade);
    }
    // Read before the heal moves the record, and recorded as shown once the
    // heal returns, whether it succeeded or not.
    let pending = crate::notices::prepare(
        &root,
        &ctx.claude_home,
        record.binary.as_deref(),
        record.plugin.as_deref(),
        older,
    );
    let out = heal_for(ctx, trigger, now, &root, &record, order);
    Outcome {
        notices: pending.commit(&root),
        ..out
    }
}

fn heal_for(
    ctx: &Ctx,
    trigger: Trigger,
    now: u64,
    root: &Path,
    record: &Record,
    order: Ordering,
) -> Outcome {
    match trigger {
        Trigger::Hook if order == Ordering::Equal => recheck_pending(ctx, root, record),
        Trigger::Start { .. } if order == Ordering::Equal => Outcome::default(),
        Trigger::Hook | Trigger::Start { .. } => {
            let waiting = record
                .failed_at
                .is_some_and(|at| now.saturating_sub(at) < RETRY_AFTER_SECS);
            if waiting {
                return Outcome {
                    open: migrate::open_items(ctx, Some(&record.pending)),
                    ..Outcome::skipped(Skip::WaitingAfterFailure)
                };
            }
            let deadline = match trigger {
                Trigger::Start { deadline } => Some(deadline),
                _ => None,
            };
            heal_now(ctx, root, 0, now, deadline)
        }
        Trigger::Fix => {
            let mut out = heal_now(ctx, root, migrate::LOCK_RETRIES, now, None);
            out.warnings.extend(migrate::owned_notes(ctx));
            out
        }
        Trigger::Init { failed, statusline } => {
            let mut open = migrate::open_items(ctx, None);
            open.retain(|i| statusline || i.id != migrate::STATUSLINE_MIGRATION);
            let out = Outcome {
                open,
                ..Outcome::default()
            };
            if failed {
                out
            } else {
                record_clean(ctx, root, out)
            }
        }
    }
}

/// The hook on a binary the record already names: rerun only the checks
/// still open, and drop the ones that cleared.
fn recheck_pending(ctx: &Ctx, root: &Path, record: &Record) -> Outcome {
    if record.pending.is_empty() {
        return Outcome::default();
    }
    let open = migrate::open_items(ctx, Some(&record.pending));
    let pending = ids(&open);
    if pending != record.pending {
        let _ = set_key(root, PENDING_KEY, &pending.join(","));
    }
    Outcome {
        open,
        recorded: true,
        ..Outcome::default()
    }
}

/// Runs the `Owned` migrations, then records this binary when they all
/// finished, or the failure time when one did not. A run that hit `deadline`
/// records neither, so the next run goes on from there.
fn heal_now(ctx: &Ctx, root: &Path, retries: u32, now: u64, deadline: Option<Instant>) -> Outcome {
    let report = migrate::run_owned(ctx, retries, deadline);
    if report.lock_busy {
        return Outcome::skipped(Skip::LockBusy);
    }
    let (failed, changed) = report
        .steps
        .into_iter()
        .filter(|s| matches!(s.status, StepStatus::Wired | StepStatus::Failed))
        .partition(|s| s.status == StepStatus::Failed);
    let mut out = Outcome {
        changed,
        failed,
        warnings: report.warnings,
        ..Outcome::default()
    };
    if report.timed_out {
        out.timed_out = true;
        out.warnings.push(TIME_LIMIT_NOTE.into());
        return out;
    }
    if report.failed {
        if !set_key(root, FAILED_AT_KEY, &now.to_string()) {
            out.warnings
                .push("could not save the failure time, so the hook may retry at once".into());
        }
        return out;
    }
    record_clean(
        ctx,
        root,
        Outcome {
            open: migrate::open_items(ctx, None),
            ..out
        },
    )
}

fn record_clean(ctx: &Ctx, root: &Path, mut out: Outcome) -> Outcome {
    let env = std::env::var("CLAUDE_PLUGIN_ROOT").ok();
    let plugin = crate::plugin_root::find(env.as_deref(), &ctx.claude_home)
        .and_then(|root| crate::plugin_root::plugin_version(&root));
    out.recorded = Record {
        binary: Some(env!("CARGO_PKG_VERSION").to_string()),
        schema: migrate::schema(),
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
    let ctx = Ctx::for_home(home);
    let now = crate::common::time::now_epoch_secs();
    std::panic::catch_unwind(|| {
        let out = run(&ctx, Trigger::Hook, now);
        notice(out.line(), &out.notices)
    })
    .ok()?
}

/// `line` and the breaking changes block, in that order, or `None` when
/// both are empty.
fn notice(line: Option<String>, notices: &crate::notices::Notices) -> Option<String> {
    let block = notices.render();
    let parts: Vec<&str> = line
        .as_deref()
        .into_iter()
        .chain((!block.is_empty()).then_some(block.trim_end()))
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// Whether the first-run step runs before `command`. Hot paths never pay for
/// it (`Command::is_hot_path`), and neither does `usage`, which the session
/// hooks start in the background; the `session-init` hook is the trigger on
/// that side. Commands that run their own heal (`migrate`, `doctor --fix`,
/// `setup` and its old name `init`), replace the binary (`update`) or remove playbook (`uninstall`)
/// skip it too. So do `doctor` and `state`, which show the home as it is;
/// the doctor names `playbook migrate` when a heal is due.
pub fn runs_on_start(command: &crate::Command) -> bool {
    use crate::Command as C;
    !command.is_hot_path()
        && command.setup_flags().is_none()
        && !matches!(
            command,
            C::Usage { .. }
                | C::Migrate { .. }
                | C::Update { .. }
                | C::Uninstall { .. }
                | C::Doctor { .. }
                | C::State { .. }
        )
}

/// The first-run step (ADR 0034 decision 13): before `command` runs, when
/// the record names an older binary, runs the `Owned` migrations once and
/// prints one notice on stderr. It asks nothing, writes no Claude Code
/// settings, skips a busy lock, starts no migration after `START_BUDGET`,
/// and does nothing on a fresh home, after a downgrade, or while it waits a
/// day after a failed run.
pub fn on_start(command: &crate::Command) {
    if !runs_on_start(command) {
        return;
    }
    let home = crate::common::home_dir();
    if crate::common::home_error(&home).is_some() {
        return;
    }
    let deadline = Instant::now() + START_BUDGET;
    if let Ok(Some(text)) = std::panic::catch_unwind(|| start(&home, deadline)) {
        eprintln!("{text}");
    }
}

/// The notice the first-run step prints for `home`, if any. Cheap when there
/// is nothing to do or it waits after a failure: one read of the record,
/// which writes nothing.
fn start(home: &Path, deadline: Instant) -> Option<String> {
    let root = crate::common::paths::playbook_root_from(home);
    if !root.is_dir() {
        return None;
    }
    let record = Record::read_only(&root).ok()?;
    if record.compare_with(env!("CARGO_PKG_VERSION"), migrate::schema()) != Ordering::Less {
        return None;
    }
    let now = crate::common::time::now_epoch_secs();
    if record
        .failed_at
        .is_some_and(|at| now.saturating_sub(at) < RETRY_AFTER_SECS)
    {
        return None;
    }
    let out = run(&Ctx::for_home(home), Trigger::Start { deadline }, now);
    if out.skipped.is_some() {
        return notice(None, &out.notices);
    }
    let mut line = out.line();
    if out.timed_out {
        line = Some(match line {
            Some(l) => format!("{l}\nplaybook: {TIME_LIMIT_NOTE}"),
            None => format!("playbook: {TIME_LIMIT_NOTE}"),
        });
    }
    notice(line, &out.notices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;

    fn ctx(home: &Path) -> Ctx {
        Ctx::for_home(home)
    }

    fn root(home: &Path) -> std::path::PathBuf {
        crate::common::paths::playbook_root_from(home)
    }

    fn seed(home: &Path, binary: &str, schema: u32, failed_at: Option<u64>) -> Record {
        let record = Record {
            binary: Some(binary.into()),
            schema,
            failed_at,
            ..Record::default()
        };
        record.write(&root(home)).unwrap();
        record
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
    fn the_first_run_step_stops_at_its_deadline_and_retries_next_time() {
        // Arrange
        let home = scratch_dir("heal-deadline");
        let before = seed(&home, "0.25.0", 1, None);
        let script = root(&home).join("hooks/lib/config-hash.sh");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(&script, "x").unwrap();

        // Act: a deadline that has passed, then one with time left.
        let late = run(
            &ctx(&home),
            Trigger::Start {
                deadline: Instant::now(),
            },
            1000,
        );
        let after_late = Record::read(&root(&home)).unwrap();
        let kept = script.exists();
        let text = start(&home, Instant::now() + START_BUDGET);
        let again = start(&home, Instant::now() + START_BUDGET);

        // Assert
        assert!(!late.recorded && late.failed.is_empty());
        assert!(late.timed_out && late.warnings.iter().any(|w| w == TIME_LIMIT_NOTE));
        assert_eq!(after_late, before);
        assert!(kept);
        assert!(text.is_some_and(|t| t.contains("updated 1 of its own files")));
        assert!(!script.exists());
        assert_eq!(again, None);
    }

    #[test]
    fn the_first_run_step_skips_hot_paths_and_commands_with_their_own_heal() {
        use clap::Parser;
        let parse = |args: &[&str]| {
            let mut all = vec!["playbook"];
            all.extend_from_slice(args);
            crate::Cli::try_parse_from(all).unwrap().command
        };
        for args in [
            &["hook", "session-init"][..],
            &["statusline"],
            &["shell-init"],
            &["version"],
            &["json", "valid-json"],
            &["usage", "warm"],
            &["migrate"],
            &["init", "--yes"],
            &["setup", "--yes"],
            &["update"],
            &["uninstall", "--yes"],
            &["doctor", "--fix"],
            &["doctor", "pending-migrations"],
            &["state", "list"],
        ] {
            assert!(!runs_on_start(&parse(args)), "{args:?}");
        }
        for args in [&["mode", "status"][..], &["plans"], &["config", "list"]] {
            assert!(runs_on_start(&parse(args)), "{args:?}");
        }
    }

    #[test]
    fn breaking_changes_are_recorded_as_shown_when_the_heal_fails() {
        use std::os::unix::fs::PermissionsExt;
        // Arrange: an older record, and a migration that cannot write.
        let home = scratch_dir("heal-notice-fail");
        seed(&home, "0.25.0", 1, None);
        let lib = root(&home).join("hooks/lib");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("config-hash.sh"), "x").unwrap();
        fs::set_permissions(&lib, fs::Permissions::from_mode(0o555)).unwrap();
        if fs::write(lib.join("probe"), "").is_ok() {
            return; // root ignores the mode, so the failure cannot be staged
        }

        // Act
        let first = run(&ctx(&home), Trigger::Fix, 1000);
        let second = run(&ctx(&home), Trigger::Fix, 2000);
        fs::set_permissions(&lib, fs::Permissions::from_mode(0o755)).unwrap();

        // Assert
        assert!(!first.failed.is_empty() && !second.failed.is_empty());
        assert_eq!(
            first.notices.entries,
            crate::breaking::entries_between("0.25.0", env!("CARGO_PKG_VERSION"))
        );
        assert!(!first.notices.entries.is_empty());
        assert!(second.notices.entries.is_empty());
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
        seed(&home, "0.25.0", 1, None);
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
        let newer = seed(&home, "999.0.0", 999, None);

        // Act
        let out = run(&ctx(&home), Trigger::Hook, 1000);
        let fix = run(&ctx(&home), Trigger::Fix, 1000);

        // Assert
        assert_eq!(out.skipped, Some(Skip::Downgrade));
        assert_eq!(fix.exit_code(), 0);
        assert!(render_fix(&fix).starts_with("nothing ran: a newer playbook ran here"));
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
                out.line()
                    .unwrap()
                    .contains("need `playbook setup --statusline`"),
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
        assert_eq!(out.skipped, Some(Skip::LockBusy));
        assert_eq!(out.exit_code(), 1);
        assert_eq!(Record::read(&root(&home)).unwrap().binary, None);
    }

    #[test]
    fn a_recent_failure_waits_a_day_in_the_hook() {
        // Arrange
        let home = scratch_dir("heal-failed");
        seed(&home, "0.25.0", 0, Some(1000));

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
        let init = |failed| Trigger::Init {
            failed,
            statusline: true,
        };
        assert!(!run(&ctx(&home), init(true), 1000).recorded);
        assert!(run(&ctx(&home), init(false), 1000).recorded);
    }

    #[test]
    fn init_without_the_status_line_leaves_its_item_out_of_the_record() {
        // Arrange
        let home = scratch_dir("heal-init-no-statusline");
        legacy_statusline(&home);

        // Act
        let out = run(
            &ctx(&home),
            Trigger::Init {
                failed: false,
                statusline: false,
            },
            1000,
        );

        // Assert
        assert!(out.open.is_empty());
        assert!(Record::read(&root(&home)).unwrap().pending.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_migration_keeps_the_old_record_and_says_it_failed() {
        use std::os::unix::fs::PermissionsExt;
        // Arrange: the config-hash script sits in a folder nobody can write.
        let home = scratch_dir("heal-fail-real");
        seed(&home, "0.25.0", 1, None);
        let lib = root(&home).join("hooks/lib");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("config-hash.sh"), "x").unwrap();
        fs::set_permissions(&lib, fs::Permissions::from_mode(0o555)).unwrap();
        if fs::write(lib.join("probe"), "").is_ok() {
            return; // root ignores the mode, so the failure cannot be staged
        }

        // Act
        let out = run(&ctx(&home), Trigger::Hook, 5000);
        fs::set_permissions(&lib, fs::Permissions::from_mode(0o755)).unwrap();

        // Assert
        assert!(!out.recorded);
        assert_eq!(out.failed.len(), 1);
        assert!(out.changed.is_empty());
        assert_eq!(out.exit_code(), 1);
        assert!(out
            .line()
            .unwrap()
            .contains("1 migration(s) failed: run `playbook migrate`"));
        assert!(render_fix(&out).contains("failed: config-hash: "));
        let rec = Record::read(&root(&home)).unwrap();
        assert_eq!(rec.binary.as_deref(), Some("0.25.0"));
        assert_eq!(rec.failed_at, Some(5000));
    }

    #[test]
    fn doctor_fix_retries_at_once_and_names_the_command_per_item() {
        // Arrange: a recent failure and a pending status line change.
        let home = scratch_dir("heal-fix");
        seed(&home, "0.25.0", 0, Some(1000));
        legacy_statusline(&home);

        // Act
        let out = run(&ctx(&home), Trigger::Fix, 1060);

        // Assert
        assert_eq!(out.skipped, None);
        assert!(out.recorded);
        assert_eq!(out.exit_code(), 0);
        assert_eq!(Record::read(&root(&home)).unwrap().failed_at, None);
        assert!(render_fix(&out).contains("(run `playbook setup --statusline`)"));
    }

    #[test]
    fn the_fix_report_has_one_line_per_change_and_note() {
        let out = Outcome {
            changed: vec![StepReport::wired("config-hash", "removed")],
            warnings: vec!["0003-system-prompt-edited: edited".into()],
            recorded: true,
            ..Outcome::default()
        };
        assert_eq!(
            render_fix(&out),
            "fixed: config-hash: removed\nnote: 0003-system-prompt-edited: edited\n"
        );
        assert_eq!(out.exit_code(), 0);
    }
}
