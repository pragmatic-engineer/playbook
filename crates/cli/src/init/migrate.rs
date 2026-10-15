// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Ordered migration registry (ADR 0016, ADR 0034). Auto migrations run once
//! and are recorded; Manual ones only warn. `init` runs every entry; the
//! session hook, the first-run step, `migrate` and `doctor --fix` run only
//! the `Owned` ones (`run_owned`).

use crate::common::atomic::{acquire_dir_lock, ensure_private_dir, remove_stale_lock_dir};
use crate::common::paths::playbook_root_from;
use crate::init::run::{StepReport, StepStatus};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LOCK_DIR: &str = "migrations.lock";
const SYSTEM_PROMPT_KEY: &str = "system-prompt";
const STATUSLINE_KEY: &str = "statusline";
const SKILL_KEY_PREFIX: &str = "skill:";
/// The `Consent` migration `init` skips when the user declines the status line.
pub const STATUSLINE_MIGRATION: &str = "0006-statusline-rust-command";
/// The `Consent` migration `init` skips when the user declines the hooks.
pub const DEAD_HOOKS_MIGRATION: &str = "0011-dead-hook-scripts";

/// What a migration needs to find the state it moves.
pub struct Ctx {
    pub home: PathBuf,
    pub claude_home: PathBuf,
    /// The shipped plugin tree, when init could resolve it.
    pub self_root: Option<PathBuf>,
    /// `(repo_root, dest_base)` when the caller is inside a repo.
    pub repo: Option<(PathBuf, PathBuf)>,
    /// The stamp for any backup a migration takes. `init` passes its run's
    /// one stamp, so all of a run's backups share it.
    pub epoch: u64,
}

impl Ctx {
    /// A context for `home` alone: no plugin tree and no repo, as the session
    /// hook and `doctor --fix` run.
    pub fn for_home(home: &Path) -> Ctx {
        Ctx {
            home: home.to_path_buf(),
            claude_home: home.join(".claude"),
            self_root: None,
            repo: None,
            epoch: crate::common::time::now_epoch_secs(),
        }
    }
}

/// How many times a waiting caller tries the migration lock, 100 ms apart.
pub const LOCK_RETRIES: u32 = 50;

