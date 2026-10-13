// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The data folder, `~/.config/playbook/data`, where every database playbook
//! creates lives. This module also holds the small migration framework that
//! moves a database from its old path into the folder.
//!
//! A step is a [`Move`]: an old file, a new file and a [`Policy`]. Running it
//! is idempotent and safe against concurrent processes:
//!
//! * The old file is copied with `VACUUM INTO`, which reads a consistent
//!   snapshot including the WAL, then checked table by table, then renamed
//!   into place. Nothing is ever deleted.
//! * The old file and its `-wal` and `-shm` are renamed to `*.migrated` once
//!   the copy is verified.
//! * When the new file already exists (for example an older binary kept
//!   writing the old path and recreated it), the rows are merged with
//!   `INSERT OR IGNORE`, which cannot duplicate a row with a primary key.
//! * A corrupt old file is left alone and reported on each run.
//! * The old file's write lock is held from the copy to the rename, and
//!   merged tables must have a primary key, so a repeat cannot double count.

use crate::common::atomic::{acquire_dir_lock_aged, ensure_private_dir, LONG_LOCK_STALE_AGE};
use crate::common::paths::playbook_root_from;
use crate::common::session::home_dir;
use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `$HOME/.config/playbook/data`.
pub fn data_dir() -> PathBuf {
    data_dir_from(&home_dir())
}

/// Core of [`data_dir`], taking `home` explicitly for tests.
pub fn data_dir_from(home: &Path) -> PathBuf {
    playbook_root_from(home).join("data")
}

/// [`data_dir`], created owner only when missing.
pub fn ensure_data_dir() -> std::io::Result<PathBuf> {
    let dir = data_dir();
    ensure_private_dir(&dir)?;
    Ok(dir)
}

/// How a step treats a new file that already exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// Merge these tables from the old file with `INSERT OR IGNORE`.
    MergeTables(&'static [&'static str]),
    /// The data is a rebuildable cache: keep the new file and set the old one
    /// aside.
    KeepNew,
}

/// One database to move into the data folder.
#[derive(Clone, Debug)]
pub struct Move {
    pub old: PathBuf,
    pub new: PathBuf,
    pub policy: Policy,
}

/// What a step did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// There was no old file.
    Nothing,
    /// The old file became the new file.
    Moved,
    /// Rows from the old file were merged into the existing new file.
    Merged { inserted: u64 },
    /// The old file was set aside because the new file wins.
    SetAside,
    /// The old file is unreadable and was left in place.
    Corrupt(String),
}

const SIDE_SUFFIXES: [&str; 3] = ["", "-wal", "-shm"];
const LOCK_RETRIES: u32 = 300;
const LOCK_DELAY: Duration = Duration::from_millis(20);

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn q(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn tables(conn: &Connection, schema: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT name FROM {schema}.sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
    ))?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}

fn columns(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA {schema}.table_info({})", q(table)))?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
    rows.collect()
}

fn count(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM {schema}.{}", q(table)),
        [],
        |r| r.get(0),
    )
}

/// Why the old file could not be opened for a move.
enum OldError {
    /// The file is not a usable database: left in place.
    Corrupt(String),
    /// A transient failure (busy, permissions): the next run retries.
    Transient(String),
}

fn classify(e: rusqlite::Error) -> OldError {
    match e.sqlite_error_code() {
        Some(rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt) => {
            OldError::Corrupt(e.to_string())
        }
        _ => OldError::Transient(e.to_string()),
    }
}

/// Open the old file read only and run an integrity check.
fn open_old(old: &Path) -> Result<Connection, OldError> {
    let conn =
        Connection::open_with_flags(old, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(classify)?;
    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(classify)?;
    let verdict: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(classify)?;
    if verdict != "ok" {
        return Err(OldError::Corrupt(verdict));
    }
    Ok(conn)
}

/// Take the old file's write lock until dropped, so no writer can commit
/// between the copy and the rename. A writer that holds it longer than the
/// wait makes the move retry on the next run.
fn lock_old(old: &Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(old, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_millis(2000))
        .map_err(|e| e.to_string())?;
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("the old database is busy: {e}"))?;
    Ok(conn)
}

