// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ordered migration registry run from `playbook init` (ADR 0016). Auto
//! migrations run once and are recorded; Manual ones only warn.

use crate::common::atomic::{
    acquire_dir_lock, ensure_private_dir, remove_stale_lock_dir, write_atomic, STALE_LOCK_AGE,
};
use crate::common::paths::playbook_root_from;
use crate::init::run::{StepReport, StepStatus};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const STATE_FILE: &str = "migrations.state";
const LOCK_DIR: &str = "migrations.lock";
const SYSTEM_PROMPT_KEY: &str = "system-prompt";

/// What a migration needs to find the state it moves.
pub struct Ctx {
    pub home: PathBuf,
    pub claude_home: PathBuf,
    /// `(repo_root, dest_base)` when the caller is inside a repo.
    pub repo: Option<(PathBuf, PathBuf)>,
}

pub enum Kind {
    /// Runs until it succeeds once, then is recorded and never run again.
    Auto,
    /// Never changes anything; only warns.
    Manual,
}

pub enum Outcome {
    /// Succeeded; record the id so it is not run again.
    Done(StepReport),
    /// Succeeded but is idempotent and self-healing; run again next time.
    Repeat(StepReport),
    /// Idempotent and nothing to report; run again next time.
    Quiet,
    /// Manual finding for the user.
    Warn(String),
    /// Did not complete; the id is not recorded and later migrations wait.
    Failed(StepReport),
}

pub struct Migration {
    pub id: &'static str,
    pub kind: Kind,
    pub run: fn(&Ctx) -> Outcome,
}

/// What a run produced, for the caller to print.
#[derive(Default)]
pub struct Report {
    pub steps: Vec<StepReport>,
    pub warnings: Vec<String>,
}

/// Shipped migrations in apply order. Append only; never reorder or reuse an id.
pub fn registry() -> Vec<Migration> {
    vec![
        Migration {
            id: "0001-memory-store-move",
            kind: Kind::Auto,
            run: memory_store_move,
        },
        Migration {
            id: "0002-gate-repo-local-move",
            kind: Kind::Auto,
            run: gate_repo_local_move,
        },
        Migration {
            id: "0003-system-prompt-edited",
            kind: Kind::Manual,
            run: system_prompt_edited,
        },
    ]
}

fn memory_store_move(ctx: &Ctx) -> Outcome {
    let step = crate::init::memory_migrate::migrate_memory(&ctx.home, &ctx.claude_home);
    if step.status == StepStatus::Failed {
        Outcome::Failed(step)
    } else {
        Outcome::Repeat(step)
    }
}

fn gate_repo_local_move(ctx: &Ctx) -> Outcome {
    let Some((repo_root, dest_base)) = &ctx.repo else {
        return Outcome::Quiet;
    };
    match crate::gate::db::migrate_legacy_repo_local(repo_root, dest_base) {
        Ok(()) => Outcome::Quiet,
        Err(err) => Outcome::Failed(StepReport::failed("gate-repo-local", err)),
    }
}

fn system_prompt_path(home: &Path) -> PathBuf {
    playbook_root_from(home)
        .join("prompts")
        .join("SYSTEM_PROMPT.md")
}

fn system_prompt_edited(ctx: &Ctx) -> Outcome {
    let path = system_prompt_path(&ctx.home);
    if user_edited(&ctx.home, SYSTEM_PROMPT_KEY, &path) {
        Outcome::Warn(format!(
            "{} was edited after playbook installed it; init replaces it with the shipped copy, so re-apply your edits afterwards",
            path.display()
        ))
    } else {
        Outcome::Quiet
    }
}

/// FNV-1a 64: stable across builds, which a persisted hash needs.
fn content_hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

fn state_path(home: &Path) -> PathBuf {
    playbook_root_from(home).join(STATE_FILE)
}

