// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The usage store: one global SQLite database (not repo-scoped, since spend
//! is only meaningful as a total across projects). Connection setup mirrors
//! `gate::db::open_db`: the dashboard server reads while `ingest` writes from
//! other processes, which is what WAL mode and `busy_timeout` are for.

use super::{ToolInvocationEvent, ToolKind, UsageEvent};
use crate::common::atomic::ensure_private_dir;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::time::Duration;

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS usage_events (
    event_id TEXT PRIMARY KEY,
    timestamp INTEGER NOT NULL,
    session_id TEXT NOT NULL,
    account TEXT NOT NULL,
    model TEXT NOT NULL,
    effort TEXT NOT NULL,
    repo TEXT NOT NULL,
    branch TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_creation_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cost_usd REAL NOT NULL CHECK(cost_usd >= 0),
    cache_creation_1h_tokens INTEGER NOT NULL DEFAULT 0,
    agent TEXT NOT NULL DEFAULT 'claude-code'
);
CREATE TABLE IF NOT EXISTS tool_invocation_events (
    event_id TEXT PRIMARY KEY,
    timestamp INTEGER NOT NULL,
    session_id TEXT NOT NULL,
    account TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('skill','agent')),
    name TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS usage_watermarks (
    source TEXT PRIMARY KEY,
    watermark INTEGER NOT NULL
);";

const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);

/// Open (creating if missing) the usage database at `path`.
pub fn open_db(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)
            .map_err(|e| format!("failed to create directory {}: {e}", parent.display()))?;
    }
    make_db_file_private(path)?;
    let conn = Connection::open(path)
        .map_err(|e| format!("failed to open usage database at {}: {e}", path.display()))?;
    // Before any statement that can contend for the write lock; a fresh
    // connection's default timeout is 0, which fails instead of retrying.
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(|e| format!("failed to set busy_timeout: {e}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("failed to set journal_mode=WAL: {e}"))?;
    conn.execute_batch(SCHEMA_SQL)
        .map_err(|e| format!("failed to create usage schema: {e}"))?;
    ensure_cache_1h_column(&conn)?;
    ensure_agent_column(&conn)?;
    Ok(conn)
}

/// The data names the account, repos, and branches, so the file is owner-only
/// on unix. Created first so SQLite never makes it with the default umask, and
/// the WAL and shm files copy this mode. An older, wider file is tightened.
fn make_db_file_private(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let error = |e: std::io::Error| format!("failed to secure {}: {e}", path.display());
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .map_err(error)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(error)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Adds `cache_creation_1h_tokens` to a database created before it existed.
/// A concurrent caller winning the race is harmless ("duplicate column").
fn ensure_cache_1h_column(conn: &Connection) -> Result<(), String> {
    let has: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('usage_events')
                           WHERE name = 'cache_creation_1h_tokens')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("failed to inspect usage_events: {e}"))?;
    if has {
        return Ok(());
    }
    match conn.execute(
        "ALTER TABLE usage_events ADD COLUMN cache_creation_1h_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    ) {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("duplicate column") => Ok(()),
        Err(e) => Err(format!("failed to add cache_creation_1h_tokens: {e}")),
    }
}

/// Adds `agent` to a database created before it existed; old rows are Claude Code.
fn ensure_agent_column(conn: &Connection) -> Result<(), String> {
    let has: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('usage_events') WHERE name = 'agent')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("failed to inspect usage_events: {e}"))?;
    if has {
        return Ok(());
    }
    match conn.execute(
        "ALTER TABLE usage_events ADD COLUMN agent TEXT NOT NULL DEFAULT 'claude-code'",
        [],
    ) {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("duplicate column") => Ok(()),
        Err(e) => Err(format!("failed to add agent: {e}")),
    }
}

/// Last ingested timestamp for `source`; 0 when it has never been ingested.
pub fn get_watermark(conn: &Connection, source: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT watermark FROM usage_watermarks WHERE source = ?1",
        [source],
        |row| row.get(0),
    )
    .optional()
    .map(|found| found.unwrap_or(0))
    .map_err(|e| format!("failed to read watermark for {source}: {e}"))
}

pub fn advance_watermark(conn: &Connection, source: &str, watermark: i64) -> Result<(), String> {
    conn.execute(
        "INSERT INTO usage_watermarks (source, watermark) VALUES (?1, ?2)
         ON CONFLICT(source) DO UPDATE SET watermark = excluded.watermark",
        params![source, watermark],
    )
    .map(|_| ())
    .map_err(|e| format!("failed to advance watermark for {source}: {e}"))
}

