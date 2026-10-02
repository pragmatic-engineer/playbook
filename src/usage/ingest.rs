// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Incremental ingest: read events newer than the source's watermark, tag the
//! account, write them, and advance the watermark, all in one transaction. A
//! failure anywhere leaves both the rows and the watermark untouched.

use super::db;
use super::UsageSource;
use rusqlite::{Connection, Transaction, TransactionBehavior};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct IngestStats {
    pub usage_inserted: usize,
    pub tools_inserted: usize,
    pub duplicates_skipped: usize,
}

/// Dedup is an app-level pre-filter on `event_id`, so re-reading events after
/// a crash that lost the watermark never errors and never double-counts.
pub fn ingest(
    source: &dyn UsageSource,
    account: &str,
    conn: &Connection,
) -> Result<IngestStats, String> {
    let name = source.name();
    let watermark = db::get_watermark(conn, name)?;
    let events = source.events_since(watermark)?;

    // IMMEDIATE takes the write lock up front, so a second ingest waits out
    // `busy_timeout` instead of failing with "database is locked" when its
    // stale read snapshot cannot upgrade to a write. It also makes the
    // event_id pre-filter below race-free.
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)
        .map_err(|e| format!("failed to begin ingest transaction: {e}"))?;
    let mut stats = IngestStats::default();
    let mut newest = watermark;

    for event in events.usage {
        newest = newest.max(event.timestamp);
        if db::has_usage_event(&tx, &event.event_id)? {
            stats.duplicates_skipped += 1;
            continue;
        }
        let mut event = event;
        event.account = account.to_string();
        db::insert_usage_event(&tx, &event)?;
        stats.usage_inserted += 1;
    }
    for event in events.tools {
        newest = newest.max(event.timestamp);
        if db::has_tool_event(&tx, &event.event_id)? {
            stats.duplicates_skipped += 1;
            continue;
        }
        let mut event = event;
        event.account = account.to_string();
        db::insert_tool_event(&tx, &event)?;
        stats.tools_inserted += 1;
    }

    db::raise_watermark(&tx, name, newest)?;
    tx.commit()
        .map_err(|e| format!("failed to commit ingest transaction: {e}"))?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use crate::usage::claude_code::ClaudeCodeSource;
    use crate::usage::{Events, ToolKind, UsageEvent};
    use std::fs;
    use std::path::PathBuf;

    fn fixture_root(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/usage")
            .join(name)
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    struct Fake(Events);
    impl UsageSource for Fake {
        fn name(&self) -> &str {
            "fake"
        }
        fn events_since(&self, _watermark: i64) -> Result<Events, String> {
            Ok(self.0.clone())
        }
    }

    fn usage(id: &str, ts: i64, cost: f64) -> UsageEvent {
        UsageEvent {
            event_id: id.into(),
            timestamp: ts,
            session_id: "s".into(),
            account: String::new(),
            model: "claude-sonnet-5".into(),
            effort: String::new(),
            repo: "r".into(),
            branch: "b".into(),
            input_tokens: 1,
            output_tokens: 1,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            cost_usd: cost,
            ..UsageEvent::default()
        }
    }

    #[test]
    fn ingest_writes_events_tags_the_account_and_advances_the_watermark() {
        let dir = scratch_dir("usage-ingest");
        let conn = db::open_db(&dir.join("usage.db")).unwrap();
        let source = ClaudeCodeSource::new(fixture_root("usage"));

        let stats = ingest(&source, "dev@example.com", &conn).unwrap();

        assert_eq!(stats.usage_inserted, 3);
        let rows = db::load_usage_events(&conn).unwrap();
        assert!(rows.iter().all(|r| r.account == "dev@example.com"));
        assert_eq!(db::get_watermark(&conn, "claude-code").unwrap(), 1788307201);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_lost_watermark_reingests_the_same_events_without_double_counting() {
        let dir = scratch_dir("usage-dedup");
        let conn = db::open_db(&dir.join("usage.db")).unwrap();
        let source = ClaudeCodeSource::new(fixture_root("tools"));
        ingest(&source, "a", &conn).unwrap();

        // Simulate a crash that lost the watermark: the source re-returns
        // every event, so only the event_id pre-filter prevents doubles.
        db::advance_watermark(&conn, "claude-code", 0).unwrap();
        let again = ingest(&source, "a", &conn).unwrap();

        assert_eq!(count(&conn, "tool_invocation_events"), 4);
        assert_eq!(count(&conn, "usage_events"), 4);
        assert_eq!(again.tools_inserted + again.usage_inserted, 0);
        assert_eq!(again.duplicates_skipped, 8);
        let tools = db::load_tool_events(&conn).unwrap();
        assert_eq!(tools[0].kind, ToolKind::Skill);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_slower_ingest_cannot_move_the_watermark_backwards() {
        let dir = scratch_dir("usage-raise");
        let conn = db::open_db(&dir.join("usage.db")).unwrap();
        db::advance_watermark(&conn, "fake", 900).unwrap();

        db::raise_watermark(&conn, "fake", 100).unwrap();
        assert_eq!(db::get_watermark(&conn, "fake").unwrap(), 900);
        db::raise_watermark(&conn, "fake", 1200).unwrap();
        assert_eq!(db::get_watermark(&conn, "fake").unwrap(), 1200);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failure_mid_batch_rolls_back_rows_and_leaves_the_watermark() {
        let dir = scratch_dir("usage-rollback");
        let conn = db::open_db(&dir.join("usage.db")).unwrap();
        // The middle event violates the cost_usd CHECK, an unrelated schema
        // constraint, so valid rows on both sides must not survive.
        let source = Fake(Events {
            usage: vec![
                usage("ok-1", 100, 0.5),
                usage("bad", 200, -1.0),
                usage("ok-2", 300, 0.5),
            ],
            tools: Vec::new(),
        });

        let result = ingest(&source, "a", &conn);

        assert!(result.is_err());
        assert_eq!(count(&conn, "usage_events"), 0);
        assert_eq!(db::get_watermark(&conn, "fake").unwrap(), 0);
        let _ = fs::remove_dir_all(dir);
    }
}
