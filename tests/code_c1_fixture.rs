// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The checkpoint C1 task file is complete before anyone runs the checkpoint.

use serde_json::Value;

#[test]
fn the_fixture_holds_twenty_complete_tasks_over_two_pinned_repos() {
    // Arrange
    let text = include_str!("fixtures/code_c1_tasks.json");

    // Act
    let doc: Value = serde_json::from_str(text).expect("fixture parses");

    // Assert
    let tasks = doc["tasks"].as_array().expect("tasks");
    assert_eq!(tasks.len(), 20);
    for task in tasks {
        let repo = task["repo"].as_str().expect("repo");
        let commit = doc["repos"][repo]["commit"].as_str().expect("commit");
        assert_eq!(commit.len(), 40, "{repo}");
        assert!(task["query"].as_str().is_some_and(|q| !q.is_empty()));
        assert!(task["symbol"].as_str().is_some_and(|s| !s.is_empty()));
        let first = &task["expected"][0];
        assert!(first["path"].as_str().is_some_and(|p| !p.is_empty()));
        assert!(first["line"].as_u64().is_some_and(|l| l > 0));
    }
    assert_eq!(doc["repos"].as_object().map(|r| r.len()), Some(2));
}