/// Like `advance_watermark`, but never moves it backwards: a slower ingest
/// that read an older watermark must not undo a newer one.
pub fn raise_watermark(conn: &Connection, source: &str, watermark: i64) -> Result<(), String> {
    conn.execute(
        "INSERT INTO usage_watermarks (source, watermark) VALUES (?1, ?2)
         ON CONFLICT(source) DO UPDATE SET watermark = MAX(watermark, excluded.watermark)",
        params![source, watermark],
    )
    .map(|_| ())
    .map_err(|e| format!("failed to raise watermark for {source}: {e}"))
}

fn exists(conn: &Connection, table: &str, event_id: &str) -> Result<bool, String> {
    conn.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE event_id = ?1)"),
        [event_id],
        |row| row.get(0),
    )
    .map_err(|e| format!("failed to look up {event_id} in {table}: {e}"))
}

pub fn has_usage_event(conn: &Connection, event_id: &str) -> Result<bool, String> {
    exists(conn, "usage_events", event_id)
}

pub fn has_tool_event(conn: &Connection, event_id: &str) -> Result<bool, String> {
    exists(conn, "tool_invocation_events", event_id)
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub fn insert_usage_event(conn: &Connection, e: &UsageEvent) -> Result<(), String> {
    conn.execute(
        "INSERT INTO usage_events (event_id, timestamp, session_id, account, model, effort,
            repo, branch, input_tokens, output_tokens, cache_creation_tokens,
            cache_read_tokens, cost_usd, cache_creation_1h_tokens, agent)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            e.event_id,
            e.timestamp,
            e.session_id,
            e.account,
            e.model,
            e.effort,
            e.repo,
            e.branch,
            to_i64(e.input_tokens),
            to_i64(e.output_tokens),
            to_i64(e.cache_creation_tokens),
            to_i64(e.cache_read_tokens),
            e.cost_usd,
            to_i64(e.cache_creation_1h_tokens),
            e.agent
        ],
    )
    .map(|_| ())
    .map_err(|e2| format!("failed to insert usage event {}: {e2}", e.event_id))
}

/// Inserts the event, or when its id is already stored raises the token
/// counts to the larger values. A message is written as several lines whose
/// output count can grow, so a poll that saw it early must not freeze a
/// partial count. Returns the number of rows written (0 when nothing grew).
pub fn upsert_usage_event(conn: &Connection, e: &UsageEvent) -> Result<usize, String> {
    conn.execute(
        "INSERT INTO usage_events (event_id, timestamp, session_id, account, model, effort,
            repo, branch, input_tokens, output_tokens, cache_creation_tokens,
            cache_read_tokens, cost_usd, cache_creation_1h_tokens, agent)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
         ON CONFLICT(event_id) DO UPDATE SET
            input_tokens = excluded.input_tokens,
            output_tokens = excluded.output_tokens,
            cache_creation_tokens = excluded.cache_creation_tokens,
            cache_read_tokens = excluded.cache_read_tokens,
            cost_usd = excluded.cost_usd,
            cache_creation_1h_tokens = excluded.cache_creation_1h_tokens
         WHERE excluded.output_tokens > usage_events.output_tokens",
        params![
            e.event_id,
            e.timestamp,
            e.session_id,
            e.account,
            e.model,
            e.effort,
            e.repo,
            e.branch,
            to_i64(e.input_tokens),
            to_i64(e.output_tokens),
            to_i64(e.cache_creation_tokens),
            to_i64(e.cache_read_tokens),
            e.cost_usd,
            to_i64(e.cache_creation_1h_tokens),
            e.agent
        ],
    )
    .map_err(|e2| format!("failed to upsert usage event {}: {e2}", e.event_id))
}

pub fn count_usage_events(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM usage_events", [], |row| row.get(0))
        .map_err(|e| format!("failed to count usage events: {e}"))
}

/// Corrects the two fields an older ingest got wrong. Returns rows changed.
pub fn set_repo_and_cache_tier(
    conn: &Connection,
    event_id: &str,
    repo: &str,
    cache_creation_1h_tokens: u64,
) -> Result<usize, String> {
    conn.execute(
        "UPDATE usage_events SET repo = ?2, cache_creation_1h_tokens = ?3 WHERE event_id = ?1",
        params![event_id, repo, to_i64(cache_creation_1h_tokens)],
    )
    .map_err(|e| format!("failed to update usage event {event_id}: {e}"))
}

