// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook migrate`, the one front door to the migration runner (ADR 0034).
//! A plain run is the same heal `doctor --fix` runs. `--status` and
//! `--dry-run` read only.

use crate::heal::{self, Outcome, Record, Trigger};
use crate::init::migrate::{self, Ctx, Planned, State};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::path::Path;

/// What `playbook migrate` was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Run,
    Status,
    DryRun,
}

/// Runs `playbook migrate` for `home`, prints the result, and returns the
/// exit code: 1 when the record could not be read or written, a migration
/// failed, or the lock was busy, else 0.
pub fn run(home: &Path, mode: Mode, json: bool) -> i32 {
    let ctx = Ctx::for_home(home);
    match mode {
        Mode::Run => {
            let out = heal::run(&ctx, Trigger::Fix, crate::common::time::now_epoch_secs());
            eprint!("{}", out.notices.render());
            if json {
                println!("{}", run_json(&out));
            } else {
                print!("{}", heal::render_fix(&out));
            }
            out.exit_code()
        }
        Mode::Status | Mode::DryRun => {
            let root = crate::common::paths::playbook_root_from(home);
            let (Ok(record), Some(plan)) = (Record::read_only(&root), migrate::plan(&ctx)) else {
                eprintln!("playbook migrate: the upgrade record could not be read");
                return 1;
            };
            let text = match (mode, json) {
                (Mode::Status, true) => format!("{}\n", status_json(&record, &plan)),
                (Mode::Status, false) => render_status(&record, &plan),
                (_, true) => format!("{}\n", dry_run_json(&record, &plan)),
                (_, false) => render_dry_run(&record, &plan),
            };
            print!("{text}");
            0
        }
    }
}

/// Whether a newer playbook wrote the record, so a run would do nothing.
fn downgraded(record: &Record) -> bool {
    record.compare_with(env!("CARGO_PKG_VERSION"), migrate::schema()) == Ordering::Greater
}

fn recorded_line(record: &Record) -> String {
    match &record.binary {
        Some(b) => format!("recorded: playbook {b}, migrations up to {}", record.schema),
        None => "recorded: nothing yet".to_string(),
    }
}

fn state_text(state: &State) -> String {
    match state {
        State::Applied => "applied".into(),
        State::Pending => "pending".into(),
        State::EveryRun => "runs on every run".into(),
        State::Clear => "nothing to do".into(),
        State::Open { message, command } => heal::needs_you(message, *command).trim_end().into(),
        State::Unknown => "not checked: old state files are not imported yet".into(),
    }
}

fn render_status(record: &Record, plan: &[Planned]) -> String {
    let mut text = format!(
        "running: playbook {}, migrations up to {}\n{}\n",
        env!("CARGO_PKG_VERSION"),
        migrate::schema(),
        recorded_line(record)
    );
    let width = plan.iter().map(|p| p.id.len()).max().unwrap_or(0);
    for p in plan {
        text.push_str(&format!(
            "{:<width$}  {:<10}  {:<7}  {}\n",
            p.id,
            p.kind,
            p.scope,
            state_text(&p.state)
        ));
    }
    text
}

/// The migrations a run would start now, in order: none after a downgrade.
fn would_run(record: &Record, plan: &[Planned]) -> Vec<&'static str> {
    if downgraded(record) {
        return Vec::new();
    }
    plan.iter().filter(|p| p.runs).map(|p| p.id).collect()
}

/// The open items of `plan`: `Owned` ones are notes, the rest need the user.
fn open_items(plan: &[Planned]) -> impl Iterator<Item = (&Planned, &String, Option<&'static str>)> {
    plan.iter().filter_map(|p| match &p.state {
        State::Open { message, command } => Some((p, message, *command)),
        _ => None,
    })
}

fn render_dry_run(record: &Record, plan: &[Planned]) -> String {
    let run = would_run(record, plan);
    let mut text = if downgraded(record) {
        format!("nothing would run: {}\n", heal::Skip::Downgrade.reason())
    } else if run.is_empty() {
        "nothing would run\n".to_string()
    } else {
        format!("would run: {}\n", run.join(", "))
    };
    for (p, message, command) in open_items(plan) {
        if p.scope == "owned" {
            text.push_str(&format!("note: {}: {message}\n", p.id));
        } else {
            text.push_str(&heal::needs_you(message, command));
        }
    }
    text.push_str("dry run: nothing was written\n");
    text
}

fn record_json(record: &Record) -> Value {
    json!({
        "binary": record.binary,
        "schema": record.schema,
        "plugin": record.plugin,
        "pending": record.pending,
        "failed_at": record.failed_at,
    })
}