pub enum Kind {
    /// Runs until it succeeds once, then is recorded and never run again.
    Auto,
    /// Self-healing and idempotent: runs every init, never recorded, no lock.
    Idempotent,
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

/// What a migration writes, next to `Kind`, which says how often it runs.
#[derive(Clone, Copy)]
pub enum Scope {
    /// Writes only playbook's own files, or nothing. Safe to run unattended.
    Owned,
    /// Writes a file the user owns, so only `init` runs it. `check` says,
    /// without writing, what the run would change, or `None`. `command` is
    /// the init command that applies it.
    Consent {
        check: fn(&Ctx) -> Option<String>,
        command: &'static str,
    },
    /// Reports on files Claude Code owns and writes nothing. A `Manual`
    /// migration whose warning the session hook repeats until it clears. A
    /// note that should not repeat each session stays `Owned`, as 0014 does.
    Report,
}

pub struct Migration {
    pub id: &'static str,
    pub kind: Kind,
    pub scope: Scope,
    pub run: fn(&Ctx) -> Outcome,
}

/// What a run produced, for the caller to print.
#[derive(Default)]
pub struct Report {
    pub steps: Vec<StepReport>,
    pub warnings: Vec<String>,
    /// A migration failed, or the applied record could not be read or written.
    pub failed: bool,
    /// Another process held the migration lock, so the locked part did not run.
    pub lock_busy: bool,
    /// The deadline passed, so the later migrations did not start.
    pub timed_out: bool,
}

/// Shipped migrations in apply order. Append only; never reorder or reuse an id.
pub fn registry() -> Vec<Migration> {
    vec![
        Migration {
            id: "0001-memory-store-move",
            kind: Kind::Idempotent,
            scope: Scope::Owned,
            run: memory_store_move,
        },
        Migration {
            id: "0002-gate-repo-local-move",
            kind: Kind::Idempotent,
            scope: Scope::Owned,
            run: gate_repo_local_move,
        },
        Migration {
            id: "0003-system-prompt-edited",
            kind: Kind::Manual,
            scope: Scope::Owned,
            run: system_prompt_edited,
        },
        Migration {
            id: "0004-skills-edited",
            kind: Kind::Manual,
            scope: Scope::Owned,
            run: skills_edited,
        },
        Migration {
            id: "0005-statusline-edited",
            kind: Kind::Manual,
            scope: Scope::Owned,
            run: statusline_edited,
        },
        Migration {
            id: STATUSLINE_MIGRATION,
            kind: Kind::Idempotent,
            scope: Scope::Consent {
                check: statusline_rust_command_pending,
                command: "playbook setup --statusline",
            },
            run: statusline_rust_command,
        },
        Migration {
            id: "0007-shell-init-rc-line",
            kind: Kind::Idempotent,
            scope: Scope::Consent {
                check: shell_init_rc_line_pending,
                command: "playbook setup",
            },
            run: shell_init_rc_line,
        },
        Migration {
            id: "0008-remove-config-hash-script",
            kind: Kind::Idempotent,
            scope: Scope::Owned,
            run: remove_config_hash_script,
        },
        Migration {
            id: "0009-adopt-late-config-json",
            kind: Kind::Idempotent,
            scope: Scope::Owned,
            run: adopt_late_config_json,
        },
        Migration {
            id: "0010-state-files-to-sqlite",
            kind: Kind::Idempotent,
            scope: Scope::Owned,
            run: state_files_to_sqlite,
        },
        Migration {
            id: DEAD_HOOKS_MIGRATION,
            kind: Kind::Idempotent,
            scope: Scope::Consent {
                check: dead_hook_scripts_pending,
                command: "playbook setup --hooks",
            },
            run: dead_hook_scripts,
        },
        Migration {
            id: "0012-other-playbook-plugins",
            kind: Kind::Manual,
            scope: Scope::Report,
            run: other_playbook_plugins,
        },
        Migration {
            id: "0013-plugin-version-range",
            kind: Kind::Manual,
            scope: Scope::Report,
            run: plugin_version_range,
        },
        Migration {
            id: "0014-orphaned-plugin-caches",
            kind: Kind::Manual,
            scope: Scope::Owned,
            run: orphaned_plugin_caches,
        },
    ]
}

/// Hook entries whose `~/.claude/hooks/` script is gone (ADR 0034 decision
/// 9). Removing them writes settings.json, so only `init` does it.
fn dead_hook_scripts(ctx: &Ctx) -> Outcome {
    let settings = ctx.claude_home.join("settings.json");
    match crate::init::dead_hooks::remove(&settings, &ctx.home, &ctx.claude_home, ctx.epoch) {
        Ok(removed) if removed.is_empty() => Outcome::Quiet,
        Ok(removed) => Outcome::Repeat(StepReport::wired(
            "dead-hooks",
            format!(
                "removed {} hook(s) whose script is gone: {}",
                removed.len(),
                removed.join(", ")
            ),
        )),
        Err(err) => Outcome::Failed(StepReport::failed("dead-hooks", err.to_string())),
    }
}

fn dead_hook_scripts_pending(ctx: &Ctx) -> Option<String> {
    crate::init::dead_hooks::pending(
        &ctx.claude_home.join("settings.json"),
        &ctx.home,
        &ctx.claude_home,
    )
}

/// The retired `playbook-hooks` plugin, or a `playbook` plugin from another
/// marketplace, runs next to this one. Claude Code owns the plugin list.
fn other_playbook_plugins(ctx: &Ctx) -> Outcome {
    let cwd = std::env::current_dir().unwrap_or_default();
    let keys = crate::common::plugin_state::other_playbook_plugins(&ctx.claude_home, &cwd);
    if keys.is_empty() {
        return Outcome::Quiet;
    }
    let commands: Vec<String> = keys
        .iter()
        .map(|k| format!("claude plugin uninstall {k}"))
        .collect();
    Outcome::Warn(format!(
        "installed next to {}, so hooks or skills run twice: {}. Run: {}",
        crate::common::plugin_state::PLUGIN_KEY,
        keys.join(", "),
        commands.join("; ")
    ))
}

/// The installed plugin is not in the minor line of this binary (ADR 0038
/// decision 7). Reads what Claude Code installed under `ctx.claude_home`,
/// not the process environment.
fn plugin_version_range(ctx: &Ctx) -> Outcome {
    let exe = std::env::current_exe().unwrap_or_default();
    let homebrew = crate::update::is_homebrew(&std::fs::canonicalize(&exe).unwrap_or(exe));
    crate::plugin_root::find(None, &ctx.claude_home)
        .and_then(|root| crate::plugin_root::plugin_version(&root))
        .and_then(|v| {
            crate::plugin_root::plugin_out_of_range(&v, env!("CARGO_PKG_VERSION"), homebrew)
        })
        .map_or(Outcome::Quiet, Outcome::Warn)
}

/// Old plugin folders Claude Code marked for removal. Playbook ignores them
/// and never deletes them. Scoped `Owned` so the note shows in `init` and
/// `doctor --fix` but the session hook does not repeat it.
fn orphaned_plugin_caches(ctx: &Ctx) -> Outcome {
    let dirs = crate::plugin_root::orphaned_cache_dirs(&ctx.claude_home);
    if dirs.is_empty() {
        return Outcome::Quiet;
    }
    Outcome::Warn(format!(
        "{} old plugin folder(s) are marked orphaned; playbook ignores them and Claude Code removes them: {}",
        dirs.len(),
        join_paths(&dirs)
    ))
}

/// A `config.json` written after the SQLite import (an older binary, or old
/// docs) would be ignored without a word. Adopt it when it agrees with the
/// store, and warn with its path when it does not. Runs on every init.
fn adopt_late_config_json(ctx: &Ctx) -> Outcome {
    let root = crate::common::paths::playbook_root_from(&ctx.home);
    match crate::config::store::adopt_late_files(&root) {
        Ok(report) if !report.conflicts.is_empty() => Outcome::Warn(
            crate::config::adopt_late_config(&ctx.home)
                .first()
                .cloned()
                .unwrap_or_default(),
        ),
        Ok(report) if !report.imported.is_empty() => Outcome::Repeat(StepReport::wired(
            "config",
            format!(
                "adopted {} late config.json file(s) into the SQLite store",
                report.imported.len()
            ),
        )),
        _ => Outcome::Quiet,
    }
}

/// The migration record and the worktree sweep markers live in the `state`
/// table now. Opening the store imports the old files, so this only makes sure
/// that happened and fails loudly when the store cannot be opened. The import
/// itself is reported by `run_pending`, which sees the files before any
/// earlier migration opens the store.
fn state_files_to_sqlite(ctx: &Ctx) -> Outcome {
    let root = crate::common::paths::playbook_root_from(&ctx.home);
    match crate::state::list(&root, "") {
        Ok(_) => Outcome::Quiet,
        Err(err) => Outcome::Failed(StepReport::failed("state", err.to_string())),
    }
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

/// The config hash is computed in Rust now. Remove the shell copy that older
/// installs placed under the playbook root, and its directories when they end
/// up empty.
fn remove_config_hash_script(ctx: &Ctx) -> Outcome {
    let root = crate::common::paths::playbook_root_from(&ctx.home);
    let script = root.join("hooks/lib/config-hash.sh");
    if !script.is_file() {
        return Outcome::Quiet;
    }
    if let Err(err) = std::fs::remove_file(&script) {
        return Outcome::Failed(StepReport::failed("config-hash", err.to_string()));
    }
    for dir in [root.join("hooks/lib"), root.join("hooks")] {
        let _ = std::fs::remove_dir(&dir);
    }
    Outcome::Repeat(StepReport::wired(
        "config-hash",
        "removed the placed config-hash.sh, the hash now runs inside playbook",
    ))
}

fn shell_init_rc_line(ctx: &Ctx) -> Outcome {
    match crate::init::shim::upgrade_legacy_rc_files(&ctx.home) {
        Ok(files) if files.is_empty() => Outcome::Quiet,
        Ok(files) => Outcome::Repeat(StepReport::wired(
            "shell-init",
            format!(
                "{} now loads the launcher with `playbook shell-init`",
                join_paths(&files)
            ),
        )),
        Err(err) => Outcome::Failed(StepReport::failed("shell-init", err.to_string())),
    }
}

fn statusline_rust_command_pending(ctx: &Ctx) -> Option<String> {
    let script = crate::init::statusline::playbook_statusline_path(&ctx.home);
    let edited = user_edited(&ctx.home, STATUSLINE_KEY, &script);
    crate::init::statusline::legacy_command_pending(
        &ctx.claude_home.join("settings.json"),
        &ctx.home,
        edited,
    )
    .then(|| "statusLine.command in settings.json still runs the old script".to_string())
}

fn shell_init_rc_line_pending(ctx: &Ctx) -> Option<String> {
    let files = crate::init::shim::legacy_rc_files(&ctx.home);
    (!files.is_empty()).then(|| {
        format!(
            "{} still loads the launcher the old way",
            join_paths(&files)
        )
    })
}

fn join_paths(files: &[PathBuf]) -> String {
    files
        .iter()
        .map(|f| f.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn statusline_rust_command(ctx: &Ctx) -> Outcome {
    let settings = ctx.claude_home.join("settings.json");
    let script = crate::init::statusline::playbook_statusline_path(&ctx.home);
    let edited = user_edited(&ctx.home, STATUSLINE_KEY, &script);
    match crate::init::statusline::migrate_legacy_command(&settings, &ctx.home, edited) {
        Ok(true) => Outcome::Repeat(StepReport::wired(
            "statusline-command",
            "statusLine.command now runs `playbook statusline`",
        )),
        Ok(false) => Outcome::Quiet,
        Err(err) => Outcome::Failed(StepReport::failed("statusline-command", err.to_string())),
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
            "{} was edited after playbook installed it, so setup leaves it in place; to take the shipped copy, delete it and run `playbook setup --system-prompt`",
            path.display()
        ))
    } else {
        Outcome::Quiet
    }
}

fn statusline_edited(ctx: &Ctx) -> Outcome {
    let path = crate::init::statusline::playbook_statusline_path(&ctx.home);
    if user_edited(&ctx.home, STATUSLINE_KEY, &path) {
        Outcome::Warn(format!(
            "{} was edited after playbook installed it, so setup leaves it in place; setup no longer ships this script, so delete it once nothing runs it",
            path.display()
        ))
    } else {
        Outcome::Quiet
    }
}

/// `(key, path)` for every shipped skill file, keyed by plugin version dir.
fn skill_files(skills_root: &Path) -> Vec<(String, PathBuf)> {
    // A dev checkout is edited on purpose; only cached plugin copies are tracked.
    if skills_root.join(".git").exists() {
        return Vec::new();
    }
    let version = skills_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Ok(entries) = fs::read_dir(skills_root.join("skills")) else {
        return Vec::new();
    };
    let mut files: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path().join("SKILL.md");
            let name = e.file_name().to_string_lossy().into_owned();
            path.is_file()
                .then(|| (format!("{SKILL_KEY_PREFIX}{version}:{name}"), path))
        })
        .collect();
    files.sort();
    files
}