pub fn insert_tool_event(conn: &Connection, e: &ToolInvocationEvent) -> Result<(), String> {
    conn.execute(
        "INSERT INTO tool_invocation_events (event_id, timestamp, session_id, account, kind, name)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            e.event_id,
            e.timestamp,
            e.session_id,
            e.account,
            e.kind.as_str(),
            e.name
        ],
    )
    .map(|_| ())
    .map_err(|e2| format!("failed to insert tool event {}: {e2}", e.event_id))
}

const USAGE_COLUMNS: &str = "event_id, timestamp, session_id, account, model, effort, repo, branch,
    input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens,
    cache_creation_1h_tokens, agent";

fn query_usage(
    conn: &Connection,
    tail: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<UsageEvent>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {USAGE_COLUMNS} FROM usage_events {tail}"))
        .map_err(|e| format!("failed to prepare usage query: {e}"))?;
    let rows = stmt
        .query_map(params, |r| {
            let mut event = UsageEvent {
                event_id: r.get(0)?,
                timestamp: r.get(1)?,
                session_id: r.get(2)?,
                account: r.get(3)?,
                model: r.get(4)?,
                effort: r.get(5)?,
                repo: r.get(6)?,
                branch: r.get(7)?,
                input_tokens: r.get::<_, i64>(8)?.max(0) as u64,
                output_tokens: r.get::<_, i64>(9)?.max(0) as u64,
                cache_creation_tokens: r.get::<_, i64>(10)?.max(0) as u64,
                cache_read_tokens: r.get::<_, i64>(11)?.max(0) as u64,
                cache_creation_1h_tokens: r.get::<_, i64>(12)?.max(0) as u64,
                agent: r.get(13)?,
                ..UsageEvent::default()
            };
            event.apply_pricing();
            Ok(event)
        })
        .map_err(|e| format!("failed to read usage events: {e}"))?;
    rows.collect::<Result<_, _>>()
        .map_err(|e| format!("failed to read a usage event row: {e}"))
}

/// Loads every usage event, pricing each one now from its stored token counts
/// and the current price table. The stored `cost_usd` column is ignored, so a
/// price correction never needs a re-ingest.
pub fn load_usage_events(conn: &Connection) -> Result<Vec<UsageEvent>, String> {
    query_usage(conn, "ORDER BY timestamp, event_id", &[])
}

/// Events at or after `since` (epoch seconds), oldest first.
pub fn load_usage_events_since(conn: &Connection, since: i64) -> Result<Vec<UsageEvent>, String> {
    query_usage(
        conn,
        "WHERE timestamp >= ?1 ORDER BY timestamp, event_id",
        &[&since],
    )
}

/// The newest `limit` events, newest first.
pub fn load_latest_usage_events(conn: &Connection, limit: i64) -> Result<Vec<UsageEvent>, String> {
    query_usage(
        conn,
        "ORDER BY timestamp DESC, event_id DESC LIMIT ?1",
        &[&limit],
    )
}

/// Every event of the given sessions, oldest first.
pub fn load_usage_events_of_sessions(
    conn: &Connection,
    sessions: &[String],
) -> Result<Vec<UsageEvent>, String> {
    if sessions.is_empty() {
        return Ok(Vec::new());
    }
    let marks = vec!["?"; sessions.len()].join(",");
    let params: Vec<&dyn rusqlite::ToSql> =
        sessions.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    query_usage(
        conn,
        &format!("WHERE session_id IN ({marks}) ORDER BY timestamp, event_id"),
        &params,
    )
}

