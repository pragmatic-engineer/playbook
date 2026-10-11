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
    skill_name TEXT NOT NULL DEFAULT '',
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
    /// The skill active for the request, empty when none.
    pub skill_name: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
}

/// Creates the telemetry tables. Safe to run any number of times.
pub fn migrate_up(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(UP_SQL)
        .map_err(|e| format!("failed to create telemetry tables: {e}"))?;
    ensure_skill_column(conn)
}

/// Tables made before skill attribution have no `skill_name` column.
pub(super) fn ensure_skill_column(conn: &Connection) -> Result<(), String> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(telemetry_requests)")
        .map_err(|e| format!("failed to inspect telemetry tables: {e}"))?;
    let has = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| format!("failed to inspect telemetry tables: {e}"))?
        .any(|c| c.is_ok_and(|n| n == "skill_name"));
    if has {
        return Ok(());
    }
    conn.execute_batch(
        "ALTER TABLE telemetry_requests ADD COLUMN skill_name TEXT NOT NULL DEFAULT ''",
    )
    .map_err(|e| format!("failed to add the skill column: {e}"))
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
            effort, agent_name, skill_name, input_tokens, output_tokens,
            cache_creation_tokens, cache_read_tokens, cost_usd)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            r.session_id,
            r.request_id,
            r.timestamp,
            r.model,
            r.effort,
            r.agent_name,
            r.skill_name,
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

/// Telemetry requests in `[since, until)` grouped by agent name, sorted by
/// name. The main session (no agent name) is `main`.
pub fn agent_groups(
    conn: &Connection,
    since: i64,
    until: i64,
) -> Result<Vec<super::aggregate::Group>, String> {
    groups(conn, "agent_name", "main", since, until)
}

/// Telemetry requests in `[since, until)` grouped by the skill that was
/// active, sorted by name. Requests made with no skill active are left out.
pub fn skill_groups(
    conn: &Connection,
    since: i64,
    until: i64,
) -> Result<Vec<super::aggregate::Group>, String> {
    groups(conn, "skill_name", "", since, until)
}

/// `column` is one of two fixed names, never user input.
fn groups(
    conn: &Connection,
    column: &str,
    empty_as: &str,
    since: i64,
    until: i64,
) -> Result<Vec<super::aggregate::Group>, String> {
    let filter = if empty_as.is_empty() {
        format!("AND {column} <> ''")
    } else {
        String::new()
    };
    let sql = format!(
        "SELECT CASE WHEN {column} = '' THEN ?3 ELSE {column} END AS k,
                COUNT(*), SUM(input_tokens), SUM(output_tokens),
                SUM(cache_creation_tokens), SUM(cache_read_tokens), SUM(cost_usd)
         FROM telemetry_requests
         WHERE timestamp >= ?1 AND timestamp < ?2 {filter}
         GROUP BY k ORDER BY k"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("failed to read telemetry groups: {e}"))?;
    let rows = stmt
        .query_map(params![since, until, empty_as], |r| {
            let n = |i: usize| -> rusqlite::Result<u64> {
                Ok(u64::try_from(r.get::<_, i64>(i)?).unwrap_or(0))
            };
            Ok(super::aggregate::Group {
                key: r.get(0)?,
                messages: n(1)?,
                input_tokens: n(2)?,
                output_tokens: n(3)?,
                cache_creation_tokens: n(4)?,
                cache_read_tokens: n(5)?,
                cost_usd: r.get(6)?,
                unpriced: 0,
            })
        })
        .map_err(|e| format!("failed to read telemetry groups: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read telemetry groups: {e}"))
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
    #[test]
    fn agent_groups_sum_cost_per_agent_inside_the_window_and_name_the_main_session() {
        let conn = mem();
        let mut main = request("r1");
        main.agent_name = String::new();
        main.cost_usd = 1.0;
        let mut sub = request("r2");
        sub.agent_name = "reviewer".into();
        sub.cost_usd = 0.5;
        let mut sub2 = request("r3");
        sub2.agent_name = "reviewer".into();
        sub2.cost_usd = 0.25;
        let mut outside = request("r4");
        outside.timestamp = 10_000;
        for r in [&main, &sub, &sub2, &outside] {
            insert_request(&conn, r).unwrap();
        }
        let groups = agent_groups(&conn, 0, 1_000).unwrap();
        let names: Vec<&str> = groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(names, ["main", "reviewer"]);
        assert!((groups[1].cost_usd - 0.75).abs() < 1e-9);
        assert_eq!(groups[1].messages, 2);
        assert!(agent_groups(&conn, 5_000, 6_000).unwrap().is_empty());
    }

    #[test]
    fn skill_groups_sum_cost_per_skill_and_skip_requests_without_one() {
        let conn = mem();
        let mut a = request("r1");
        a.skill_name = "review".into();
        a.cost_usd = 0.5;
        let mut b = request("r2");
        b.skill_name = "review".into();
        b.cost_usd = 0.25;
        let mut c = request("r3");
        c.skill_name = "plan".into();
        let none = request("r4");
        for r in [&a, &b, &c, &none] {
            insert_request(&conn, r).unwrap();
        }
        let groups = skill_groups(&conn, 0, 1_000).unwrap();
        let names: Vec<&str> = groups.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(names, ["plan", "review"]);
        assert!((groups[1].cost_usd - 0.75).abs() < 1e-9);
        assert_eq!(groups[1].messages, 2);
    }

    #[test]
    fn a_table_made_before_skills_gains_the_column_and_keeps_its_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE telemetry_requests (
                session_id TEXT NOT NULL, request_id TEXT NOT NULL, timestamp INTEGER NOT NULL,
                model TEXT NOT NULL, effort TEXT NOT NULL DEFAULT '',
                agent_name TEXT NOT NULL DEFAULT '', input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL, cache_creation_tokens INTEGER NOT NULL,
                cache_read_tokens INTEGER NOT NULL, cost_usd REAL NOT NULL,
                PRIMARY KEY (session_id, request_id));
             INSERT INTO telemetry_requests VALUES ('s','r',1,'m','','',1,1,0,0,0.5);",
        )
        .unwrap();
        migrate_up(&conn).unwrap();
        migrate_up(&conn).unwrap();
        assert_eq!(count_requests(&conn).unwrap(), 1);
        assert!(skill_groups(&conn, 0, 10).unwrap().is_empty());
    }
}