fn status_json(record: &Record, plan: &[Planned]) -> Value {
    let migrations: Vec<Value> = plan
        .iter()
        .map(|p| {
            let (message, command) = match &p.state {
                State::Open { message, command } => (Some(message.as_str()), *command),
                _ => (None, None),
            };
            json!({
                "id": p.id,
                "kind": p.kind,
                "scope": p.scope,
                "state": p.state.code(),
                "runs": p.runs,
                "message": message,
                "command": command,
            })
        })
        .collect();
    json!({
        "running": {"binary": env!("CARGO_PKG_VERSION"), "schema": migrate::schema()},
        "recorded": record_json(record),
        "migrations": migrations,
    })
}

fn dry_run_json(record: &Record, plan: &[Planned]) -> Value {
    let (notes, open): (Vec<_>, Vec<_>) =
        open_items(plan).partition(|(p, _, _)| p.scope == "owned");
    json!({
        "would_run": would_run(record, plan),
        "skipped": downgraded(record).then(|| heal::Skip::Downgrade.code()),
        "notes": notes
            .iter()
            .map(|(p, message, _)| format!("{}: {message}", p.id))
            .collect::<Vec<_>>(),
        "open": open
            .iter()
            .map(|(p, message, command)| json!({"id": p.id, "message": message, "command": command}))
            .collect::<Vec<_>>(),
        "written": false,
    })
}

fn run_json(out: &Outcome) -> Value {
    let steps = |list: &[crate::init::run::StepReport]| {
        list.iter()
            .map(|s| json!({"name": s.name, "detail": s.detail}))
            .collect::<Vec<_>>()
    };
    json!({
        "skipped": out.skipped.map(heal::Skip::code),
        "reason": out.skipped.map(heal::Skip::reason),
        "changed": steps(&out.changed),
        "failed": steps(&out.failed),
        "notes": out.warnings,
        "open": out
            .open
            .iter()
            .map(|i| json!({"id": i.id, "message": i.message, "command": i.command}))
            .collect::<Vec<_>>(),
        "recorded": out.recorded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(binary: Option<&str>, schema: u32) -> Record {
        Record {
            binary: binary.map(str::to_string),
            schema,
            ..Record::default()
        }
    }

    fn planned(id: &'static str, scope: &'static str, state: State, runs: bool) -> Planned {
        Planned {
            id,
            kind: "idempotent",
            scope,
            state,
            runs,
        }
    }

    fn sample() -> Vec<Planned> {
        vec![
            planned("0001-a", "owned", State::EveryRun, true),
            planned("0002-b", "owned", State::Applied, false),
            planned(
                "0006-c",
                "consent",
                State::Open {
                    message: "old command".into(),
                    command: Some("playbook setup --statusline"),
                },
                false,
            ),
            planned(
                "0014-d",
                "owned",
                State::Open {
                    message: "old folders".into(),
                    command: None,
                },
                false,
            ),
        ]
    }

    #[test]
    fn a_dry_run_names_what_a_run_starts_unless_a_newer_playbook_ran() {
        // Arrange
        let plan = sample();

        // Act
        let old = render_dry_run(&record(Some("0.25.0"), 1), &plan);
        let same = render_dry_run(
            &record(Some(env!("CARGO_PKG_VERSION")), migrate::schema()),
            &plan,
        );
        let newer = render_dry_run(&record(Some("999.0.0"), 999), &plan);

        // Assert
        assert!(old.starts_with("would run: 0001-a\n"), "{old}");
        assert!(old.contains("needs you: old command (run `playbook setup --statusline`)"));
        assert!(old.contains("note: 0014-d: old folders\n"), "{old}");
        assert!(old.ends_with("dry run: nothing was written\n"));
        assert!(same.starts_with("would run: 0001-a\n"), "{same}");
        assert!(
            newer.starts_with("nothing would run: a newer playbook"),
            "{newer}"
        );
        let value = dry_run_json(&record(Some("999.0.0"), 999), &plan);
        assert_eq!(value["would_run"], json!([]));
        assert_eq!(value["skipped"], "downgrade");
        assert_eq!(value["notes"], json!(["0014-d: old folders"]));
        assert_eq!(value["open"][0]["command"], "playbook setup --statusline");
    }

    #[test]
    fn status_shows_each_migration_with_its_scope_and_state() {
        // Arrange
        let plan = sample();

        // Act
        let text = render_status(&record(None, 0), &plan);
        let value = status_json(&record(None, 0), &plan);

        // Assert
        assert!(text.contains("recorded: nothing yet"), "{text}");
        assert!(
            text.contains("0001-a  idempotent  owned    runs on every run"),
            "{text}"
        );
        assert!(
            text.contains("0002-b  idempotent  owned    applied"),
            "{text}"
        );
        assert_eq!(value["migrations"][0]["state"], "every_run");
        assert_eq!(value["migrations"][2]["state"], "open");
        assert_eq!(value["running"]["binary"], env!("CARGO_PKG_VERSION"));
    }
}