fn skills_edited(ctx: &Ctx) -> Outcome {
    let Some(self_root) = &ctx.self_root else {
        return Outcome::Quiet;
    };
    let lines = read_state(&ctx.home);
    let edited: Vec<String> = skill_files(self_root)
        .into_iter()
        .filter(|(key, path)| edited_in(&lines, key, path))
        .map(|(key, _)| key.rsplit(':').next().unwrap_or_default().to_string())
        .collect();
    if edited.is_empty() {
        Outcome::Quiet
    } else {
        Outcome::Warn(format!(
            "skills edited after install: {}; a plugin update replaces them, so move your edits into your own skill",
            edited.join(", ")
        ))
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

/// The migration record as the `applied <id>` and `shipped <key> <hash>` lines
/// it has always had, now read from the `state` table. `None` when the store
/// cannot be read, so a write never wipes a record it failed to load.
fn load_state(home: &Path) -> Option<Vec<String>> {
    if let Some(lines) = SNAPSHOT.with(|s| s.borrow().clone()) {
        return Some(lines);
    }
    let root = playbook_root_from(home);
    let applied = crate::state::list(&root, crate::state::MIGRATIONS_APPLIED).ok()?;
    let shipped = crate::state::list(&root, crate::state::MIGRATIONS_SHIPPED).ok()?;
    Some(record_lines(applied, shipped))
}

thread_local! {
    /// The record `plan` read once without writing. While it is set, every
    /// read of the record on this thread returns it and opens no store.
    static SNAPSHOT: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// The record read through read-only connections, which create, import and
/// change nothing. `None` when the store cannot be read.
fn load_state_read_only(home: &Path) -> Option<Vec<String>> {
    let root = playbook_root_from(home);
    let applied = crate::state::list_read_only(&root, crate::state::MIGRATIONS_APPLIED).ok()?;
    let shipped = crate::state::list_read_only(&root, crate::state::MIGRATIONS_SHIPPED).ok()?;
    Some(record_lines(applied, shipped))
}

fn record_lines(applied: Vec<(String, String)>, shipped: Vec<(String, String)>) -> Vec<String> {
    let mut lines: Vec<String> = applied
        .into_iter()
        .map(|(k, _)| format!("applied {}", &k[crate::state::MIGRATIONS_APPLIED.len()..]))
        .collect();
    lines.extend(shipped.into_iter().map(|(k, v)| {
        format!(
            "shipped {} {v}",
            &k[crate::state::MIGRATIONS_SHIPPED.len()..]
        )
    }));
    lines
}

fn read_state(home: &Path) -> Vec<String> {
    load_state(home).unwrap_or_default()
}

/// Replaces the whole record with `lines`, in one transaction.
fn write_state(home: &Path, lines: &[String]) -> std::io::Result<()> {
    use crate::state::{MIGRATIONS_APPLIED, MIGRATIONS_SHIPPED};
    let root = playbook_root_from(home);
    crate::state::transaction(&root, |tx| {
        tx.delete_prefix("migrations/")?;
        for line in lines {
            if let Some(id) = line.strip_prefix("applied ") {
                tx.set(&format!("{MIGRATIONS_APPLIED}{id}"), "1")?;
            } else if let Some((key, hash)) = line
                .strip_prefix("shipped ")
                .and_then(|rest| rest.rsplit_once(' '))
            {
                tx.set(&format!("{MIGRATIONS_SHIPPED}{key}"), hash)?;
            }
        }
        Ok(())
    })
    .map_err(|e| std::io::Error::other(e.to_string()))
}

/// Whether `path` differs from the hash recorded when playbook placed it.
/// No record means nothing was placed by us, so never "edited".
pub fn user_edited(home: &Path, key: &str, path: &Path) -> bool {
    edited_in(&read_state(home), key, path)
}

/// Whether a hash was recorded for `key` and `path` still matches it: the
/// only proof that the file at `path` is the one playbook placed, untouched.
pub fn placed_unchanged(home: &Path, key: &str, path: &Path) -> bool {
    let prefix = format!("shipped {key} ");
    read_state(home)
        .iter()
        .find_map(|l| l.strip_prefix(&prefix).map(str::to_string))
        .zip(fs::read(path).ok())
        .is_some_and(|(recorded, bytes)| content_hash(&bytes) == recorded)
}

fn edited_in(lines: &[String], key: &str, path: &Path) -> bool {
    let prefix = format!("shipped {key} ");
    let Some(recorded) = lines
        .iter()
        .find_map(|l| l.strip_prefix(&prefix).map(str::to_string))
    else {
        return false;
    };
    fs::read(path).is_ok_and(|bytes| content_hash(&bytes) != recorded)
}

/// Records the hash of the file playbook just placed at `path`.
pub fn record_shipped(home: &Path, key: &str, path: &Path) {
    record_many(home, &[(key.to_string(), path.to_path_buf())], None);
}

/// Records several files in one locked write; `drop` clears matching stale keys first.
fn record_many(home: &Path, files: &[(String, PathBuf)], drop: Option<&dyn Fn(&str) -> bool>) {
    let hashed: Vec<(String, String)> = files
        .iter()
        .filter_map(|(key, path)| {
            let bytes = fs::read(path).ok()?;
            Some((format!("shipped {key} "), content_hash(&bytes)))
        })
        .collect();
    with_lock(home, || {
        let Some(mut lines) = load_state(home) else {
            return;
        };
        let before = lines.clone();
        if let Some(drop) = drop {
            lines.retain(|l| !l.strip_prefix("shipped ").is_some_and(drop));
        }
        for (prefix, hash) in &hashed {
            lines.retain(|l| !l.starts_with(prefix));
            lines.push(format!("{prefix}{hash}"));
        }
        let mut before_sorted = before;
        before_sorted.sort();
        lines.sort();
        if lines != before_sorted {
            let _ = write_state(home, &lines);
        }
    });
}

/// Records shipped skill files, never an edited one, and drops other versions' records.
pub fn record_skills(home: &Path, skills_root: &Path) {
    let files = skill_files(skills_root);
    let Some(version_prefix) = files
        .first()
        .and_then(|(key, _)| key.rsplit_once(':'))
        .map(|(v, _)| format!("{v}:"))
    else {
        return;
    };
    // An edited skill keeps its old record so it stays reported until resolved.
    let lines = read_state(home);
    let fresh: Vec<_> = files
        .into_iter()
        .filter(|(key, path)| !edited_in(&lines, key, path))
        .collect();
    let stale = |key: &str| key.starts_with(SKILL_KEY_PREFIX) && !key.starts_with(&version_prefix);
    record_many(home, &fresh, Some(&stale));
}

/// One item a run left for the user: a `Consent` migration's pending change or
/// a `Report` warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenItem {
    pub id: &'static str,
    pub message: String,
    /// The init command that applies a `Consent` change. `None` for a report.
    pub command: Option<&'static str>,
}

/// The open `Consent` and `Report` items, writing nothing. `only` limits the
/// checks to those ids.
pub fn open_items(ctx: &Ctx, only: Option<&[String]>) -> Vec<OpenItem> {
    registry()
        .iter()
        .filter(|m| only.is_none_or(|ids| ids.iter().any(|id| id == m.id)))
        .filter_map(|m| match m.scope {
            Scope::Consent { check, command } => check(ctx).map(|message| OpenItem {
                id: m.id,
                message,
                command: Some(command),
            }),
            Scope::Report => match (m.run)(ctx) {
                Outcome::Warn(message) => Some(OpenItem {
                    id: m.id,
                    message,
                    command: None,
                }),
                _ => None,
            },
            Scope::Owned => None,
        })
        .collect()
}

/// Runs the `Owned` migrations that write, under the migration lock, taken
/// with `retries` tries 100 ms apart. A `Consent` migration never runs here.
/// No migration starts after `deadline`; the report then says it timed out.
pub fn run_owned(ctx: &Ctx, retries: u32, deadline: Option<Instant>) -> Report {
    let owned: Vec<Migration> = registry().into_iter().filter(heal_runs).collect();
    let mut report = Report::default();
    let locked = with_lock_retry(&ctx.home, retries, || {
        run_idempotent(&owned, ctx, &mut report, deadline);
        run_auto(&owned, ctx, &mut report, deadline);
    });
    report.lock_busy = !locked;
    report
}

/// Whether a heal runs `m`: an `Owned` migration that writes.
fn heal_runs(m: &Migration) -> bool {
    matches!(m.scope, Scope::Owned) && !matches!(m.kind, Kind::Manual)
}

/// The notes of the `Manual` migrations scoped `Owned`: files playbook placed
/// and the user may have edited, and orphaned plugin folders. `init` and
/// `doctor --fix` show them; the session hook does not repeat them.
pub fn owned_notes(ctx: &Ctx) -> Vec<String> {
    registry()
        .iter()
        .filter(|m| matches!(m.kind, Kind::Manual) && matches!(m.scope, Scope::Owned))
        .filter_map(|m| match (m.run)(ctx) {
            Outcome::Warn(msg) => Some(format!("{}: {msg}", m.id)),
            _ => None,
        })
        .collect()
}

/// Where a migration stands, read without writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// An `Auto` migration in the applied record.
    Applied,
    /// An `Auto` migration not in the applied record yet.
    Pending,
    /// An `Idempotent` `Owned` migration, which runs on every heal.
    EveryRun,
    /// A check that found nothing to do.
    Clear,
    /// A check that found something for the user, with the command that
    /// applies it when it is a `Consent` change.
    Open {
        message: String,
        command: Option<&'static str>,
    },
    /// Not checked, because the check reads the record and old state files
    /// are still waiting to be imported into it.
    Unknown,
}

impl State {
    /// The stable token for JSON output.
    pub fn code(&self) -> &'static str {
        match self {
            State::Applied => "applied",
            State::Pending => "pending",
            State::EveryRun => "every_run",
            State::Clear => "clear",
            State::Open { .. } => "open",
            State::Unknown => "unknown",
        }
    }
}

