// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The usage store: one global SQLite database (not repo-scoped, since spend
//! is only meaningful as a total across projects). Connection setup mirrors
//! `gate::db::open_db`: the dashboard server reads while `ingest` writes from
//! other processes, which is what WAL mode and `busy_timeout` are for.

use super::{ToolInvocationEvent, ToolKind, UsageEvent};
use rusqlite::{params, Connection, OptionalExtension};
use std::fs;
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
    cost_usd REAL NOT NULL CHECK(cost_usd >= 0)
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
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create directory {}: {e}", parent.display()))?;
    }
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
    Ok(conn)
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
            cache_read_tokens, cost_usd)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
            e.cost_usd
        ],
    )
    .map(|_| ())
    .map_err(|e2| format!("failed to insert usage event {}: {e2}", e.event_id))
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

pub fn load_usage_events(conn: &Connection) -> Result<Vec<UsageEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT event_id, timestamp, session_id, account, model, effort, repo, branch,
                    input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens, cost_usd
             FROM usage_events ORDER BY timestamp, event_id",
        )
        .map_err(|e| format!("failed to prepare usage query: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(UsageEvent {
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
                cost_usd: r.get(12)?,
            })
        })
        .map_err(|e| format!("failed to read usage events: {e}"))?;
    rows.collect::<Result<_, _>>()
        .map_err(|e| format!("failed to read a usage event row: {e}"))
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
