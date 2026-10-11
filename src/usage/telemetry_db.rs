// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Storage for OpenTelemetry usage rows. These tables sit next to the
//! transcript tables and never touch them: the transcript ingest, its tables
//! and its totals are unchanged. Reverting the feature means dropping the
//! tables (`migrate_down`), which loses no transcript data.
//!
//! Rows hold only numbers and short names (session id, request id, model,
//! effort, agent name). Prompts, tool content, paths, emails and user ids are
//! never stored; the receiver strips them before a row reaches this module.

use rusqlite::{params, Connection, OptionalExtension};

pub(super) const UP_SQL: &str = "
CREATE TABLE IF NOT EXISTS telemetry_requests (
    session_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    model TEXT NOT NULL,
    effort TEXT NOT NULL DEFAULT '',
    agent_name TEXT NOT NULL DEFAULT '',
    input_tokens INTEGER NOT NULL CHECK(input_tokens >= 0),
    output_tokens INTEGER NOT NULL CHECK(output_tokens >= 0),
    cache_creation_tokens INTEGER NOT NULL CHECK(cache_creation_tokens >= 0),
    cache_read_tokens INTEGER NOT NULL CHECK(cache_read_tokens >= 0),
    cost_usd REAL NOT NULL CHECK(cost_usd >= 0),
    PRIMARY KEY (session_id, request_id)
);
CREATE INDEX IF NOT EXISTS telemetry_requests_by_time
    ON telemetry_requests (timestamp, session_id, request_id);
CREATE TABLE IF NOT EXISTS session_sources (
    session_id TEXT PRIMARY KEY,
    source TEXT NOT NULL CHECK(source IN ('telemetry','transcript')),
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS session_reconcile (
    session_id TEXT PRIMARY KEY,
    state TEXT NOT NULL CHECK(state IN ('complete','partial','disagree')),
    telemetry_tokens INTEGER NOT NULL,
    transcript_tokens INTEGER NOT NULL,
    telemetry_cost REAL NOT NULL,
    transcript_cost REAL NOT NULL,
    note TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);";

const DOWN_SQL: &str = "
DROP INDEX IF EXISTS telemetry_requests_by_time;
DROP TABLE IF EXISTS telemetry_requests;
DROP TABLE IF EXISTS session_sources;
DROP TABLE IF EXISTS session_reconcile;";

/// Where a session's numbers come from. A session with no recorded source is
/// read from transcripts, so history before telemetry needs no backfill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Telemetry,
    Transcript,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Telemetry => "telemetry",
            Source::Transcript => "transcript",
        }
    }

    fn parse(text: &str) -> Source {
        if text == "telemetry" {
            Source::Telemetry
        } else {
            Source::Transcript
        }
    }
}

/// One API request as reported by telemetry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TelemetryRequest {
    pub session_id: String,
    pub request_id: String,
    pub timestamp: i64,
    pub model: String,
    pub effort: String,
    pub agent_name: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
}

/// Creates the telemetry tables. Safe to run any number of times.
pub fn migrate_up(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(UP_SQL)
        .map_err(|e| format!("failed to create telemetry tables: {e}"))
}

/// Drops the telemetry tables and nothing else.
pub fn migrate_down(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(DOWN_SQL)
        .map_err(|e| format!("failed to drop telemetry tables: {e}"))
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Stores a request. The pair (session id, request id) is the key, so a
/// redelivered batch is a no-op. Returns 1 when a row was written, else 0.
pub fn insert_request(conn: &Connection, r: &TelemetryRequest) -> Result<usize, String> {
    let cost = if r.cost_usd.is_finite() && r.cost_usd >= 0.0 {
        r.cost_usd
    } else {
        0.0
    };
    conn.execute(
        "INSERT OR IGNORE INTO telemetry_requests (session_id, request_id, timestamp, model,
            effort, agent_name, input_tokens, output_tokens, cache_creation_tokens,
            cache_read_tokens, cost_usd)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            r.session_id,
            r.request_id,
            r.timestamp,
            r.model,
            r.effort,
            r.agent_name,
            to_i64(r.input_tokens),
            to_i64(r.output_tokens),
            to_i64(r.cache_creation_tokens),
            to_i64(r.cache_read_tokens),
            cost
        ],
    )
    .map_err(|e| format!("failed to insert telemetry request {}: {e}", r.request_id))
}

pub fn count_requests(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM telemetry_requests", [], |r| r.get(0))
        .map_err(|e| format!("failed to count telemetry requests: {e}"))
}

/// Records which source a session uses.
pub fn set_session_source(
    conn: &Connection,
    session_id: &str,
    source: Source,
    now: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO session_sources (session_id, source, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(session_id) DO UPDATE SET source = excluded.source,
            updated_at = excluded.updated_at",
        params![session_id, source.as_str(), now],
    )
    .map(|_| ())
    .map_err(|e| format!("failed to set the source of {session_id}: {e}"))
}