/// One registry entry and where it stands.
pub struct Planned {
    pub id: &'static str,
    pub kind: &'static str,
    pub scope: &'static str,
    pub state: State,
    /// Whether a heal would start it now, unless the record is newer.
    pub runs: bool,
}

/// Checks that read the record, which `plan` skips while old state files
/// wait to be imported, because the record is not complete yet.
const READS_STATE: [&str; 4] = [
    "0003-system-prompt-edited",
    "0004-skills-edited",
    "0005-statusline-edited",
    STATUSLINE_MIGRATION,
];

/// Where every migration stands, writing nothing: the record is read once
/// through read-only connections, and every check sees that copy. While old
/// state files wait to be imported, the checks that read the record are
/// skipped. `None` when the record cannot be read.
pub fn plan(ctx: &Ctx) -> Option<Vec<Planned>> {
    let root = playbook_root_from(&ctx.home);
    let state_ready = crate::state::pending_legacy(&root) == 0;
    let lines = load_state_read_only(&ctx.home)?;
    let applied_ids = applied(&lines);
    SNAPSHOT.with(|s| *s.borrow_mut() = Some(lines));
    let plan = std::panic::catch_unwind(|| planned(ctx, state_ready, &applied_ids));
    SNAPSHOT.with(|s| *s.borrow_mut() = None);
    plan.ok()
}

