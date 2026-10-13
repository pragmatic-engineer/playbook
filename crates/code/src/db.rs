// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The code index database. It is a cache: a schema version mismatch or a
//! corrupt file is deleted and rebuilt by `index`, and query commands never
//! delete anything. Connection setup follows the usage store's ordering.

use super::host;
use playbook_common::sqlite::{retry_busy, OPEN_BUDGET};
use rusqlite::{Connection, ErrorCode};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Bumped when the table layout changes. A different non-zero value in
/// `PRAGMA user_version` makes the next `index` run rebuild the file.
pub const SCHEMA_VERSION: i64 = 2;

const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

/// How a caller opens the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    /// Used only by `index` after it holds the run lock: creates the file and
    /// deletes and recreates it once when it is corrupt or from another version.
    Index,
    /// Used by every other command: never creates or deletes anything.
    Query,
}

/// Why the index could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbError {
    /// No index file exists yet (query mode only).
    Missing,
    /// The file is corrupt or from another schema version and query mode does
    /// not delete it. The message names `--rebuild`.
    NeedsRebuild(String),
    /// Any other failure.
    Failed(String),
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Missing => write!(f, "no index yet; run `{}` first", host::command("index")),
            DbError::NeedsRebuild(msg) | DbError::Failed(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for DbError {}

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    lang TEXT,
    sha256 TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL,
    indexed_at INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('ok', 'partial', 'failed', 'skipped')),
    error TEXT
);
CREATE TABLE IF NOT EXISTS symbols (
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    parent_id INTEGER,
    name TEXT NOT NULL,
    qualified_name TEXT NOT NULL,
    kind TEXT NOT NULL,
    signature TEXT,
    signature_cut INTEGER NOT NULL DEFAULT 0,
    exported INTEGER NOT NULL DEFAULT 0,
    start_byte INTEGER NOT NULL,
    end_byte INTEGER NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS symbols_name_lower ON symbols(lower(name));
CREATE INDEX IF NOT EXISTS symbols_qualified ON symbols(qualified_name);
CREATE INDEX IF NOT EXISTS symbols_file ON symbols(file_id);
CREATE TABLE IF NOT EXISTS imports (
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    module TEXT NOT NULL,
    names TEXT,
    line INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS imports_file ON imports(file_id);
CREATE INDEX IF NOT EXISTS imports_module ON imports(module);
CREATE TABLE IF NOT EXISTS chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS chunks_file ON chunks(file_id);
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
    body,
    content='',
    contentless_delete=1,
    tokenize=\"unicode61 tokenchars '_'\"
);
";

/// Whether this build of SQLite was compiled with FTS5.
pub fn fts5_available(conn: &Connection) -> bool {
    let Ok(mut stmt) = conn.prepare("PRAGMA compile_options") else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) else {
        return false;
    };
    let options: Vec<String> = rows.flatten().collect();
    options.iter().any(|opt| opt == "ENABLE_FTS5")
}

/// The db file and the two files SQLite keeps beside it in WAL mode.
fn sibling_files(path: &Path) -> Vec<PathBuf> {
    let base = path.as_os_str().to_os_string();
    ["", "-wal", "-shm"]
        .iter()
        .map(|suffix| {
            let mut name = base.clone();
            name.push(suffix);
            PathBuf::from(name)
        })
        .collect()
}

/// Deletes the db, its WAL and its shm file. A missing file is fine.
pub fn delete_index_files(path: &Path) -> Result<(), String> {
    for file in sibling_files(path) {
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("failed to delete {}: {e}", file.display())),
        }
    }
    Ok(())
}

fn rebuild_hint(path: &Path, why: &str) -> String {
    format!(
        "the code index at {} {why}; run `{}`",
        path.display(),
        host::command("index --rebuild")
    )
}

/// Opens the index at `path` for `mode`. See [`OpenMode`].
pub fn open_index(path: &Path, mode: OpenMode) -> Result<Connection, DbError> {
    open_index_with(path, mode, delete_index_files)
}

/// [`open_index`] with the file deleter injected, so a test can make the
/// recreate step fail without relying on directory permissions.
fn open_index_with(
    path: &Path,
    mode: OpenMode,
    delete: impl Fn(&Path) -> Result<(), String>,
) -> Result<Connection, DbError> {
    if mode == OpenMode::Query && !path.exists() {
        return Err(DbError::Missing);
    }
    match try_open(path, mode) {
        Err(DbError::NeedsRebuild(first)) if mode == OpenMode::Index => {
            delete(path).map_err(|e| {
                DbError::Failed(format!("{e}; run `{}`", host::command("index --rebuild")))
            })?;
            try_open(path, mode).map_err(|second| {
                DbError::Failed(rebuild_hint(
                    path,
                    &format!("could not be recreated ({first}; then {second})"),
                ))
            })
        }
        other => other,
    }
}

