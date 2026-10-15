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
/// exit code: 1 when a migration failed, the lock was busy or the record was
/// not written, else 0.
pub fn run(home: &Path, mode: Mode, json: bool) -> i32 {
    let ctx = Ctx::for_home(home);
    match mode {
        Mode::Run => {
            let out = heal::run(&ctx, Trigger::Fix, crate::common::time::now_epoch_secs());
            if json {
                println!("{}", run_json(&out));
            } else {
                print!("{}", heal::render_fix(&out));
            }
            out.exit_code()
        }
        Mode::Status | Mode::DryRun => {
            let root = crate::common::paths::playbook_root_from(home);
            let record = match Record::read_only(&root) {
                Ok(record) => record,
                Err(err) => {
                    eprintln!("playbook migrate: the upgrade record could not be read: {err}");
                    return 1;
                }
            };
            let ready = crate::config::store::db_path(&root).is_file()
                && crate::state::pending_legacy(&root) == 0;
            let plan = migrate::plan(&ctx, ready);
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

/// How the record compares with this binary: `Less` when a heal is due.
fn due(record: &Record) -> Ordering {
    match &record.binary {
        None => Ordering::Less,
        Some(b) => crate::update::resolve::compare(b, env!("CARGO_PKG_VERSION"))
            .then(record.schema.cmp(&migrate::schema())),
    }
}

fn recorded_line(record: &Record) -> String {
    match &record.binary {
        Some(b) => format!("recorded: playbook {b}, migrations up to {}", record.schema),
        None => "recorded: nothing yet".to_string(),
    }
}

fn state_text(state: &State, heal_due: bool) -> String {
    match state {
        State::Applied => "applied".into(),
        State::Pending => "pending".into(),
        State::EachHeal if heal_due => "pending".into(),
        State::EachHeal => "applied".into(),
        State::Clear => "nothing to do".into(),
        State::Open {
            message,
            command: Some(c),
        } => format!("needs you: {message} (run `{c}`)"),
        State::Open {
            message,
            command: None,
        } => format!("needs you: {message}"),
        State::Unknown => "not checked: no state yet".into(),
    }
}

fn render_status(record: &Record, plan: &[Planned]) -> String {
    let heal_due = due(record) == Ordering::Less;
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
            state_text(&p.state, heal_due)
        ));
    }
    text
}

/// The `Owned` migrations a heal would run now, in order.
fn would_run(record: &Record, plan: &[Planned]) -> Vec<&'static str> {
    if due(record) != Ordering::Less {
        return Vec::new();
    }
    plan.iter()
        .filter(|p| p.scope == "owned" && p.kind != "manual")
        .map(|p| p.id)
        .collect()
}

fn render_dry_run(record: &Record, plan: &[Planned]) -> String {
    let mut text = String::new();
    let run = would_run(record, plan);
    match due(record) {
        Ordering::Greater => text.push_str(
            "nothing would run: a newer playbook ran here; nothing runs after a downgrade\n",
        ),
        Ordering::Equal => text.push_str(&format!(
            "nothing would run: the record names playbook {}\n",
            env!("CARGO_PKG_VERSION")
        )),
        Ordering::Less => text.push_str(&format!("would run: {}\n", run.join(", "))),
    }
    for p in plan {
        if let State::Open { message, command } = &p.state {
            match command {
                Some(c) => text.push_str(&format!("needs you: {message} (run `{c}`)\n")),
                None if p.scope == "owned" => {
                    text.push_str(&format!("note: {}: {message}\n", p.id))
                }
                None => text.push_str(&format!("needs you: {message}\n")),
            }
        }
    }
    text.push_str("dry run: nothing was written\n");
    text
}

fn planned_json(p: &Planned, heal_due: bool) -> Value {
    let (message, command) = match &p.state {
        State::Open { message, command } => (Some(message.clone()), *command),
        _ => (None, None),
    };
    json!({
        "id": p.id,
        "kind": p.kind,
        "scope": p.scope,
        "state": match &p.state {
            State::Open { .. } => "open".to_string(),
            other => state_text(other, heal_due),
        },
        "message": message,
        "command": command,
    })
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
    let heal_due = due(record) == Ordering::Less;
    json!({
        "running": {"binary": env!("CARGO_PKG_VERSION"), "schema": migrate::schema()},
        "recorded": record_json(record),
        "migrations": plan.iter().map(|p| planned_json(p, heal_due)).collect::<Vec<_>>(),
    })
}

fn dry_run_json(record: &Record, plan: &[Planned]) -> Value {
    json!({
        "would_run": would_run(record, plan),
        "open": plan
            .iter()
            .filter(|p| matches!(p.state, State::Open { .. }))
            .map(|p| planned_json(p, false))
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
        "skipped": out.skipped.map(heal::Skip::reason),
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

    fn planned(id: &'static str, kind: &'static str, scope: &'static str, state: State) -> Planned {
        Planned {
            id,
            kind,
            scope,
            state,
        }
    }

    #[test]
    fn a_dry_run_names_the_owned_migrations_only_when_a_heal_is_due() {
        let plan = vec![
            planned("0001-a", "idempotent", "owned", State::EachHeal),
            planned("0003-b", "manual", "owned", State::Clear),
            planned(
                "0006-c",
                "idempotent",
                "consent",
                State::Open {
                    message: "old command".into(),
                    command: Some("playbook init --statusline"),
                },
            ),
        ];
        let old = render_dry_run(&record(Some("0.25.0"), 1), &plan);
        assert!(old.starts_with("would run: 0001-a\n"), "{old}");
        assert!(old.contains("needs you: old command (run `playbook init --statusline`)"));
        assert!(old.ends_with("dry run: nothing was written\n"));
        let same = render_dry_run(
            &record(Some(env!("CARGO_PKG_VERSION")), migrate::schema()),
            &plan,
        );
        assert!(
            same.starts_with("nothing would run: the record names"),
            "{same}"
        );
        let newer = render_dry_run(&record(Some("999.0.0"), 999), &plan);
        assert!(newer.contains("nothing runs after a downgrade"), "{newer}");
    }

    #[test]
    fn status_shows_each_migration_with_its_scope_and_state() {
        let plan = vec![
            planned("0001-a", "idempotent", "owned", State::EachHeal),
            planned("0012-b", "manual", "report", State::Unknown),
        ];
        let text = render_status(&record(None, 0), &plan);
        assert!(text.contains("recorded: nothing yet"), "{text}");
        assert!(
            text.contains("0001-a  idempotent  owned    pending"),
            "{text}"
        );
        assert!(text.contains("not checked: no state yet"), "{text}");
        let value = status_json(&record(None, 0), &plan);
        assert_eq!(value["migrations"][0]["state"], "pending");
        assert_eq!(value["running"]["binary"], env!("CARGO_PKG_VERSION"));
    }
}