fn planned(ctx: &Ctx, state_ready: bool, applied_ids: &[String]) -> Vec<Planned> {
    registry()
        .iter()
        .map(|m| {
            let needs_state = matches!(m.kind, Kind::Auto) || READS_STATE.contains(&m.id);
            let state = if needs_state && !state_ready {
                State::Unknown
            } else {
                match (&m.kind, m.scope) {
                    (_, Scope::Consent { check, command }) => {
                        check(ctx).map_or(State::Clear, |message| State::Open {
                            message,
                            command: Some(command),
                        })
                    }
                    (Kind::Manual, _) => match (m.run)(ctx) {
                        Outcome::Warn(message) => State::Open {
                            message,
                            command: None,
                        },
                        _ => State::Clear,
                    },
                    (Kind::Auto, _) if applied_ids.iter().any(|d| d == m.id) => State::Applied,
                    (Kind::Auto, _) => State::Pending,
                    (Kind::Idempotent, _) => State::EveryRun,
                }
            };
            Planned {
                runs: heal_runs(m) && state != State::Applied,
                id: m.id,
                kind: match m.kind {
                    Kind::Auto => "auto",
                    Kind::Idempotent => "idempotent",
                    Kind::Manual => "manual",
                },
                scope: match m.scope {
                    Scope::Owned => "owned",
                    Scope::Consent { .. } => "consent",
                    Scope::Report => "report",
                },
                state,
            }
        })
        .collect()
}

/// The highest migration number in the registry, from the id prefix.
pub fn schema() -> u32 {
    registry()
        .iter()
        .filter_map(|m| m.id.split('-').next()?.parse::<u32>().ok())
        .max()
        .unwrap_or(0)
}

/// Manual findings only, for `playbook doctor`; runs nothing else. `Report`
/// items are left out, because the doctor has its own rows for them.
pub fn pending_manual(ctx: &Ctx) -> Vec<String> {
    registry()
        .iter()
        .filter(|m| matches!(m.kind, Kind::Manual) && !matches!(m.scope, Scope::Report))
        .filter_map(|m| match (m.run)(ctx) {
            Outcome::Warn(msg) => Some(format!("{}: {msg}", m.id)),
            _ => None,
        })
        .collect()
}

/// Records the system prompt placed by init so later edits are detectable.
pub fn record_system_prompt(home: &Path) {
    record_shipped(home, SYSTEM_PROMPT_KEY, &system_prompt_path(home));
}

fn with_lock(home: &Path, f: impl FnOnce()) -> bool {
    with_lock_retry(home, LOCK_RETRIES, f)
}

/// A migration can copy a whole store, so the stale age is far above the default.
const MIGRATION_LOCK_STALE: Duration = Duration::from_secs(300);