pub fn load_tool_events(conn: &Connection) -> Result<Vec<ToolInvocationEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT event_id, timestamp, session_id, account, kind, name
             FROM tool_invocation_events ORDER BY timestamp, event_id",
        )
        .map_err(|e| format!("failed to prepare tool query: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            let kind: String = r.get(4)?;
            Ok(ToolInvocationEvent {
                event_id: r.get(0)?,
                timestamp: r.get(1)?,
                session_id: r.get(2)?,
                account: r.get(3)?,
                kind: if kind == "skill" {
                    ToolKind::Skill
                } else {
                    ToolKind::Agent
                },
                name: r.get(5)?,
            })
        })
        .map_err(|e| format!("failed to read tool events: {e}"))?;
    rows.collect::<Result<_, _>>()
        .map_err(|e| format!("failed to read a tool event row: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::fs;

    #[cfg(unix)]
    #[test]
    fn the_database_and_its_directory_are_owner_only_and_an_older_file_is_tightened() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;

        // Arrange: a database from before this change, readable by others.
        let dir = scratch_dir("usage-private").join("usage");
        let path = dir.join("usage.db");
        drop(open_db(&path).unwrap());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        // Act
        let conn = open_db(&path).unwrap();
        conn.execute("INSERT INTO usage_watermarks VALUES ('s', 1)", [])
            .unwrap();

        // Assert
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&path), 0o600);
        let wal = dir.join("usage.db-wal");
        assert!(
            wal.exists() && mode(&wal) & 0o077 == 0,
            "the WAL must be private too"
        );
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn open_creates_all_three_tables_in_wal_mode() {
        let dir = scratch_dir("usage-open");
        let conn = open_db(&dir.join("usage.db")).unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();

        assert_eq!(
            tables,
            vec!["tool_invocation_events", "usage_events", "usage_watermarks"]
        );
        assert_eq!(mode, "wal");
        let _ = fs::remove_dir_all(dir);
    }

    fn event(id: &str, model: &str, stored_cost: f64) -> UsageEvent {
        UsageEvent {
            event_id: id.into(),
            timestamp: 100,
            session_id: "s".into(),
            account: "a".into(),
            model: model.into(),
            input_tokens: 1_000_000,
            cost_usd: stored_cost,
            ..UsageEvent::default()
        }
    }

    #[test]
    fn load_prices_from_tokens_and_ignores_the_stored_cost() {
        let dir = scratch_dir("usage-read-pricing");
        let conn = open_db(&dir.join("usage.db")).unwrap();
        insert_usage_event(&conn, &event("a", "claude-sonnet-5", 99.0)).unwrap();
        insert_usage_event(&conn, &event("b", "mystery-model", 99.0)).unwrap();

        let loaded = load_usage_events(&conn).unwrap();

        // Sonnet 5 input is $2 per million; the stored 99.0 is not used.
        assert!((loaded[0].cost_usd - 2.0).abs() < 1e-9 && !loaded[0].unpriced);
        assert_eq!((loaded[1].cost_usd, loaded[1].unpriced), (0.0, true));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_database_made_before_the_cache_tier_column_gains_it_with_zero() {
        let dir = scratch_dir("usage-old-schema");
        let path = dir.join("usage.db");
        fs::create_dir_all(&dir).unwrap();
        let old = Connection::open(&path).unwrap();
        old.execute_batch(
            "CREATE TABLE usage_events (
                event_id TEXT PRIMARY KEY, timestamp INTEGER NOT NULL,
                session_id TEXT NOT NULL, account TEXT NOT NULL, model TEXT NOT NULL,
                effort TEXT NOT NULL, repo TEXT NOT NULL, branch TEXT NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                cache_creation_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
                cost_usd REAL NOT NULL CHECK(cost_usd >= 0));
             INSERT INTO usage_events VALUES
                ('old', 1, 's', 'a', 'claude-sonnet-5', '', 'r', 'b', 1, 2, 3, 4, 0.5);",
        )
        .unwrap();
        drop(old);

        let conn = open_db(&path).unwrap();
        let again = open_db(&path).unwrap();

        let loaded = load_usage_events(&conn).unwrap();
        assert_eq!(loaded[0].cache_creation_1h_tokens, 0);
        assert_eq!(loaded[0].cache_creation_tokens, 3);
        assert_eq!(
            loaded[0].agent, "claude-code",
            "old rows default to Claude Code"
        );
        assert_eq!(count_usage_events(&again).unwrap(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn watermark_defaults_to_zero_and_only_moves_when_advanced() {
        let dir = scratch_dir("usage-watermark");
        let conn = open_db(&dir.join("usage.db")).unwrap();

        assert_eq!(get_watermark(&conn, "claude-code").unwrap(), 0);
        advance_watermark(&conn, "claude-code", 500).unwrap();
        assert_eq!(get_watermark(&conn, "claude-code").unwrap(), 500);
        assert_eq!(get_watermark(&conn, "other").unwrap(), 0);
        let _ = fs::remove_dir_all(dir);
    }
}
