// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Schema and connection layer for the gate-check database at
//! `.claude/state.db`, backed by `rusqlite` (the `bundled` feature compiles
//! SQLite from source, so no system SQLite install is required).
//!
//! An earlier version used `libsql` (tokio-based, async). `gate check`
//! reliably segfaulted inside `sqlite3Close` on real musl release builds
//! (x86_64 and aarch64 both, confirmed via `gdb`), even after fixing every
//! runtime-lifecycle issue that could explain it: a `Connection`'s open,
//! use, and drop were kept inside one tokio runtime, and every `Rows`
//! cursor was drained to completion. Neither resolved it, and the crash
//! matched an open, maintainer-filed bug in `libsql`'s own connection
//! lifecycle code. `rusqlite` needs none of that: it is fully synchronous,
//! is the de facto standard Rust SQLite binding, and this binary has no
//! other reason to be async.

use rusqlite::OptionalExtension;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCHEMA_SQL: &str = "CREATE TABLE IF NOT EXISTS gate_phases (
    plan_slug TEXT NOT NULL,
    phase TEXT NOT NULL,
    verdict TEXT NOT NULL CHECK(verdict IN ('PASS','FAIL','WARN','INCONCLUSIVE')),
    evidence TEXT NOT NULL,
    command TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    PRIMARY KEY (plan_slug, phase)
) WITHOUT ROWID";

/// One row of `gate_phases`. Returned whole rather than just the verdict:
/// a single-row lookup by primary key costs nothing extra to carry the full
/// record, and a future consumer (e.g. a `gate show`) can use it as-is.
pub struct GatePhaseRow {
    pub verdict: String,
    pub evidence: String,
    pub command: String,
    pub recorded_at: String,
}