/// Row counts per table of `conn`, sorted by table name.
fn table_counts(conn: &Connection) -> Result<Vec<(String, i64)>, String> {
    let mut out = Vec::new();
    let mut names = tables(conn, "main").map_err(|e| e.to_string())?;
    names.sort();
    for t in names {
        let n = count(conn, "main", &t).map_err(|e| e.to_string())?;
        out.push((t, n));
    }
    Ok(out)
}

/// Rename the old file and its sidecars to `*.migrated`; when that name is
/// taken (an older binary recreated the old path and it was merged again) the
/// name gets a numeric suffix. Nothing is overwritten.
fn set_aside(old: &Path) -> Result<(), String> {
    let mut n = 0u32;
    let suffix = loop {
        let s = if n == 0 {
            ".migrated".to_string()
        } else {
            format!(".migrated.{n}")
        };
        if !with_suffix(old, &s).exists() {
            break s;
        }
        n += 1;
    };
    for side in SIDE_SUFFIXES {
        let from = with_suffix(old, side);
        if from.exists() {
            // SQLite pairs a main file with `<main>-wal`, so the sidecars
            // keep that shape next to the renamed main file.
            let to = with_suffix(&with_suffix(old, &suffix), side);
            fs::rename(&from, &to)
                .map_err(|e| format!("could not rename {}: {e}", from.display()))?;
        }
    }
    Ok(())
}

fn copy_then_verify(step: &Move, old_conn: &Connection) -> Result<(), String> {
    let tmp = with_suffix(&step.new, &format!(".tmp-{}", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let result = (|| {
        old_conn
            .execute("VACUUM INTO ?1", [tmp.to_string_lossy().as_ref()])
            .map_err(|e| format!("copy failed: {e}"))?;
        let copy = Connection::open_with_flags(&tmp, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| e.to_string())?;
        if table_counts(&copy)? != table_counts(old_conn)? {
            return Err("row counts differ after the copy".to_string());
        }
        drop(copy);
        crate::common::atomic::make_db_file_private(&tmp)?;
        // A hard link fails when the target exists, unlike rename, which
        // would silently replace a file another process just created.
        fs::hard_link(&tmp, &step.new).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                "the new file appeared during the copy".to_string()
            } else {
                e.to_string()
            }
        })?;
        Ok(())
    })();
    let _ = fs::remove_file(&tmp);
    result
}

/// Remove partial copies a killed process left next to `new`. Called with
/// the migrate lock held.
fn remove_stale_tmp(new: &Path) {
    let (Some(dir), Some(name)) = (new.parent(), new.file_name()) else {
        return;
    };
    let prefix = format!("{}.tmp-", name.to_string_lossy());
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}