fn is_corrupt(err: &rusqlite::Error) -> bool {
    matches!(
        err.sqlite_error_code(),
        Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt)
    )
}

fn step(path: &Path, what: &str, err: rusqlite::Error) -> DbError {
    if is_corrupt(&err) {
        DbError::NeedsRebuild(rebuild_hint(path, "is not a valid index database"))
    } else {
        DbError::Failed(format!(
            "failed to {what} for the code index at {}: {err}",
            path.display()
        ))
    }
}

fn try_open(path: &Path, mode: OpenMode) -> Result<Connection, DbError> {
    if mode == OpenMode::Index {
        if let Some(parent) = path.parent() {
            host::ensure_private_dir(parent).map_err(|e| {
                DbError::Failed(format!(
                    "failed to create directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
        host::make_file_private(path).map_err(DbError::Failed)?;
    }
    let conn = Connection::open(path).map_err(|e| step(path, "open the database", e))?;
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(|e| step(path, "set busy_timeout", e))?;
    retry_busy(
        || conn.pragma_update(None, "journal_mode", "WAL"),
        OPEN_BUDGET,
    )
    .map_err(|e| step(path, "set journal_mode=WAL", e))?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| step(path, "set foreign_keys", e))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| step(path, "set synchronous", e))?;
    if !fts5_available(&conn) {
        return Err(DbError::Failed(
            "this build has no SQLite FTS5; rebuild with FTS5 enabled".to_string(),
        ));
    }
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| step(path, "read user_version", e))?;
    if version == SCHEMA_VERSION {
        return Ok(conn);
    }
    if version != 0 {
        return Err(DbError::NeedsRebuild(rebuild_hint(
            path,
            &format!("has schema version {version}, this build uses {SCHEMA_VERSION}"),
        )));
    }
    if mode == OpenMode::Query {
        return Err(DbError::NeedsRebuild(rebuild_hint(path, "is empty")));
    }
    retry_busy(|| conn.execute_batch(SCHEMA_SQL), OPEN_BUDGET)
        .map_err(|e| step(path, "create the schema", e))?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|e| step(path, "set user_version", e))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use playbook_common::test_support::scratch_dir;

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .expect("count should run")
    }

    #[test]
    fn fts5_is_available_in_the_bundled_build() {
        // Arrange
        let conn = Connection::open_in_memory().expect("memory db");

        // Act
        let available = fts5_available(&conn);

        // Assert
        assert!(available, "bundled SQLite must have FTS5");
    }

    #[test]
    fn an_fts5_table_matches_the_row_it_holds() {
        // Arrange
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch("CREATE VIRTUAL TABLE t USING fts5(body);")
            .expect("fts5 table");
        conn.execute("INSERT INTO t(rowid, body) VALUES (7, 'hello world')", [])
            .expect("insert");

        // Act
        let hit: i64 = conn
            .query_row("SELECT rowid FROM t WHERE t MATCH 'hello'", [], |r| {
                r.get(0)
            })
            .expect("match");

        // Assert
        assert_eq!(hit, 7);
    }

    #[cfg(unix)]
    #[test]
    fn open_creates_a_private_dir_and_file_at_the_current_schema_version() {
        use std::os::unix::fs::PermissionsExt;
        // Arrange
        let dir = scratch_dir("code-db-modes");
        let path = dir.join("code").join("index.sqlite3");

        // Act
        let conn = open_index(&path, OpenMode::Index).expect("open");

        // Assert
        let dir_mode = std::fs::metadata(path.parent().expect("parent"))
            .expect("dir meta")
            .permissions()
            .mode()
            & 0o777;
        let file_mode = std::fs::metadata(&path)
            .expect("file meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("version");
        assert_eq!(version, SCHEMA_VERSION);
        let fk: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .expect("fk");
        assert_eq!(fk, 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_version_mismatch_rebuilds_and_drops_stray_tables() {
        // Arrange
        let dir = scratch_dir("code-db-version");
        let path = dir.join("index.sqlite3");
        std::fs::create_dir_all(&dir).expect("dir");
        {
            let conn = Connection::open(&path).expect("raw db");
            conn.execute_batch("CREATE TABLE stray(x); PRAGMA user_version = 99;")
                .expect("seed");
        }

        // Act
        let conn = open_index(&path, OpenMode::Index).expect("open");

        // Assert
        let stray: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'stray'",
                [],
                |r| r.get(0),
            )
            .expect("master");
        assert_eq!(stray, 0);
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .expect("version");
        assert_eq!(version, SCHEMA_VERSION);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn query_mode_on_a_missing_file_reports_missing_and_creates_nothing() {
        // Arrange
        let dir = scratch_dir("code-db-missing");
        let path = dir.join("nested").join("index.sqlite3");

        // Act
        let got = open_index(&path, OpenMode::Query);

        // Assert
        assert_eq!(got.err(), Some(DbError::Missing));
        assert!(!dir.exists(), "no directory may be created");
    }

    #[test]
    fn a_corrupt_file_rebuilds_in_index_mode_and_errors_in_query_mode() {
        // Arrange
        let dir = scratch_dir("code-db-corrupt");
        let path = dir.join("index.sqlite3");
        std::fs::create_dir_all(&dir).expect("dir");
        let garbage = vec![0x5Au8; 4096];
        std::fs::write(&path, &garbage).expect("garbage");

        // Act
        let query = open_index(&path, OpenMode::Query);

        // Assert: query mode names --rebuild and leaves the file alone.
        let message = query.expect_err("query must fail").to_string();
        assert!(message.contains("--rebuild"), "{message}");
        assert_eq!(std::fs::read(&path).expect("still there"), garbage);

        // Act
        let index = open_index(&path, OpenMode::Index).expect("index mode rebuilds");

        // Assert
        assert_eq!(count(&index, "files"), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_recreate_names_rebuild() {
        // Arrange: a corrupt db and a deleter that always fails.
        let dir = scratch_dir("code-db-undeletable");
        let path = dir.join("index.sqlite3");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(&path, vec![0x5Au8; 4096]).expect("garbage");

        // Act
        let got = open_index_with(&path, OpenMode::Index, |_| Err("denied".to_string()));

        // Assert
        let message = got.expect_err("must fail").to_string();
        assert!(message.contains("denied"), "{message}");
        assert!(message.contains("--rebuild"), "{message}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn deleting_a_file_row_cascades_to_symbols_imports_and_chunks() {
        // Arrange
        let dir = scratch_dir("code-db-cascade");
        let path = dir.join("index.sqlite3");
        let conn = open_index(&path, OpenMode::Index).expect("open");
        conn.execute_batch(
            "INSERT INTO files(id, path, sha256, size, mtime_ns, indexed_at, status)
                 VALUES (1, 'a.rs', 'h', 1, 1, 1, 'ok');
             INSERT INTO symbols(file_id, name, qualified_name, kind, start_byte, end_byte, start_line, end_line)
                 VALUES (1, 'f', 'f', 'function', 0, 1, 1, 1);
             INSERT INTO imports(file_id, module, names, line) VALUES (1, 'm', 'x', 1);
             INSERT INTO chunks(file_id, start_line, end_line) VALUES (1, 1, 1);",
        )
        .expect("seed");
        for table in ["symbols", "imports", "chunks"] {
            assert_eq!(count(&conn, table), 1, "{table} before");
        }

        // Act
        conn.execute("DELETE FROM files WHERE id = 1", [])
            .expect("delete");

        // Assert
        for table in ["symbols", "imports", "chunks"] {
            assert_eq!(count(&conn, table), 0, "{table} after");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_contentless_fts_row_is_deleted_by_rowid() {
        // Arrange
        let dir = scratch_dir("code-db-fts-delete");
        let conn = open_index(&dir.join("index.sqlite3"), OpenMode::Index).expect("open");
        conn.execute(
            "INSERT INTO chunks_fts(rowid, body) VALUES (5, 'create_payment')",
            [],
        )
        .expect("insert");
        let hit = |q: &str| -> i64 {
            conn.query_row(
                "SELECT COUNT(*) FROM chunks_fts WHERE chunks_fts MATCH ?1",
                [q],
                |r| r.get(0),
            )
            .expect("match")
        };
        assert_eq!(hit("\"create_payment\""), 1);

        // Act
        conn.execute("DELETE FROM chunks_fts WHERE rowid = 5", [])
            .expect("delete");

        // Assert
        assert_eq!(hit("\"create_payment\""), 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn open_in_query_mode_works_on_an_existing_index() {
        // Arrange
        let dir = scratch_dir("code-db-query");
        let path = dir.join("index.sqlite3");
        drop(open_index(&path, OpenMode::Index).expect("create"));

        // Act
        let got = open_index(&path, OpenMode::Query);

        // Assert
        assert!(got.is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }
}