/// The source of a session; transcripts when none was recorded.
pub fn session_source(conn: &Connection, session_id: &str) -> Result<Source, String> {
    conn.query_row(
        "SELECT source FROM session_sources WHERE session_id = ?1",
        [session_id],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map(|found| found.map_or(Source::Transcript, |s| Source::parse(&s)))
    .map_err(|e| format!("failed to read the source of {session_id}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::db::{count_usage_events, open_db, upsert_usage_event};
    use crate::usage::UsageEvent;

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate_up(&conn).unwrap();
        conn
    }

    fn request(id: &str) -> TelemetryRequest {
        TelemetryRequest {
            session_id: "s1".into(),
            request_id: id.into(),
            timestamp: 100,
            model: "opus".into(),
            agent_name: "reviewer".into(),
            input_tokens: 10,
            output_tokens: 5,
            cost_usd: 0.25,
            ..TelemetryRequest::default()
        }
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [name],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn inserting_the_same_request_twice_stores_one_row() {
        let conn = mem();
        assert_eq!(insert_request(&conn, &request("r1")).unwrap(), 1);
        assert_eq!(insert_request(&conn, &request("r1")).unwrap(), 0);
        assert_eq!(count_requests(&conn).unwrap(), 1);
    }

    #[test]
    fn the_same_request_id_in_two_sessions_is_two_rows() {
        let conn = mem();
        let mut other = request("r1");
        other.session_id = "s2".into();
        insert_request(&conn, &request("r1")).unwrap();
        insert_request(&conn, &other).unwrap();
        assert_eq!(count_requests(&conn).unwrap(), 2);
    }

    #[test]
    fn a_bad_cost_is_stored_as_zero() {
        let conn = mem();
        let mut r = request("r1");
        r.cost_usd = f64::NAN;
        insert_request(&conn, &r).unwrap();
        let cost: f64 = conn
            .query_row("SELECT cost_usd FROM telemetry_requests", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn a_session_without_a_record_reads_from_transcripts() {
        let conn = mem();
        assert_eq!(session_source(&conn, "none").unwrap(), Source::Transcript);
    }

    #[test]
    fn the_session_source_can_change_and_is_kept_per_session() {
        let conn = mem();
        set_session_source(&conn, "s1", Source::Telemetry, 1).unwrap();
        set_session_source(&conn, "s2", Source::Transcript, 1).unwrap();
        assert_eq!(session_source(&conn, "s1").unwrap(), Source::Telemetry);
        set_session_source(&conn, "s1", Source::Transcript, 2).unwrap();
        assert_eq!(session_source(&conn, "s1").unwrap(), Source::Transcript);
        assert_eq!(session_source(&conn, "s2").unwrap(), Source::Transcript);
    }

    #[test]
    fn the_source_column_rejects_an_unknown_value() {
        let conn = mem();
        let err = conn.execute("INSERT INTO session_sources VALUES ('s', 'guess', 1)", []);
        assert!(err.is_err());
    }

    #[test]
    fn migrate_up_is_idempotent_and_down_removes_only_telemetry_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE usage_events (event_id TEXT);")
            .unwrap();
        migrate_up(&conn).unwrap();
        migrate_up(&conn).unwrap();
        insert_request(&conn, &request("r1")).unwrap();
        assert!(table_exists(&conn, "telemetry_requests"));

        migrate_down(&conn).unwrap();
        migrate_down(&conn).unwrap();

        assert!(!table_exists(&conn, "telemetry_requests"));
        assert!(!table_exists(&conn, "session_sources"));
        assert!(!table_exists(&conn, "session_reconcile"));
        assert!(table_exists(&conn, "usage_events"));
        migrate_up(&conn).unwrap();
        assert_eq!(count_requests(&conn).unwrap(), 0);
    }

    #[test]
    fn opening_the_usage_db_creates_the_tables_and_leaves_transcript_ingest_alone() {
        let dir = crate::common::test_support::scratch_dir("telemetry-open");
        let conn = open_db(&dir.join("usage.db")).unwrap();
        assert!(table_exists(&conn, "telemetry_requests"));
        let event = UsageEvent {
            event_id: "m1".into(),
            session_id: "s1".into(),
            model: "opus".into(),
            input_tokens: 3,
            ..UsageEvent::default()
        };
        upsert_usage_event(&conn, &event).unwrap();
        upsert_usage_event(&conn, &event).unwrap();
        assert_eq!(count_usage_events(&conn).unwrap(), 1);
        assert_eq!(count_requests(&conn).unwrap(), 0);
        assert_eq!(session_source(&conn, "s1").unwrap(), Source::Transcript);
    }
}
