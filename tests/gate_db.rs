// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration-level coverage for `playbook::gate::db`, exercised through
//! the real `open_db` the way a future CLI caller would: reopen the same
//! path and confirm no error and no duplicate rows.

use playbook::gate::db::{new_runtime, open_db, query_phase, upsert_phase};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch_db_path(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!("playbook-gate-db-{tag}-{}-{n}", std::process::id()))
        .join("state.db")
}

#[test]
fn reopen_after_upserts_does_not_duplicate_rows() {
    // Arrange
    let path = scratch_db_path("reopen");
    new_runtime().expect("runtime").block_on(async {
        let conn = open_db(&path).await.expect("first open");
        upsert_phase(
            &conn,
            "plan-a",
            "spec",
            "WARN",
            "first evidence",
            "cmd-1",
            "2026-01-01T00:00:00Z",
        )
        .await
        .expect("first upsert");
        upsert_phase(
            &conn,
            "plan-a",
            "spec",
            "PASS",
            "second evidence",
            "cmd-2",
            "2026-01-02T00:00:00Z",
        )
        .await
        .expect("second upsert");
    });

    // Act, Assert: a separate runtime and connection, proving reopen after
    // the first one closed neither errors nor loses data. Every connection
    // still opens, is used, and drops inside its OWN one `block_on`.
    new_runtime().expect("runtime").block_on(async {
        let reopened = open_db(&path).await;
        assert!(
            reopened.is_ok(),
            "reopening the same path should not error: {:?}",
            reopened.err()
        );
        let row = query_phase(&reopened.unwrap(), "plan-a", "spec")
            .await
            .expect("query after reopen")
            .expect("row should survive reopen");
        assert_eq!(row.verdict, "PASS");
        assert_eq!(row.evidence, "second evidence");
    });
}

#[test]
fn cross_plan_isolation_holds_through_real_open_db() {
    // Arrange, Act, Assert
    let path = scratch_db_path("cross-plan");
    new_runtime().expect("runtime").block_on(async {
        let conn = open_db(&path).await.expect("open");
        upsert_phase(
            &conn,
            "plan-a",
            "spec",
            "PASS",
            "a-evidence",
            "a-cmd",
            "2026-01-01T00:00:00Z",
        )
        .await
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
        .await
        .expect("upsert plan-b");

        let row_a = query_phase(&conn, "plan-a", "spec")
            .await
            .expect("query plan-a")
            .expect("row a");
        let row_b = query_phase(&conn, "plan-b", "spec")
            .await
            .expect("query plan-b")
            .expect("row b");

        assert_eq!(row_a.verdict, "PASS");
        assert_eq!(row_b.verdict, "FAIL");
        assert_ne!(row_a.evidence, row_b.evidence);
    });
}