fn merge(step: &Move, tables_to_merge: &[&str]) -> Result<u64, String> {
    let mut conn = Connection::open(&step.new).map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(|e| e.to_string())?;
    conn.execute(
        "ATTACH DATABASE ?1 AS old",
        [step.old.to_string_lossy().as_ref()],
    )
    .map_err(|e| e.to_string())?;
    let old_tables = tables(&conn, "old").map_err(|e| e.to_string())?;
    // Deferred: an immediate transaction would also lock the attached old
    // file, which this process already holds a write lock on.
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .map_err(|e| e.to_string())?;
    // Every non-empty old table must be listed, or it would be dropped when
    // the old file is set aside.
    for t in &old_tables {
        if !tables_to_merge.contains(&t.as_str())
            && count(&tx, "old", t).map_err(|e| e.to_string())? > 0
        {
            return Err(format!(
                "old table {t} has rows but is not in the merge list"
            ));
        }
    }
    let mut inserted = 0u64;
    for table in tables_to_merge {
        if !old_tables.iter().any(|t| t == table) {
            continue;
        }
        let new_cols = columns(&tx, "main", table).map_err(|e| e.to_string())?;
        if new_cols.is_empty() {
            return Err(format!("table {table} is missing in the new file"));
        }
        if !has_primary_key(&tx, table)? {
            // Without a key INSERT OR IGNORE appends every row, so a repeat
            // after a crash would double count.
            return Err(format!("table {table} has no primary key, cannot merge"));
        }
        let old_cols = columns(&tx, "old", table).map_err(|e| e.to_string())?;
        let common: Vec<String> = new_cols
            .iter()
            .filter(|c| old_cols.contains(c))
            .map(|c| q(c))
            .collect();
        let list = common.join(", ");
        let n = tx
            .execute(
                &format!(
                    "INSERT OR IGNORE INTO main.{t} ({list}) SELECT {list} FROM old.{t}",
                    t = q(table)
                ),
                [],
            )
            .map_err(|e| e.to_string())?;
        inserted += n as u64;
        // Every old row must now exist in the new file, by the primary key.
        let missing: i64 = tx
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM (SELECT {list} FROM old.{t} EXCEPT SELECT {list} FROM main.{t})",
                    t = q(table)
                ),
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        // A row whose key exists with different content stays as the new
        // file has it (the newer write wins), so only rows whose key is
        // absent count as lost.
        if missing > 0 {
            let lost = lost_rows(&tx, table, &common)?;
            if lost > 0 {
                return Err(format!("{lost} rows of {table} were not merged"));
            }
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(inserted)
}

fn has_primary_key(conn: &Connection, table: &str) -> Result<bool, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA main.table_info({})", q(table)))
        .map_err(|e| e.to_string())?;
    let any = stmt
        .query_map([], |r| r.get::<_, i64>(5))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .any(|pk| pk > 0);
    Ok(any)
}