fn with_lock_retry(home: &Path, retries: u32, f: impl FnOnce()) -> bool {
    let root = playbook_root_from(home);
    if ensure_private_dir(&root).is_err() {
        return false;
    }
    let lock = root.join(LOCK_DIR);
    remove_stale_lock_dir(&lock, MIGRATION_LOCK_STALE);
    if !acquire_dir_lock(&lock, retries, Duration::from_millis(100)) {
        return false;
    }
    /// Removes the lock when it goes out of scope, also when `f` panics.
    struct Release<'a>(&'a Path);
    impl Drop for Release<'_> {
        fn drop(&mut self) {
            let _ = fs::remove_dir(self.0);
        }
    }
    let _release = Release(&lock);
    f();
    true
}

/// Runs the shipped registry.
pub fn run_pending(ctx: &Ctx) -> Report {
    run_pending_for(ctx, true, true)
}

/// Runs the shipped registry. With `statusline` false the migration that
/// rewrites `statusLine.command` stays pending, so init leaves that key alone.
/// With `hooks` false the dead hook migration stays pending, so init leaves
/// the hook entries alone.
pub fn run_pending_for(ctx: &Ctx, statusline: bool, hooks: bool) -> Report {
    let root = playbook_root_from(&ctx.home);
    let pending = crate::state::pending_legacy(&root);
    let mut migrations = registry();
    migrations.retain(|m| {
        (statusline || m.id != STATUSLINE_MIGRATION) && (hooks || m.id != DEAD_HOOKS_MIGRATION)
    });
    let mut report = run_with(&migrations, ctx);
    if pending > 0 && crate::state::pending_legacy(&root) == 0 {
        report.steps.insert(
            0,
            StepReport::wired(
                "state",
                format!(
                    "imported {pending} old state file(s) into the state table, each renamed to .migrated"
                ),
            ),
        );
    }
    report
}

/// Runs `migrations` in order: Manual ones always, Auto ones once each.
pub fn run_with(migrations: &[Migration], ctx: &Ctx) -> Report {
    run_with_retries(migrations, ctx, LOCK_RETRIES)
}

fn run_with_retries(migrations: &[Migration], ctx: &Ctx, retries: u32) -> Report {
    let mut report = Report::default();
    for m in migrations.iter().filter(|m| matches!(m.kind, Kind::Manual)) {
        if let Outcome::Warn(msg) = (m.run)(ctx) {
            report.warnings.push(format!("{}: {msg}", m.id));
        }
    }
    run_idempotent(migrations, ctx, &mut report, None);
    let done = applied(&read_state(&ctx.home));
    let pending = |m: &&Migration| matches!(m.kind, Kind::Auto) && !done.iter().any(|d| d == m.id);
    if !migrations.iter().any(|m| pending(&m)) {
        return report;
    }
    let locked = with_lock_retry(&ctx.home, retries, || {
        run_auto(migrations, ctx, &mut report, None)
    });
    if !locked {
        report.lock_busy = true;
        report.steps.push(StepReport::skipped(
            "migrations",
            "could not take the migration lock; will retry on the next run",
        ));
    }
    report
}

fn applied(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|l| l.strip_prefix("applied ").map(str::to_string))
        .collect()
}

/// Whether `deadline` has passed, which marks `report` as timed out.
fn out_of_time(deadline: Option<Instant>, report: &mut Report) -> bool {
    report.timed_out |= deadline.is_some_and(|d| Instant::now() >= d);
    report.timed_out
}

/// Runs every `Idempotent` migration in `migrations`. A failure is reported
/// and the rest still run.
fn run_idempotent(
    migrations: &[Migration],
    ctx: &Ctx,
    report: &mut Report,
    deadline: Option<Instant>,
) {
    for m in migrations
        .iter()
        .filter(|m| matches!(m.kind, Kind::Idempotent))
    {
        if out_of_time(deadline, report) {
            return;
        }
        match (m.run)(ctx) {
            Outcome::Failed(step) => {
                report.failed = true;
                report.steps.push(step);
            }
            Outcome::Done(step) | Outcome::Repeat(step) => report.steps.push(step),
            Outcome::Warn(msg) => report.warnings.push(format!("{}: {msg}", m.id)),
            Outcome::Quiet => {}
        }
    }
}