/// Open (creating if missing) the SQLite database at `path`, ensuring the
/// `gate_phases` schema and pragmas are in place.
pub fn open_db(path: &Path) -> Result<rusqlite::Connection, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create directory {}: {e}", parent.display()))?;
    }

    let conn = rusqlite::Connection::open(path)
        .map_err(|e| format!("failed to open database at {}: {e}", path.display()))?;

    // busy_timeout must be set BEFORE any statement that can contend for
    // the write lock (the schema creation below, and future callers
    // opening the same fresh file concurrently): a connection's default
    // busy timeout is 0, so an SQLITE_BUSY hit before this is set fails
    // immediately instead of retrying. `commands/scope.md` fires two
    // `gate record` processes in parallel (Phase 2 and Phase 3), so this
    // ordering is load-bearing, not cosmetic.
    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(|e| format!("failed to set busy_timeout: {e}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("failed to set journal_mode=WAL: {e}"))?;
    conn.execute_batch(SCHEMA_SQL)
        .map_err(|e| format!("failed to create gate_phases schema: {e}"))?;

    Ok(conn)
}

/// Insert or replace the `(plan_slug, phase)` row with the given values.
pub fn upsert_phase(
    conn: &rusqlite::Connection,
    plan_slug: &str,
    phase: &str,
    verdict: &str,
    evidence: &str,
    command: &str,
    recorded_at: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO gate_phases \
         (plan_slug, phase, verdict, evidence, command, recorded_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![plan_slug, phase, verdict, evidence, command, recorded_at],
    )
    .map_err(|e| format!("failed to upsert gate phase {plan_slug}/{phase}: {e}"))?;
    Ok(())
}

/// Look up the row for `(plan_slug, phase)`, or `None` if it has never been
/// recorded.
pub fn query_phase(
    conn: &rusqlite::Connection,
    plan_slug: &str,
    phase: &str,
) -> Result<Option<GatePhaseRow>, String> {
    conn.query_row(
        "SELECT verdict, evidence, command, recorded_at FROM gate_phases \
         WHERE plan_slug = ?1 AND phase = ?2",
        rusqlite::params![plan_slug, phase],
        |row| {
            Ok(GatePhaseRow {
                verdict: row.get(0)?,
                evidence: row.get(1)?,
                command: row.get(2)?,
                recorded_at: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("failed to query gate phase {plan_slug}/{phase}: {e}"))
}

/// Marks a completed [`migrate_legacy_repo_local`] run; absent means resume.
const MIGRATION_SENTINEL: &str = ".migration-complete";

/// The legacy items a pre-ADR-0012 checkout held under its own `.claude/`.
const LEGACY_ITEMS: &[&str] = &["state.db", "plans", "designs", "implement", "worktrees"];

/// Moves legacy `.claude/{state.db,plans,designs,implement,worktrees}` items
/// under `repo_root` to `dest_base`, once. Copies and verifies before deleting the source, so any error leaves the original untouched.
pub(crate) fn migrate_legacy_repo_local(repo_root: &Path, dest_base: &Path) -> Result<(), String> {
    let legacy_root = repo_root.join(".claude");
    let sentinel = dest_base.join(MIGRATION_SENTINEL);
    if sentinel.is_file() {
        return Ok(());
    }

    let present: Vec<&str> = LEGACY_ITEMS
        .iter()
        .copied()
        .filter(|name| legacy_root.join(name).exists())
        .collect();
    if present.is_empty() {
        return Ok(());
    }

    fs::create_dir_all(dest_base)
        .map_err(|e| format!("failed to create {}: {e}", dest_base.display()))?;

    let mut files = Vec::new();
    for name in &present {
        let item = legacy_root.join(name);
        if item.is_dir() {
            collect_relative_files(&item, Path::new(name), &mut files);
        } else {
            files.push(PathBuf::from(name));
        }
    }

    copy_all(&legacy_root, dest_base, &files).map_err(|e| {
        format!(
            "legacy migration copy to {} failed, the original is untouched: {e}",
            dest_base.display()
        )
    })?;

    if !all_copied_and_verified(&legacy_root, dest_base, &files) {
        return Err(format!(
            "legacy migration verification failed after copying to {}, the original is untouched",
            dest_base.display()
        ));
    }

    write_migration_sentinel(&sentinel).map_err(|e| {
        format!("legacy migration copy verified but could not write completion marker: {e}")
    })?;

    // Removed only now that the destination is verified complete.
    for name in &present {
        remove_legacy_item(&legacy_root.join(name));
    }
    // Harmless no-op if anything else still lives under `.claude`.
    let _ = fs::remove_dir(&legacy_root);
    Ok(())
}

/// Every regular file under `dir`, recursively, relative to `prefix`.
fn collect_relative_files(dir: &Path, prefix: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let rel = prefix.join(entry.file_name());
        if path.is_dir() {
            collect_relative_files(&path, &rel, out);
        } else {
            out.push(rel);
        }
    }
}

/// Copies each of `files` from `old_root` to `new_root`, stopping at the first failure so a partial copy is never marked done.
fn copy_all(old_root: &Path, new_root: &Path, files: &[PathBuf]) -> std::io::Result<()> {
    for rel in files {
        let dest = new_root.join(rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(old_root.join(rel), &dest)?;
    }
    Ok(())
}

/// Every source file exists at the destination with a matching size.
fn all_copied_and_verified(old_root: &Path, new_root: &Path, files: &[PathBuf]) -> bool {
    files.iter().all(|rel| {
        let source_len = fs::metadata(old_root.join(rel)).ok().map(|m| m.len());
        let dest_len = fs::metadata(new_root.join(rel)).ok().map(|m| m.len());
        source_len.is_some() && source_len == dest_len
    })
}

fn write_migration_sentinel(sentinel: &Path) -> std::io::Result<()> {
    if let Some(parent) = sentinel.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(sentinel, "migrated\n")
}

fn remove_legacy_item(path: &Path) {
    if path.is_dir() {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    /// Count rows for a `(plan_slug, phase)` pair via a fresh connection to
    /// `path`, proving `INSERT OR REPLACE` never leaves a duplicate behind.
    fn count_rows(path: &Path, plan_slug: &str, phase: &str) -> i64 {
        let conn = open_db(path).expect("open_db for count_rows");
        conn.query_row(
            "SELECT COUNT(*) FROM gate_phases WHERE plan_slug = ?1 AND phase = ?2",
            rusqlite::params![plan_slug, phase],
            |row| row.get(0),
        )
        .expect("count query")
    }

    #[test]
    fn open_db_sets_busy_timeout_before_returning() {
        // Arrange
        let dir = scratch_dir("busy-timeout");
        let path = dir.join("state.db");

        // Act
        let conn = open_db(&path).expect("open");
        let value: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("busy_timeout query");

        // Assert: pinned at the value open_db sets, proving the call ran
        // (not the SQLite default of 0) and that it does not get shadowed
        // by a later statement in open_db's own body.
        assert_eq!(value, 5000);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_db_creates_schema_on_fresh_path() {
        // Arrange
        let dir = scratch_dir("open-fresh");
        let path = dir.join("state.db");

        // Act
        let result = open_db(&path);

        // Assert
        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
        assert!(path.is_file(), "database file should exist at {path:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_db_reopen_is_idempotent_and_keeps_data() {
        // Arrange
        let dir = scratch_dir("open-reopen");
        let path = dir.join("state.db");
        let conn = open_db(&path).expect("first open");
        upsert_phase(
            &conn,
            "plan-a",
            "spec",
            "PASS",
            "ev",
            "cmd",
            "2026-01-01T00:00:00Z",
        )
        .expect("upsert before reopen");
        drop(conn);

        // Act
        let reopened = open_db(&path);

        // Assert
        assert!(
            reopened.is_ok(),
            "reopen should not error: {:?}",
            reopened.err()
        );
        let row = query_phase(&reopened.unwrap(), "plan-a", "spec")
            .expect("query after reopen")
            .expect("row should survive reopen");
        assert_eq!(row.verdict, "PASS");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_phase_twice_keeps_a_single_row_with_latest_values() {
        // Arrange
        let dir = scratch_dir("upsert-twice");
        let path = dir.join("state.db");
        let conn = open_db(&path).expect("open");
        upsert_phase(
            &conn,
            "plan-a",
            "spec",
            "WARN",
            "first evidence",
            "cmd-1",
            "2026-01-01T00:00:00Z",
        )
        .expect("first upsert");

        // Act
        upsert_phase(
            &conn,
            "plan-a",
            "spec",
            "PASS",
            "second evidence",
            "cmd-2",
            "2026-01-02T00:00:00Z",
        )
        .expect("second upsert");

        // Assert
        let row = query_phase(&conn, "plan-a", "spec")
            .expect("query")
            .expect("row should exist");
        assert_eq!(row.verdict, "PASS");
        assert_eq!(row.evidence, "second evidence");
        assert_eq!(count_rows(&path, "plan-a", "spec"), 1);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn upsert_phase_rejects_invalid_verdict() {
        // Arrange
        let dir = scratch_dir("bad-verdict");
        let path = dir.join("state.db");
        let conn = open_db(&path).expect("open");

        // Act
        let result = upsert_phase(
            &conn,
            "plan-a",
            "spec",
            "NOPE",
            "ev",
            "cmd",
            "2026-01-01T00:00:00Z",
        );

        // Assert
        assert!(
            result.is_err(),
            "an invalid verdict should be rejected, got {result:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn query_phase_never_bleeds_across_plan_slugs() {
        // Arrange
        let dir = scratch_dir("cross-plan");
        let path = dir.join("state.db");
        let conn = open_db(&path).expect("open");
        upsert_phase(
            &conn,
            "plan-a",
            "spec",
            "PASS",
            "a-evidence",
            "a-cmd",
            "2026-01-01T00:00:00Z",
        )
        .expect("upsert plan-a");
        upsert_phase(
            &conn,
            "plan-b",
            "spec",
            "FAIL",
            "b-evidence",
            "b-cmd",
            "2026-01-02T00:00:00Z",
        )
        .expect("upsert plan-b");

        // Act
        let row_a = query_phase(&conn, "plan-a", "spec")
            .expect("query plan-a")
            .expect("row a");
        let row_b = query_phase(&conn, "plan-b", "spec")
            .expect("query plan-b")
            .expect("row b");

        // Assert
        assert_eq!(row_a.verdict, "PASS");
        assert_eq!(row_b.verdict, "FAIL");
        assert_ne!(row_a.evidence, row_b.evidence);

        let _ = fs::remove_dir_all(&dir);
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn legacy_migration_moves_state_db_plans_designs_implement_worktrees() {
        // Arrange: every legacy item present, `plans`/`designs` holding real
        // nested content, matching the Done-When's content-preserving claim.
        let base = scratch_dir("legacy-migrate-full-set");
        let repo_root = base.join("repo");
        let legacy = repo_root.join(".claude");
        write_file(&legacy.join("state.db"), "sqlite bytes");
        write_file(&legacy.join("plans").join("topic.md"), "plan content");
        write_file(&legacy.join("designs").join("topic.md"), "design content");
        write_file(&legacy.join("implement").join("topic.progress.md"), "wip");
        write_file(&legacy.join("worktrees").join(".keep"), "");
        let dest_base = base.join("dest");

        // Act
        let result = migrate_legacy_repo_local(&repo_root, &dest_base);

        // Assert
        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
        assert_eq!(
            fs::read_to_string(dest_base.join("state.db")).unwrap(),
            "sqlite bytes"
        );
        assert_eq!(
            fs::read_to_string(dest_base.join("plans").join("topic.md")).unwrap(),
            "plan content"
        );
        assert_eq!(
            fs::read_to_string(dest_base.join("designs").join("topic.md")).unwrap(),
            "design content"
        );
        assert_eq!(
            fs::read_to_string(dest_base.join("implement").join("topic.progress.md")).unwrap(),
            "wip"
        );
        assert!(dest_base.join(MIGRATION_SENTINEL).is_file());
        assert!(
            !legacy.exists(),
            "the legacy .claude dir's migrated items should be gone"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn legacy_migration_is_idempotent_second_call_finds_nothing() {
        // Arrange: a sentinel already present, plus stray content on both
        // sides that a no-op second call must not touch.
        let base = scratch_dir("legacy-migrate-idempotent");
        let repo_root = base.join("repo");
        let legacy = repo_root.join(".claude");
        write_file(&legacy.join("state.db"), "leftover legacy bytes");
        let dest_base = base.join("dest");
        write_file(&dest_base.join("state.db"), "already migrated bytes");
        write_file(&dest_base.join(MIGRATION_SENTINEL), "migrated\n");

        // Act
        let result = migrate_legacy_repo_local(&repo_root, &dest_base);

        // Assert
        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
        assert_eq!(
            fs::read_to_string(legacy.join("state.db")).unwrap(),
            "leftover legacy bytes",
            "a no-op second call must not touch the untouched legacy leftovers"
        );
        assert_eq!(
            fs::read_to_string(dest_base.join("state.db")).unwrap(),
            "already migrated bytes"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn legacy_migration_leaves_source_untouched_on_simulated_failure() {
        // Arrange: a read-only destination so the very first copy write
        // fails, simulating a crash partway through.
        let base = scratch_dir("legacy-migrate-crash");
        let repo_root = base.join("repo");
        let legacy = repo_root.join(".claude");
        write_file(&legacy.join("state.db"), "must survive a crash");
        let dest_base = base.join("dest");
        fs::create_dir_all(&dest_base).unwrap();
        fs::set_permissions(&dest_base, fs::Permissions::from_mode(0o555)).unwrap();
        let probe = dest_base.join(".write-probe");
        let permissions_are_enforced = fs::write(&probe, "x").is_err();
        let _ = fs::set_permissions(&dest_base, fs::Permissions::from_mode(0o755));
        let _ = fs::remove_file(&probe);
        if !permissions_are_enforced {
            eprintln!(
                "skipping legacy_migration_leaves_source_untouched_on_simulated_failure: \
                 running as a user that bypasses directory permissions"
            );
            return;
        }
        fs::set_permissions(&dest_base, fs::Permissions::from_mode(0o555)).unwrap();

        // Act
        let result = migrate_legacy_repo_local(&repo_root, &dest_base);

        // Assert
        fs::set_permissions(&dest_base, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_err(), "expected Err on a simulated write failure");
        assert_eq!(
            fs::read_to_string(legacy.join("state.db")).unwrap(),
            "must survive a crash"
        );
        assert!(legacy.join("state.db").exists());
        assert!(!dest_base.join(MIGRATION_SENTINEL).exists());

        let _ = fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn legacy_migration_resumes_correctly_after_simulated_kill_mid_copy() {
        // Arrange: `plans/` partially copied already (one of two files), no
        // sentinel, simulating a process killed mid-copy on a prior attempt.
        let base = scratch_dir("legacy-migrate-resume");
        let repo_root = base.join("repo");
        let legacy = repo_root.join(".claude");
        write_file(&legacy.join("plans").join("a.md"), "plan a content");
        write_file(&legacy.join("plans").join("b.md"), "plan b content");
        let dest_base = base.join("dest");
        write_file(&dest_base.join("plans").join("a.md"), "plan a content");

        // Act
        let result = migrate_legacy_repo_local(&repo_root, &dest_base);

        // Assert
        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
        assert_eq!(
            fs::read_to_string(dest_base.join("plans").join("a.md")).unwrap(),
            "plan a content"
        );
        assert_eq!(
            fs::read_to_string(dest_base.join("plans").join("b.md")).unwrap(),
            "plan b content"
        );
        assert!(dest_base.join(MIGRATION_SENTINEL).is_file());
        assert!(
            !legacy.exists(),
            "a completed resume should remove the original"
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn legacy_migration_is_a_noop_when_nothing_legacy_exists() {
        // Arrange: a fresh checkout with no `.claude` at all.
        let base = scratch_dir("legacy-migrate-fresh");
        let repo_root = base.join("repo");
        fs::create_dir_all(&repo_root).unwrap();
        let dest_base = base.join("dest");

        // Act
        let result = migrate_legacy_repo_local(&repo_root, &dest_base);

        // Assert
        assert!(result.is_ok(), "expected Ok, got {:?}", result.err());
        assert!(!dest_base.exists());

        let _ = fs::remove_dir_all(&base);
    }
}
