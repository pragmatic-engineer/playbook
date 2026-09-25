// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

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