/// Runs each `Auto` migration not yet applied, in order, and records it. The
/// caller holds the migration lock. A failure stops the later ones.
fn run_auto(migrations: &[Migration], ctx: &Ctx, report: &mut Report, deadline: Option<Instant>) {
    for m in migrations.iter().filter(|m| matches!(m.kind, Kind::Auto)) {
        let Some(mut lines) = load_state(&ctx.home) else {
            report.failed = true;
            report
                .warnings
                .push("migrations skipped: could not read the migration record".to_string());
            break;
        };
        if applied(&lines).iter().any(|d| d == m.id) {
            continue;
        }
        if out_of_time(deadline, report) {
            return;
        }
        match (m.run)(ctx) {
            Outcome::Done(step) => {
                lines.push(format!("applied {}", m.id));
                if let Err(err) = write_state(&ctx.home, &lines) {
                    report.failed = true;
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
                report.failed = true;
                report.steps.push(step);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    static CALLS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    static SLOW_RUNS: AtomicUsize = AtomicUsize::new(0);

    fn home(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pb-migrate-{tag}-{}", crate::testing::run_id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ctx(home: &Path) -> Ctx {
        Ctx::for_home(home)
    }

    fn done(name: &'static str) -> Outcome {
        Outcome::Done(StepReport::wired(name, "ok"))
    }

    fn auto(id: &'static str, run: fn(&Ctx) -> Outcome) -> Migration {
        Migration {
            id,
            kind: Kind::Auto,
            scope: Scope::Owned,
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

    fn settings_with(h: &Path, command: &str) -> PathBuf {
        let path = h.join(".claude").join("settings.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body = format!(
            r#"{{"model":"x","statusLine":{{"type":"command","command":{command:?},"refreshInterval":30}}}}"#
        );
        fs::write(&path, body).unwrap();
        path
    }

    fn command_of(path: &Path) -> String {
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        v["statusLine"]["command"].as_str().unwrap().to_string()
    }

    #[test]
    fn the_placed_config_hash_script_is_removed_once() {
        let h = home("cfg-hash");
        let lib = crate::common::paths::playbook_root_from(&h).join("hooks/lib");
        fs::create_dir_all(&lib).unwrap();
        fs::write(lib.join("config-hash.sh"), "config_hash() { :; }\n").unwrap();
        assert!(matches!(
            remove_config_hash_script(&ctx(&h)),
            Outcome::Repeat(_)
        ));
        assert!(!lib.join("config-hash.sh").exists());
        assert!(!lib.exists(), "empty lib dir goes too");
        assert!(matches!(
            remove_config_hash_script(&ctx(&h)),
            Outcome::Quiet
        ));
    }

    #[test]
    fn statusline_migration_rewrites_the_playbook_script_command() {
        let h = home("sl-old");
        let path = settings_with(&h, "bash $HOME/.config/playbook/statusline.sh");
        let out = statusline_rust_command(&ctx(&h));
        assert!(matches!(out, Outcome::Repeat(_)));
        assert_eq!(command_of(&path), "playbook statusline");
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["statusLine"]["refreshInterval"], 30);
        assert_eq!(v["model"], "x");
    }

    #[test]
    fn statusline_migration_rewrites_an_absolute_home_path_too() {
        let h = home("sl-abs");
        let cmd = format!("bash {}/.config/playbook/statusline.sh", h.display());
        let path = settings_with(&h, &cmd);
        statusline_rust_command(&ctx(&h));
        assert_eq!(command_of(&path), "playbook statusline");
    }

    #[test]
    fn statusline_migration_rewrites_a_tilde_path() {
        let h = home("sl-tilde");
        let path = settings_with(&h, "bash ~/.config/playbook/statusline.sh");
        statusline_rust_command(&ctx(&h));
        assert_eq!(command_of(&path), "playbook statusline");
    }

    #[test]
    fn statusline_migration_keeps_a_symlinked_settings_file_a_symlink() {
        let h = home("sl-link");
        let real = h.join("dotfiles").join("settings.json");
        fs::create_dir_all(real.parent().unwrap()).unwrap();
        let link = settings_with(&h, "bash $HOME/.config/playbook/statusline.sh");
        fs::rename(&link, &real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        statusline_rust_command(&ctx(&h));
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(command_of(&real), "playbook statusline");
    }

    #[test]
    fn statusline_migration_leaves_a_custom_command_alone() {
        let h = home("sl-custom");
        for cmd in [
            "bash $HOME/my-statusline.sh",
            "bash $HOME/.config/playbook/statusline.sh --extra",
            "sh $HOME/.config/playbook/statusline.sh",
            "my-tool render",
        ] {
            let path = settings_with(&h, cmd);
            let out = statusline_rust_command(&ctx(&h));
            assert!(matches!(out, Outcome::Quiet));
            assert_eq!(command_of(&path), cmd);
        }
    }

    #[test]
    fn statusline_migration_is_idempotent() {
        let h = home("sl-idem");
        let path = settings_with(&h, "bash $HOME/.config/playbook/statusline.sh");
        statusline_rust_command(&ctx(&h));
        let first = fs::read_to_string(&path).unwrap();
        let out = statusline_rust_command(&ctx(&h));
        assert!(matches!(out, Outcome::Quiet));
        assert_eq!(fs::read_to_string(&path).unwrap(), first);
    }

    #[test]
    fn statusline_migration_ignores_missing_or_invalid_settings() {
        let h = home("sl-none");
        assert!(matches!(statusline_rust_command(&ctx(&h)), Outcome::Quiet));
        let path = h.join(".claude").join("settings.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not json").unwrap();
        assert!(matches!(statusline_rust_command(&ctx(&h)), Outcome::Quiet));
        assert_eq!(fs::read_to_string(&path).unwrap(), "not json");
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
        assert!(report.failed);
        assert!(!read_state(&h).iter().any(|l| l == "applied f"));
    }

    #[test]
    fn a_held_lock_marks_the_report_busy_when_an_auto_migration_waits() {
        // Arrange
        let h = home("busy");
        fs::create_dir_all(playbook_root_from(&h).join(LOCK_DIR)).unwrap();

        // Act
        let report = run_with_retries(&[auto("n", never)], &ctx(&h), 0);

        // Assert
        assert!(report.lock_busy);
        assert!(!report.failed);
    }

    /// How long `sleepy` runs, past the deadline the test gives it. The
    /// margin keeps a loaded machine from reaching the first check late.
    const DEADLINE: Duration = Duration::from_secs(2);

    fn sleepy(_: &Ctx) -> Outcome {
        std::thread::sleep(DEADLINE + Duration::from_millis(100));
        done("sleepy")
    }

    #[test]
    fn a_deadline_lets_the_running_migration_finish_and_starts_no_other() {
        // Arrange
        let h = home("deadline");
        let ms = [auto("s", sleepy), auto("n", never)];
        let mut report = Report::default();
        let deadline = Instant::now() + DEADLINE;

        // Act
        let locked = with_lock_retry(&h, 0, || {
            run_auto(&ms, &ctx(&h), &mut report, Some(deadline))
        });

        // Assert
        assert!(locked && report.timed_out && !report.failed);
        let lines = read_state(&h);
        assert!(lines.iter().any(|l| l == "applied s"), "{lines:?}");
        assert!(!lines.iter().any(|l| l == "applied n"), "{lines:?}");
    }

    #[test]
    fn a_panic_under_the_lock_still_releases_it() {
        // Arrange
        let h = home("unwind");

        // Act
        let caught = std::panic::catch_unwind(|| with_lock_retry(&h, 0, || panic!("boom")));

        // Assert
        assert!(caught.is_err());
        assert!(!playbook_root_from(&h).join(LOCK_DIR).exists());
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
        let notes = owned_notes(&ctx(&h));
        assert_eq!(notes.len(), 1);
        assert!(notes[0].starts_with("0003-system-prompt-edited: "));
    }

    #[test]
    fn an_unrecorded_file_is_never_reported_as_edited() {
        let h = home("norecord");
        let prompt = system_prompt_path(&h);
        fs::create_dir_all(prompt.parent().unwrap()).unwrap();
        fs::write(&prompt, "anything").unwrap();
        assert!(!user_edited(&h, SYSTEM_PROMPT_KEY, &prompt));
    }

    static HELD_RUNS: AtomicUsize = AtomicUsize::new(0);
    fn held_probe(_: &Ctx) -> Outcome {
        HELD_RUNS.fetch_add(1, Ordering::SeqCst);
        done("held")
    }

    #[test]
    fn a_held_lock_skips_auto_migrations_with_a_warning() {
        let h = home("held");
        fs::create_dir_all(playbook_root_from(&h).join(LOCK_DIR)).unwrap();
        let report = run_with_retries(&[auto("held-a", held_probe)], &ctx(&h), 2);
        assert_eq!(HELD_RUNS.load(Ordering::SeqCst), 0);
        assert!(report.steps[0].detail.contains("lock"));
    }

    fn repeats(_: &Ctx) -> Outcome {
        Outcome::Repeat(StepReport::wired("rep", "ok"))
    }

    #[test]
    fn a_repeat_migration_is_never_recorded() {
        let h = home("repeat");
        let ms = [Migration {
            id: "r",
            kind: Kind::Idempotent,
            scope: Scope::Owned,
            run: repeats,
        }];
        run_with(&ms, &ctx(&h));
        let second = run_with(&ms, &ctx(&h));
        assert_eq!(second.steps.len(), 1);
        assert!(!read_state(&h).iter().any(|l| l.starts_with("applied")));
    }

    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push((p.clone(), fs::read(&p).unwrap_or_default()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn owned_and_report_migrations_write_nothing_outside_the_playbook_folder() {
        // Arrange: user files that the Consent migrations would change, and
        // Claude Code files the Report and orphan checks read.
        let h = home("scope-guard");
        let script = crate::init::statusline::playbook_statusline_path(&h);
        let settings = h.join(".claude/settings.json");
        fs::create_dir_all(h.join(".claude")).unwrap();
        fs::write(
            &settings,
            format!(
                r#"{{"statusLine":{{"type":"command","command":"bash {}"}},"hooks":{{"Stop":[{{"hooks":[{{"type":"command","command":"~/.claude/hooks/gone.sh"}}]}}]}}}}"#,
                script.display()
            ),
        )
        .unwrap();
        let cache = h.join(".claude/plugins/cache/pragmatic-engineer/playbook");
        fs::create_dir_all(cache.join("0.24.0")).unwrap();
        fs::write(cache.join("0.24.0/.orphaned_at"), "1").unwrap();
        fs::write(
            h.join(".claude/plugins/installed_plugins.json"),
            r#"{"version":2,"plugins":{"playbook-hooks@pragmatic-engineer":[{"scope":"user"}]}}"#,
        )
        .unwrap();
        fs::write(h.join(".zshrc"), "source ~/.claude/shell/zsh/cc.zsh\n").unwrap();
        fs::create_dir_all(h.join("project")).unwrap();
        fs::write(h.join("project/notes.md"), "mine").unwrap();
        let outside = |h: &Path| -> Vec<(PathBuf, Vec<u8>)> {
            snapshot(h)
                .into_iter()
                .filter(|(p, _)| !p.starts_with(playbook_root_from(h)))
                .collect()
        };
        let before = outside(&h);

        // Act
        let run = run_owned(&ctx(&h), 0, None);
        let notes = owned_notes(&ctx(&h));
        let open = open_items(&ctx(&h), None);

        // Assert
        assert!(!run.lock_busy && !run.failed);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].starts_with("0014-orphaned-plugin-caches: "));
        assert_eq!(outside(&h), before);
        let command = |id: &str| open.iter().find(|i| i.id == id).and_then(|i| i.command);
        assert_eq!(
            command("0006-statusline-rust-command"),
            Some("playbook setup --statusline")
        );
        assert_eq!(command("0007-shell-init-rc-line"), Some("playbook setup"));
        assert_eq!(
            command(DEAD_HOOKS_MIGRATION),
            Some("playbook setup --hooks")
        );
        assert!(open
            .iter()
            .any(|i| i.id == "0012-other-playbook-plugins" && i.command.is_none()));
    }

    #[test]
    fn the_statusline_check_matches_the_rewrite_and_skips_an_edited_script() {
        let h = home("statusline-pending");
        let script = crate::init::statusline::playbook_statusline_path(&h);
        let path = settings_with(&h, &format!("bash {}", script.display()));
        let pending = |edited| crate::init::statusline::legacy_command_pending(&path, &h, edited);
        assert!(pending(false));
        assert!(!pending(true));
        settings_with(&h, "my-own-statusline");
        assert!(!pending(false));
    }

    #[test]
    fn every_migration_is_safe_and_idempotent_on_a_fresh_machine() {
        // Arrange: an empty home, so every file a migration reads is missing.
        let h = home("fresh-all");

        // Act
        let first = run_pending(&ctx(&h));
        let after_first = snapshot(&h);
        let second = run_pending(&ctx(&h));

        // Assert
        for report in [&first, &second] {
            assert!(!report.failed && !report.lock_busy);
            assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        }
        assert!(snapshot(&h)
            .iter()
            .all(|(p, _)| p.starts_with(playbook_root_from(&h))));
        assert_eq!(snapshot(&h), after_first);
        assert!(open_items(&ctx(&h), None).is_empty());
    }

    #[test]
    fn the_schema_is_the_highest_registry_number() {
        assert_eq!(schema(), 14);
    }
}
