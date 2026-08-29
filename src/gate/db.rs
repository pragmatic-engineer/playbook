// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Schema and connection layer for the gate-check database at
//! `.claude/state.db`. `libsql`'s Rust API is async (tokio-based), so each
//! public function here builds its OWN scoped current-thread runtime and
//! blocks on it; nothing else in this binary becomes async as a result.

use crate::common::atomic::with_dir_lock;
use std::fs;
use std::io::Write;
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
/// `gate check` (a later Work Unit) needs the evidence and command text
/// too, and a single-row lookup by primary key costs nothing extra to carry
/// the full record.
pub struct GatePhaseRow {
    pub verdict: String,
    pub evidence: String,
    pub command: String,
    pub recorded_at: String,
}

/// Open (creating if missing) the libsql database at `path`, ensure the
/// `gate_phases` schema and pragmas are in place, and gitignore
/// `.claude/state.db` the first time it is created there.
pub fn open_db(path: &Path) -> Result<libsql::Connection, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create directory {}: {e}", parent.display()))?;
    }
    ensure_state_db_gitignored(path);

    let runtime = new_runtime()?;
    runtime.block_on(async {
        let db = libsql::Builder::new_local(path)
            .build()
            .await
            .map_err(|e| format!("failed to open database at {}: {e}", path.display()))?;
        let conn = db
            .connect()
            .map_err(|e| format!("failed to connect to database at {}: {e}", path.display()))?;

        conn.execute(SCHEMA_SQL, ())
            .await
            .map_err(|e| format!("failed to create gate_phases schema: {e}"))?;
        run_pragma(&conn, "PRAGMA journal_mode=WAL").await?;
        run_pragma(&conn, "PRAGMA busy_timeout=5000").await?;

        Ok(conn)
    })
}

/// Insert or replace the `(plan_slug, phase)` row with the given values.
pub fn upsert_phase(
    conn: &libsql::Connection,
    plan_slug: &str,
    phase: &str,
    verdict: &str,
    evidence: &str,
    command: &str,
    recorded_at: &str,
) -> Result<(), String> {
    let runtime = new_runtime()?;
    runtime.block_on(async {
        conn.execute(
            "INSERT OR REPLACE INTO gate_phases \
             (plan_slug, phase, verdict, evidence, command, recorded_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (plan_slug, phase, verdict, evidence, command, recorded_at),
        )
        .await
        .map_err(|e| format!("failed to upsert gate phase {plan_slug}/{phase}: {e}"))?;
        Ok(())
    })
}

/// Look up the row for `(plan_slug, phase)`, or `None` if it has never been
/// recorded.
pub fn query_phase(
    conn: &libsql::Connection,
    plan_slug: &str,
    phase: &str,
) -> Result<Option<GatePhaseRow>, String> {
    let runtime = new_runtime()?;
    runtime.block_on(async {
        let mut rows = conn
            .query(
                "SELECT verdict, evidence, command, recorded_at FROM gate_phases \
                 WHERE plan_slug = ?1 AND phase = ?2",
                (plan_slug, phase),
            )
            .await
            .map_err(|e| format!("failed to query gate phase {plan_slug}/{phase}: {e}"))?;

        let row = rows
            .next()
            .await
            .map_err(|e| format!("failed to read gate phase {plan_slug}/{phase}: {e}"))?;
        let Some(row) = row else {
            return Ok(None);
        };

        Ok(Some(GatePhaseRow {
            verdict: row
                .get(0)
                .map_err(|e| format!("failed to read verdict column: {e}"))?,
            evidence: row
                .get(1)
                .map_err(|e| format!("failed to read evidence column: {e}"))?,
            command: row
                .get(2)
                .map_err(|e| format!("failed to read command column: {e}"))?,
            recorded_at: row
                .get(3)
                .map_err(|e| format!("failed to read recorded_at column: {e}"))?,
        }))
    })
}

/// A scoped current-thread runtime, built fresh for a single `block_on` call.
/// `libsql`'s Rust API is async, but its local-file operations never
/// actually wait on I/O, so a runtime that lives only for the duration of
/// one call is enough; nothing else in this binary needs to become async.
fn new_runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to build async runtime: {e}"))
}