/// Old rows with no counterpart sharing the table's primary key.
fn lost_rows(conn: &Connection, table: &str, cols: &[String]) -> Result<i64, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA main.table_info({})", q(table)))
        .map_err(|e| e.to_string())?;
    let pks: Vec<String> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(5)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .filter(|(n, pk)| *pk > 0 && cols.contains(&q(n)))
        .map(|(n, _)| q(&n))
        .collect();
    if pks.is_empty() {
        return Ok(1);
    }
    let on = pks
        .iter()
        .map(|k| format!("o.{k} IS n.{k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM old.{t} o WHERE NOT EXISTS (SELECT 1 FROM main.{t} n WHERE {on})",
            t = q(table)
        ),
        [],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

/// Run one step. Takes a lock directory next to the new file so concurrent
/// processes serialize; the second finds the old file gone and does nothing.
pub fn migrate(step: &Move) -> Result<Outcome, String> {
    if !step.old.is_file() {
        return Ok(Outcome::Nothing);
    }
    let parent = step.new.parent().ok_or("the new path has no parent")?;
    ensure_private_dir(parent)
        .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    let lock = with_suffix(&step.new, ".migrate.lock");
    // Without the lock another process may be mid-move, so do nothing and let
    // the next run retry. Only the holder removes the lock directory.
    if !acquire_dir_lock_aged(&lock, LOCK_RETRIES, LOCK_DELAY, LONG_LOCK_STALE_AGE) {
        return Ok(Outcome::Nothing);
    }
    let result = run_locked(step);
    let _ = fs::remove_dir(&lock);
    result
}

fn run_locked(step: &Move) -> Result<Outcome, String> {
    if !step.old.is_file() {
        return Ok(Outcome::Nothing);
    }
    remove_stale_tmp(&step.new);
    let old_conn = match open_old(&step.old) {
        Ok(c) => c,
        Err(OldError::Corrupt(e)) => return Ok(Outcome::Corrupt(e)),
        Err(OldError::Transient(e)) => return Err(e),
    };
    // Held until the old file is set aside.
    let writer_lock = lock_old(&step.old)?;
    let outcome = if !step.new.exists() {
        copy_then_verify(step, &old_conn)?;
        Outcome::Moved
    } else {
        match step.policy {
            Policy::KeepNew => Outcome::SetAside,
            Policy::MergeTables(list) => {
                let inserted = merge(step, list)?;
                drop(old_conn);
                set_aside(&step.old)?;
                drop(writer_lock);
                return Ok(Outcome::Merged { inserted });
            }
        }
    };
    drop(old_conn);
    set_aside(&step.old)?;
    drop(writer_lock);
    Ok(outcome)
}

/// Print one notice line, only when stderr is a terminal. Hooks and pipes
/// have a captured stderr, so they stay silent.
pub fn notice(line: &str) {
    if std::io::stderr().is_terminal() {
        eprintln!("{line}");
    }
}

/// Run `step` and report a move or a skipped corrupt file with one line.
pub fn migrate_with_notice(step: &Move, what: &str) -> Result<Outcome, String> {
    let outcome = migrate(step)?;
    match &outcome {
        Outcome::Moved => notice(&format!(
            "playbook: moved the {what} database to {}",
            step.new.display()
        )),
        Outcome::Corrupt(e) => notice(&format!(
            "playbook: left the {what} database at {} in place ({e})",
            step.old.display()
        )),
        _ => {}
    }
    Ok(outcome)
}

/// One file in the data folder, for `playbook doctor`.
pub struct Entry {
    pub path: PathBuf,
    pub bytes: u64,
}

/// Every regular file under `dir`, sorted, with its size.
pub fn list_files(dir: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(p),
                Ok(t) if t.is_file() => {
                    let bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
                    out.push(Entry { path: p, bytes });
                }
                _ => {}
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Whether a path is a leftover from a finished migration.
pub fn is_migrated_leftover(path: &Path) -> bool {
    let p = path.to_string_lossy();
    p.contains(".migrated") || p.contains(".tmp-") || p.ends_with(".migrate.lock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;

    const T: &[&str] = &["t"];

    fn make(path: &Path, rows: &[(i64, &str)]) {
        let c = Connection::open(path).unwrap();
        c.pragma_update(None, "journal_mode", "WAL").unwrap();
        c.execute_batch("CREATE TABLE IF NOT EXISTS t (k INTEGER PRIMARY KEY, v TEXT NOT NULL)")
            .unwrap();
        for (k, v) in rows {
            c.execute("INSERT OR REPLACE INTO t VALUES (?1, ?2)", (k, v))
                .unwrap();
        }
    }

    fn rows(path: &Path) -> Vec<(i64, String)> {
        let c = Connection::open(path).unwrap();
        let mut s = c.prepare("SELECT k, v FROM t ORDER BY k").unwrap();
        s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    fn step(tag: &str, policy: Policy) -> (Move, PathBuf) {
        let dir = scratch_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        let m = Move {
            old: dir.join("old.db"),
            new: dir.join("data").join("new.db"),
            policy,
        };
        (m, dir)
    }

    #[test]
    fn data_dir_sits_under_the_playbook_root() {
        assert_eq!(
            data_dir_from(Path::new("/h")),
            PathBuf::from("/h/.config/playbook/data")
        );
    }

    #[test]
    fn none_present_does_nothing() {
        let (m, _d) = step("dm-none", Policy::MergeTables(T));
        assert_eq!(migrate(&m).unwrap(), Outcome::Nothing);
        assert!(!m.new.exists());
    }

    #[test]
    fn only_old_is_moved_and_set_aside_with_wal_content() {
        let (m, _d) = step("dm-old", Policy::MergeTables(T));
        make(&m.old, &[(1, "a"), (2, "b")]);
        // A live connection keeps rows in the WAL.
        let keep = Connection::open(&m.old).unwrap();
        keep.execute("INSERT INTO t VALUES (3, 'c')", []).unwrap();
        assert_eq!(migrate(&m).unwrap(), Outcome::Moved);
        drop(keep);
        assert_eq!(rows(&m.new).len(), 3);
        assert!(!m.old.exists());
        assert!(with_suffix(&m.old, ".migrated").exists());
    }

    #[test]
    fn only_new_is_untouched() {
        let (m, _d) = step("dm-new", Policy::MergeTables(T));
        fs::create_dir_all(m.new.parent().unwrap()).unwrap();
        make(&m.new, &[(1, "a")]);
        assert_eq!(migrate(&m).unwrap(), Outcome::Nothing);
        assert_eq!(rows(&m.new).len(), 1);
    }

    #[test]
    fn both_merge_without_double_counting_and_rerun_is_idempotent() {
        let (m, _d) = step("dm-both", Policy::MergeTables(T));
        fs::create_dir_all(m.new.parent().unwrap()).unwrap();
        make(&m.new, &[(1, "new"), (2, "b")]);
        make(&m.old, &[(1, "old"), (3, "c")]);
        assert_eq!(migrate(&m).unwrap(), Outcome::Merged { inserted: 1 });
        assert_eq!(
            rows(&m.new),
            vec![(1, "new".into()), (2, "b".into()), (3, "c".into())]
        );
        assert_eq!(migrate(&m).unwrap(), Outcome::Nothing);
        // An older binary writes the old path again: merged once, quietly.
        make(&m.old, &[(3, "c"), (4, "d")]);
        assert_eq!(migrate(&m).unwrap(), Outcome::Merged { inserted: 1 });
        assert_eq!(rows(&m.new).len(), 4);
        assert!(with_suffix(&m.old, ".migrated.1").exists());
    }

    #[test]
    fn corrupt_old_is_left_in_place() {
        let (m, _d) = step("dm-bad", Policy::MergeTables(T));
        fs::write(
            &m.old,
            b"this is not a database at all, just text padding.....",
        )
        .unwrap();
        assert!(matches!(migrate(&m).unwrap(), Outcome::Corrupt(_)));
        assert!(m.old.exists());
        assert!(!m.new.exists());
    }

    #[test]
    fn keep_new_sets_the_old_cache_aside() {
        let (m, _d) = step("dm-keep", Policy::KeepNew);
        fs::create_dir_all(m.new.parent().unwrap()).unwrap();
        make(&m.new, &[(1, "a")]);
        make(&m.old, &[(9, "z")]);
        assert_eq!(migrate(&m).unwrap(), Outcome::SetAside);
        assert_eq!(rows(&m.new), vec![(1, "a".into())]);
        assert!(!m.old.exists());
    }

    #[test]
    fn concurrent_writer_and_two_migrators_lose_nothing() {
        let (m, _d) = step("dm-conc", Policy::MergeTables(T));
        make(&m.old, &[(1, "a")]);
        let m2 = m.clone();
        let m3 = m.clone();
        let t1 = std::thread::spawn(move || migrate(&m2));
        let t2 = std::thread::spawn(move || migrate(&m3));
        let r1 = t1.join().unwrap().unwrap();
        let r2 = t2.join().unwrap().unwrap();
        let moved = [&r1, &r2].iter().filter(|r| ***r == Outcome::Moved).count();
        assert_eq!(moved, 1);
        assert_eq!(rows(&m.new), vec![(1, "a".into())]);
    }

    #[test]
    fn held_lock_makes_the_step_wait_and_do_nothing() {
        let (m, _d) = step("dm-held", Policy::MergeTables(T));
        make(&m.old, &[(1, "a")]);
        fs::create_dir_all(m.new.parent().unwrap()).unwrap();
        fs::create_dir(with_suffix(&m.new, ".migrate.lock")).unwrap();
        assert_eq!(migrate(&m).unwrap(), Outcome::Nothing);
        assert!(m.old.exists());
        assert!(!m.new.exists());
        // The lock that was not ours is still there.
        assert!(with_suffix(&m.new, ".migrate.lock").is_dir());
    }

    #[test]
    fn a_writer_holding_the_old_file_makes_the_move_fail_and_keep_it() {
        let (m, _d) = step("dm-writer", Policy::MergeTables(T));
        make(&m.old, &[(1, "a")]);
        let w = Connection::open(&m.old).unwrap();
        w.execute_batch("BEGIN IMMEDIATE; INSERT INTO t VALUES (2, 'b');")
            .unwrap();
        assert!(migrate(&m).is_err());
        assert!(m.old.exists());
        assert!(!m.new.exists());
        w.execute_batch("COMMIT").unwrap();
        drop(w);
        assert_eq!(migrate(&m).unwrap(), Outcome::Moved);
        assert_eq!(rows(&m.new).len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn moved_file_and_folder_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let (m, _d) = step("dm-perm", Policy::MergeTables(T));
        make(&m.old, &[(1, "a")]);
        migrate(&m).unwrap();
        let file = fs::metadata(&m.new).unwrap().permissions().mode() & 0o777;
        let dir = fs::metadata(m.new.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!((file, dir), (0o600, 0o700));
    }

    #[test]
    fn merge_refuses_a_table_without_a_primary_key_and_keeps_the_old_file() {
        let (m, _d) = step("dm-nopk", Policy::MergeTables(&["t"]));
        fs::create_dir_all(m.new.parent().unwrap()).unwrap();
        for p in [&m.old, &m.new] {
            let c = Connection::open(p).unwrap();
            c.execute_batch("CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('x');")
                .unwrap();
        }
        assert!(migrate(&m).is_err());
        assert!(m.old.exists());
    }

    #[test]
    fn merge_refuses_an_unlisted_non_empty_old_table() {
        let (m, _d) = step("dm-unlisted", Policy::MergeTables(T));
        fs::create_dir_all(m.new.parent().unwrap()).unwrap();
        make(&m.new, &[(1, "a")]);
        make(&m.old, &[(2, "b")]);
        Connection::open(&m.old)
            .unwrap()
            .execute_batch(
                "CREATE TABLE extra (k INTEGER PRIMARY KEY); INSERT INTO extra VALUES (1);",
            )
            .unwrap();
        assert!(migrate(&m).is_err());
        assert!(m.old.exists());
        assert_eq!(rows(&m.new).len(), 1);
    }

    #[test]
    fn sidecars_follow_the_renamed_main_file() {
        let (m, _d) = step("dm-side", Policy::KeepNew);
        fs::create_dir_all(m.new.parent().unwrap()).unwrap();
        make(&m.new, &[(1, "a")]);
        make(&m.old, &[(2, "b")]);
        fs::write(with_suffix(&m.old, "-wal"), b"").unwrap();
        migrate(&m).unwrap();
        assert!(with_suffix(&m.old, ".migrated").exists());
        assert!(with_suffix(&m.old, ".migrated-wal").exists());
    }

    #[test]
    fn a_stale_partial_copy_is_removed_before_the_move() {
        let (m, _d) = step("dm-tmp", Policy::MergeTables(T));
        make(&m.old, &[(1, "a")]);
        fs::create_dir_all(m.new.parent().unwrap()).unwrap();
        let stale = with_suffix(&m.new, ".tmp-99999");
        fs::write(&stale, b"partial").unwrap();
        migrate(&m).unwrap();
        assert!(!stale.exists());
    }

    #[test]
    fn list_files_reports_sizes_and_leftovers() {
        let d = scratch_dir("dm-list");
        fs::create_dir_all(d.join("x")).unwrap();
        fs::write(d.join("x").join("a.db"), b"12345").unwrap();
        fs::write(d.join("a.db.migrated"), b"1").unwrap();
        fs::write(d.join("b.db.tmp-1"), b"1").unwrap();
        let files = list_files(&d);
        assert_eq!(files.len(), 3);
        assert!(files.iter().any(|f| f.bytes == 5));
        assert!(files.iter().any(|f| is_migrated_leftover(&f.path)));
    }
}
