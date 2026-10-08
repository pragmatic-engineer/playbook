// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for `playbook::config::write::set`: creating and
//! merging tier files, rejecting an invalid key, value, or missing repo
//! context before any write happens, and concurrent-writer safety.

use playbook::config::write::{set, Tier};
use playbook::config::ConfigError;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch directory standing in for `$HOME`, unique per call so
/// parallel tests never collide.
fn scratch_home(tag: &str) -> PathBuf {
    let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "playbook-config-write-{}-{tag}-{n}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("scratch home should be creatable");
    dir
}

fn repo_config_path(home: &Path, owner: &str, repo: &str) -> PathBuf {
    home.join(".config")
        .join("playbook")
        .join("repos")
        .join(owner)
        .join(repo)
        .join(".config")
        .join("config.json")
}

fn read_json(path: &Path) -> Value {
    let raw = fs::read_to_string(path).expect("tier file should be readable");
    serde_json::from_str(&raw).expect("tier file should be valid JSON")
}

#[test]
fn writing_to_an_absent_tier_creates_its_directory_and_file() {
    // Arrange
    let home = scratch_home("fresh");
    let path = repo_config_path(&home, "owner", "repo");

    // Act
    let result = set(
        Tier::Repo,
        "autoReview.enabled",
        json!(false),
        &home,
        Some("owner/repo"),
    );

    // Assert
    assert!(result.is_ok());
    assert_eq!(read_json(&path), json!({"autoReview": {"enabled": false}}));

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn set_merges_a_new_key_without_dropping_existing_content() {
    // Arrange
    let home = scratch_home("merge");
    let path = repo_config_path(&home, "owner", "repo");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, r#"{"someFutureKey": 1}"#).unwrap();

    // Act
    let result = set(
        Tier::Repo,
        "autoReview.enabled",
        json!(true),
        &home,
        Some("owner/repo"),
    );

    // Assert
    assert!(result.is_ok());
    assert_eq!(
        read_json(&path),
        json!({"someFutureKey": 1, "autoReview": {"enabled": true}})
    );

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn an_unknown_key_is_rejected_and_writes_nothing() {
    // Arrange
    let home = scratch_home("unknown-key");
    let path = repo_config_path(&home, "owner", "repo");

    // Act
    let result = set(
        Tier::Repo,
        "notAKnownKey",
        json!(true),
        &home,
        Some("owner/repo"),
    );

    // Assert
    let err = result.unwrap_err();
    assert!(matches!(&err, ConfigError::UnknownKey(k) if k == "notAKnownKey"));
    assert!(err.to_string().contains("autoReview.enabled"));
    assert!(!path.exists());
}

#[test]
fn a_wrong_typed_value_is_rejected_and_writes_nothing() {
    // Arrange
    let home = scratch_home("wrong-type");
    let path = repo_config_path(&home, "owner", "repo");

    // Act
    let result = set(
        Tier::Repo,
        "autoReview.enabled",
        json!("not-a-bool"),
        &home,
        Some("owner/repo"),
    );

    // Assert
    assert!(matches!(result.unwrap_err(), ConfigError::WrongType { .. }));
    assert!(!path.exists());
}

#[test]
fn an_out_of_enum_value_is_rejected_and_writes_nothing() {
    // Arrange
    let home = scratch_home("bad-enum");
    let path = repo_config_path(&home, "owner", "repo");

    // Act
    let result = set(
        Tier::Repo,
        "autoReview.type",
        json!("medium"),
        &home,
        Some("owner/repo"),
    );

    // Assert
    assert!(matches!(
        result.unwrap_err(),
        ConfigError::InvalidEnumValue { .. }
    ));
    assert!(!path.exists());
}

#[test]
fn repo_tier_with_no_repo_slug_is_rejected_and_writes_nothing() {
    // Arrange
    let home = scratch_home("no-repo-slug");

    // Act
    let result = set(Tier::Repo, "autoReview.enabled", json!(true), &home, None);

    // Assert
    assert!(matches!(
        result.unwrap_err(),
        ConfigError::MissingRepoContext
    ));

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn an_uncreatable_directory_is_rejected_with_a_path_naming_error() {
    // Arrange: a regular file occupies the exact path the repo's parent
    // directory needs to be, so `create_dir_all` cannot succeed.
    let home = scratch_home("uncreatable-dir");
    let blocking_path = home
        .join(".config")
        .join("playbook")
        .join("repos")
        .join("owner");
    fs::create_dir_all(blocking_path.parent().unwrap()).unwrap();
    fs::write(&blocking_path, "not a directory").unwrap();

    // Act
    let result = set(
        Tier::Repo,
        "autoReview.enabled",
        json!(true),
        &home,
        Some("owner/repo"),
    );

    // Assert
    match result.unwrap_err() {
        ConfigError::DirectoryUnwritable(path) => {
            assert!(path.starts_with(&blocking_path));
        }
        other => panic!("expected DirectoryUnwritable, got {other:?}"),
    }

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn a_non_object_existing_tier_file_is_rejected_rather_than_overwritten() {
    // Arrange
    let home = scratch_home("non-object");
    let path = repo_config_path(&home, "owner", "repo");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "[1,2,3]").unwrap();

    // Act
    let result = set(
        Tier::Repo,
        "autoReview.enabled",
        json!(true),
        &home,
        Some("owner/repo"),
    );

    // Assert
    let err = result.unwrap_err();
    assert!(matches!(err, ConfigError::Malformed(_)));
    assert!(err.to_string().contains(&path.display().to_string()));
    assert_eq!(fs::read_to_string(&path).unwrap(), "[1,2,3]");

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn twenty_concurrent_writers_never_tear_the_tier_file() {
    // Arrange
    let home = scratch_home("concurrent");
    let path = repo_config_path(&home, "owner", "repo");
    let thread_count = 20;

    // Act
    let handles: Vec<_> = (0..thread_count)
        .map(|i| {
            let home = home.clone();
            std::thread::spawn(move || {
                if i % 2 == 0 {
                    let enabled = i % 4 == 0;
                    set(
                        Tier::Repo,
                        "autoReview.enabled",
                        json!(enabled),
                        &home,
                        Some("owner/repo"),
                    )
                } else {
                    let kind = if i % 4 == 1 { "quick" } else { "deep" };
                    set(
                        Tier::Repo,
                        "autoReview.type",
                        json!(kind),
                        &home,
                        Some("owner/repo"),
                    )
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap().expect("each write should succeed");
    }

    // Assert: valid JSON, and whichever keys survived hold a value some
    // thread actually wrote. `with_dir_lock` is fail-open (it still runs the
    // critical section after exhausting its retries), so an entire key
    // dropping under sustained contention is an accepted lost update, not a
    // defect this test rules out; only a torn or unparseable file would be.
    let content = read_json(&path);
    let auto_review = content.get("autoReview").and_then(|v| v.as_object());
    if let Some(enabled) = auto_review.and_then(|o| o.get("enabled")) {
        enabled.as_bool().expect("enabled should be a bool");
    }
    if let Some(kind) = auto_review.and_then(|o| o.get("type")) {
        let kind = kind.as_str().expect("type should be a string");
        assert!(kind == "quick" || kind == "deep");
    }

    let _ = fs::remove_dir_all(&home);
}

fn global_config_path(home: &Path) -> PathBuf {
    home.join(".config").join("playbook").join("config.json")
}

/// Writes `value` to the global tier and asserts the write is refused with a
/// message that names `key` and carries `constraint`, leaving no file behind.
fn assert_rejected(key: &str, value: &Value, constraint: &str) {
    // Arrange
    let home = scratch_home("range-rejected");

    // Act
    let result = set(Tier::Global, key, value.clone(), &home, None);

    // Assert
    let err = result.expect_err(&format!("{key} = {value} must be rejected"));
    let message = err.to_string();
    assert!(message.contains(key), "{key} = {value}: {message}");
    assert!(
        message.contains(constraint),
        "{key} = {value} must explain '{constraint}': {message}"
    );
    assert!(
        !global_config_path(&home).exists(),
        "{key} = {value}: a rejected write must not create the file"
    );

    let _ = fs::remove_dir_all(&home);
}

/// Writes `value` to the global tier and asserts it lands in the file.
fn assert_written(key: &str, value: &Value) {
    // Arrange
    let home = scratch_home("range-accepted");

    // Act
    let result = set(Tier::Global, key, value.clone(), &home, None);

    // Assert
    result.unwrap_or_else(|e| panic!("{key} = {value} must be accepted: {e}"));
    let stored = read_json(&global_config_path(&home));
    let leaf = key
        .split('.')
        .fold(&stored, |node, segment| &node[segment])
        .clone();
    assert_eq!(&leaf, value, "{key}");

    let _ = fs::remove_dir_all(&home);
}

#[test]
fn a_budget_under_one_cent_is_rejected_with_a_clear_message() {
    for value in [
        json!(0),
        json!(0.0),
        json!(-3),
        json!(-0.5),
        json!(0.004),
        json!(0.009),
    ] {
        assert_rejected("auto.budgetUsd", &value, "at least 0.01");
    }
}

#[test]
fn a_budget_of_a_cent_or_more_including_a_fractional_one_is_written() {
    for value in [json!(5), json!(1000), json!(0.5), json!(2.5), json!(0.01)] {
        assert_written("auto.budgetUsd", &value);
    }
}

#[test]
fn a_warn_percentage_outside_one_to_a_hundred_or_fractional_is_rejected() {
    for value in [json!(0), json!(-1), json!(101), json!(1000), json!(70.5)] {
        assert_rejected("auto.warnPct", &value, "between 1 and 100");
    }
}

#[test]
fn a_warn_percentage_from_one_to_a_hundred_is_written() {
    for value in [json!(1), json!(70), json!(100)] {
        assert_written("auto.warnPct", &value);
    }
}

#[test]
fn a_fix_threshold_that_is_not_a_positive_integer_is_rejected() {
    for key in ["fix.maxFiles", "fix.maxLines"] {
        for value in [json!(0), json!(-1), json!(2.5)] {
            assert_rejected(key, &value, "positive integer");
        }
    }
}

#[test]
fn a_positive_integer_fix_threshold_is_written() {
    for key in ["fix.maxFiles", "fix.maxLines"] {
        for value in [json!(1), json!(99)] {
            assert_written(key, &value);
        }
    }
}

#[test]
fn a_non_number_for_a_numeric_auto_or_fix_key_is_rejected_and_writes_nothing() {
    for key in [
        "auto.budgetUsd",
        "auto.warnPct",
        "fix.maxFiles",
        "fix.maxLines",
    ] {
        // Arrange
        let home = scratch_home("non-number");

        // Act
        let result = set(Tier::Global, key, json!("banana"), &home, None);

        // Assert
        let err = result.expect_err(&format!("{key} = banana must be rejected"));
        assert!(err.to_string().contains(key), "{err}");
        assert!(!global_config_path(&home).exists(), "{key}");

        let _ = fs::remove_dir_all(&home);
    }
}

#[test]
fn zero_is_still_accepted_for_the_other_numeric_keys() {
    assert_written("worktreeCleanup.staleAfterDays", &json!(0));
}

const BOOL_SETTINGS: [&str; 4] = [
    "autoReview.fix",
    "autoMerge.enabled",
    "commit.signOff",
    "pr.draft",
];

#[test]
fn a_non_bool_for_the_pr_and_commit_settings_is_rejected_and_writes_nothing() {
    for key in BOOL_SETTINGS {
        for value in [json!("yes"), json!(1)] {
            // Arrange
            let home = scratch_home("non-bool");

            // Act
            let result = set(Tier::Global, key, value.clone(), &home, None);

            // Assert
            let err = result.expect_err(&format!("{key} = {value} must be rejected"));
            assert!(matches!(err, ConfigError::WrongType { .. }), "{key}: {err}");
            assert!(err.to_string().contains(key), "{err}");
            assert!(!global_config_path(&home).exists(), "{key}");

            let _ = fs::remove_dir_all(&home);
        }
    }
}

#[test]
fn a_bool_for_the_pr_and_commit_settings_is_written() {
    for key in BOOL_SETTINGS {
        assert_written(key, &json!(true));
        assert_written(key, &json!(false));
    }
}