/// `PRAGMA` statements that return a row (`journal_mode`, `busy_timeout`)
/// panic under `Connection::execute` with `ExecuteReturnedRows`, confirmed
/// by spike. Run them with `query` instead and drain the row it hands back.
async fn run_pragma(conn: &libsql::Connection, sql: &str) -> Result<(), String> {
    let mut rows = conn
        .query(sql, ())
        .await
        .map_err(|e| format!("failed to run `{sql}`: {e}"))?;
    while rows
        .next()
        .await
        .map_err(|e| format!("failed to read `{sql}` response: {e}"))?
        .is_some()
    {}
    Ok(())
}

/// Append `.claude/state.db` to the repo's `.gitignore` the first time a DB
/// is created at that exact shape of path (a `state.db` file directly under
/// a `.claude` directory), so it never needs a separate manual step. Any
/// other `path` is left alone: this DB has exactly one real caller path, so
/// there is nothing else to generalize for yet.
fn ensure_state_db_gitignored(path: &Path) {
    let (Some(file_name), Some(claude_dir)) = (path.file_name(), path.parent()) else {
        return;
    };
    if file_name != "state.db" {
        return;
    }
    let Some(claude_dir_name) = claude_dir.file_name() else {
        return;
    };
    if claude_dir_name != ".claude" {
        return;
    }
    let Some(repo_root) = claude_dir.parent() else {
        return;
    };

    let entry = format!(
        "{}/{}",
        claude_dir_name.to_string_lossy(),
        file_name.to_string_lossy()
    );
    append_gitignore_entry(&repo_root.join(".gitignore"), &entry);
}

/// Locked, idempotent check-then-append: skip the append if `entry` is
/// already present, matching the shell pattern `grep -qxF "$d" .gitignore
/// || printf '%s\n' "$d" >> .gitignore` used elsewhere in this repo. Mirrors
/// `atomic_append`'s locking (`src/common/atomic.rs:77-93`): only removes
/// the lock directory this call created, and fails soft, never panics.
fn append_gitignore_entry(gitignore_path: &Path, entry: &str) {
    let mut lock_os = gitignore_path.as_os_str().to_owned();
    lock_os.push(".lock");
    let lock_path = PathBuf::from(lock_os);

    let (acquired, ()) = with_dir_lock(&lock_path, 50, Duration::from_millis(10), || {
        let already_present = fs::read_to_string(gitignore_path)
            .map(|contents| contents.lines().any(|line| line == entry))
            .unwrap_or(false);
        if already_present {
            return;
        }
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(gitignore_path)
        {
            let _ = writeln!(file, "{entry}");
        }
    });
    if acquired {
        let _ = fs::remove_dir(&lock_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;

    /// Count rows for a `(plan_slug, phase)` pair via a fresh connection to
    /// `path`, proving `INSERT OR REPLACE` never leaves a duplicate behind.
    fn count_rows(path: &Path, plan_slug: &str, phase: &str) -> i64 {
        let conn = open_db(path).expect("open_db for count_rows");
        let runtime = new_runtime().expect("runtime for count_rows");
        runtime.block_on(async {
            let mut rows = conn
                .query(
                    "SELECT COUNT(*) FROM gate_phases WHERE plan_slug = ?1 AND phase = ?2",
                    (plan_slug, phase),
                )
                .await
                .expect("count query");
            let row = rows
                .next()
                .await
                .expect("count row read")
                .expect("count row present");
            row.get::<i64>(0).expect("count column")
        })
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

    #[test]
    fn open_db_gitignores_claude_state_db_path() {
        // Arrange
        let repo = scratch_dir("gitignore-repo");
        let path = repo.join(".claude").join("state.db");

        // Act
        open_db(&path).expect("open");

        // Assert
        let gitignore =
            fs::read_to_string(repo.join(".gitignore")).expect("gitignore should be created");
        assert!(
            gitignore.lines().any(|l| l == ".claude/state.db"),
            "gitignore should list .claude/state.db, got: {gitignore:?}"
        );

        let _ = fs::remove_dir_all(&repo);
    }
}