fn read_state(home: &Path) -> Vec<String> {
    fs::read_to_string(state_path(home))
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

fn write_state(home: &Path, lines: &[String]) -> std::io::Result<()> {
    let mut body = lines.join("\n");
    body.push('\n');
    write_atomic(&state_path(home), &body)
}

/// Whether `path` differs from the hash recorded when playbook placed it.
/// No record means nothing was placed by us, so never "edited".
pub fn user_edited(home: &Path, key: &str, path: &Path) -> bool {
    let prefix = format!("shipped {key} ");
    let Some(recorded) = read_state(home)
        .iter()
        .find_map(|l| l.strip_prefix(&prefix).map(str::to_string))
    else {
        return false;
    };
    fs::read(path).is_ok_and(|bytes| content_hash(&bytes) != recorded)
}

/// Records the hash of the file playbook just placed at `path`.
pub fn record_shipped(home: &Path, key: &str, path: &Path) {
    let Ok(bytes) = fs::read(path) else { return };
    let prefix = format!("shipped {key} ");
    with_lock(home, || {
        let mut lines = read_state(home);
        lines.retain(|l| !l.starts_with(&prefix));
        lines.push(format!("{prefix}{}", content_hash(&bytes)));
        let _ = write_state(home, &lines);
    });
}

/// Records the system prompt placed by init so later edits are detectable.
pub fn record_system_prompt(home: &Path) {
    record_shipped(home, SYSTEM_PROMPT_KEY, &system_prompt_path(home));
}

fn with_lock(home: &Path, f: impl FnOnce()) -> bool {
    let root = playbook_root_from(home);
    if ensure_private_dir(&root).is_err() {
        return false;
    }
    let lock = root.join(LOCK_DIR);
    remove_stale_lock_dir(&lock, STALE_LOCK_AGE);
    if !acquire_dir_lock(&lock, 50, Duration::from_millis(100)) {
        return false;
    }
    f();
    let _ = fs::remove_dir(&lock);
    true
}

/// Runs the shipped registry.
pub fn run_pending(ctx: &Ctx) -> Report {
    run_with(&registry(), ctx)
}

/// Runs `migrations` in order: Manual ones always, Auto ones once each.
pub fn run_with(migrations: &[Migration], ctx: &Ctx) -> Report {
    let mut report = Report::default();
    for m in migrations.iter().filter(|m| matches!(m.kind, Kind::Manual)) {
        if let Outcome::Warn(msg) = (m.run)(ctx) {
            report.warnings.push(format!("{}: {msg}", m.id));
        }
    }
    let applied = |lines: &[String]| -> Vec<String> {
        lines
            .iter()
            .filter_map(|l| l.strip_prefix("applied ").map(str::to_string))
            .collect()
    };
    let done = applied(&read_state(&ctx.home));
    if migrations
        .iter()
        .all(|m| matches!(m.kind, Kind::Manual) || done.iter().any(|d| d == m.id))
    {
        return report;
    }
    let locked = with_lock(&ctx.home, || {
        for m in migrations.iter().filter(|m| matches!(m.kind, Kind::Auto)) {
            let mut lines = read_state(&ctx.home);
            if applied(&lines).iter().any(|d| d == m.id) {
                continue;
            }
            match (m.run)(ctx) {
                Outcome::Done(step) => {
                    lines.push(format!("applied {}", m.id));
                    if let Err(err) = write_state(&ctx.home, &lines) {
                        report
                            .warnings
                            .push(format!("{}: could not record: {err}", m.id));
                        break;
                    }
                    report.steps.push(step);
                }
                Outcome::Repeat(step) => report.steps.push(step),
                Outcome::Quiet => {}
                Outcome::Warn(msg) => report.warnings.push(format!("{}: {msg}", m.id)),
                Outcome::Failed(step) => {
                    report.steps.push(step);
                    break;
                }
            }
        }
    });
    if !locked {
        report
            .warnings
            .push("migrations skipped: could not take the migration lock".to_string());
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    static CALLS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    static SLOW_RUNS: AtomicUsize = AtomicUsize::new(0);

    fn home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pb-migrate-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ctx(home: &Path) -> Ctx {
        Ctx {
            home: home.to_path_buf(),
            claude_home: home.join(".claude"),
            repo: None,
        }
    }

    fn done(name: &'static str) -> Outcome {
        Outcome::Done(StepReport::wired(name, "ok"))
    }

    fn auto(id: &'static str, run: fn(&Ctx) -> Outcome) -> Migration {
        Migration {
            id,
            kind: Kind::Auto,
            run,
        }
    }

    fn order_a(_: &Ctx) -> Outcome {
        CALLS.lock().unwrap().push("a");
        done("a")
    }
    fn order_b(_: &Ctx) -> Outcome {
        CALLS.lock().unwrap().push("b");
        done("b")
    }

    #[test]
    fn runs_in_registry_order_then_never_again() {
        let h = home("order");
        CALLS.lock().unwrap().clear();
        let ms = [auto("a", order_a), auto("b", order_b)];
        let first = run_with(&ms, &ctx(&h));
        let second = run_with(&ms, &ctx(&h));
        assert_eq!(*CALLS.lock().unwrap(), vec!["a", "b"]);
        assert_eq!(first.steps.len(), 2);
        assert!(second.steps.is_empty());
    }

    fn fails(_: &Ctx) -> Outcome {
        Outcome::Failed(StepReport::failed("boom", "nope"))
    }
    fn never(_: &Ctx) -> Outcome {
        panic!("must not run after a failure");
    }

    #[test]
    fn a_failure_is_not_recorded_and_stops_later_migrations() {
        let h = home("fail");
        let ms = [auto("f", fails), auto("n", never)];
        let report = run_with(&ms, &ctx(&h));
        assert_eq!(report.steps.len(), 1);
        assert_eq!(report.steps[0].status, StepStatus::Failed);
        assert!(!read_state(&h).iter().any(|l| l == "applied f"));
    }

    fn slow(_: &Ctx) -> Outcome {
        SLOW_RUNS.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(150));
        done("slow")
    }

    #[test]
    fn concurrent_runners_apply_a_migration_once() {
        let h = Arc::new(home("lock"));
        SLOW_RUNS.store(0, Ordering::SeqCst);
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let h = Arc::clone(&h);
                std::thread::spawn(move || {
                    run_with(&[auto("slow", slow)], &ctx(&h));
                })
            })
            .collect();
        for t in handles {
            t.join().unwrap();
        }
        assert_eq!(SLOW_RUNS.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn warns_for_an_edited_file_and_not_for_an_untouched_one() {
        let h = home("edit");
        let prompt = system_prompt_path(&h);
        fs::create_dir_all(prompt.parent().unwrap()).unwrap();
        fs::write(&prompt, "shipped text").unwrap();
        record_system_prompt(&h);
        let clean = run_with(&registry(), &ctx(&h));
        assert!(clean.warnings.is_empty());

        fs::write(&prompt, "my edits").unwrap();
        let dirty = run_with(&registry(), &ctx(&h));
        assert_eq!(dirty.warnings.len(), 1);
        assert!(dirty.warnings[0].contains("edited"));
    }

    #[test]
    fn an_unrecorded_file_is_never_reported_as_edited() {
        let h = home("norecord");
        let prompt = system_prompt_path(&h);
        fs::create_dir_all(prompt.parent().unwrap()).unwrap();
        fs::write(&prompt, "anything").unwrap();
        assert!(!user_edited(&h, SYSTEM_PROMPT_KEY, &prompt));
    }
}
