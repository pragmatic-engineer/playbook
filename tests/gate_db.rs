// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Integration-level coverage for `playbook::gate::db`, exercised through
//! the real `open_db` the way a future CLI caller would: reopen the same
//! path and confirm no error and no duplicate rows.

use playbook::gate::db::{open_db, query_phase, upsert_phase};
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
    let conn = open_db(&path).expect("first open");
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

    // Act
    let reopened = open_db(&path);

    // Assert
    assert!(
        reopened.is_ok(),
        "reopening the same path should not error: {:?}",
        reopened.err()
    );
    let row = query_phase(&reopened.unwrap(), "plan-a", "spec")
        .expect("query after reopen")
        .expect("row should survive reopen");
    assert_eq!(row.verdict, "PASS");
    assert_eq!(row.evidence, "second evidence");
}

#[test]
fn cross_plan_isolation_holds_through_real_open_db() {
    // Arrange
    let path = scratch_db_path("cross-plan");
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
}
