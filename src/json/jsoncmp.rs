// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Structural JSON equality, replacing `diff <(jq -S ...) <(jq -S ...)` call
//! sites that compare two files while ignoring key order and, optionally, a
//! handful of named top-level keys (e.g. a generated timestamp).
//! Deliberately not a general JSON diff: only top-level keys are named and
//! dropped or compared, matching every real call site this backs.

use serde_json::Value;
use std::path::Path;

/// Removes each named key from `value` if it is a top-level object member.
/// No-op when a key is absent, or when `value` is not an object at all
/// (an array or scalar top-level document has no keys to drop).
pub fn drop_top_level_keys(value: &mut Value, keys: &[&str]) {
    if let Some(map) = value.as_object_mut() {
        for key in keys {
            map.remove(*key);
        }
    }
}

/// Compares two JSON files for structural equality after dropping
/// `ignore_keys` from each parsed top-level document. Key order never
/// matters, since `serde_json::Value`'s `PartialEq` on objects compares by
/// key rather than by insertion order.
///
/// Returns `Err` naming every top-level key that differs (added, removed, or
/// changed value), not only the first, so a failing comparison is as
/// diagnosable as the `diff` output it replaces. A non-object top-level
/// value (an array or a scalar) is compared directly, since
/// `drop_top_level_keys` is a no-op on it; the `Err` in that case just
/// states the two documents differ, since there are no top-level keys to
/// list.
pub fn json_equal(a: &Path, b: &Path, ignore_keys: &[&str]) -> Result<(), String> {
    let mut a_value = read_json(a)?;
    let mut b_value = read_json(b)?;

    drop_top_level_keys(&mut a_value, ignore_keys);
    drop_top_level_keys(&mut b_value, ignore_keys);

    if a_value == b_value {
        return Ok(());
    }

    match (a_value.as_object(), b_value.as_object()) {
        (Some(a_map), Some(b_map)) => {
            let mut differing: Vec<&str> = a_map
                .keys()
                .chain(b_map.keys())
                .filter(|key| a_map.get(key.as_str()) != b_map.get(key.as_str()))
                .map(String::as_str)
                .collect();
            differing.sort_unstable();
            differing.dedup();
            Err(format!(
                "{} and {} differ in top-level key(s): {}",
                a.display(),
                b.display(),
                differing.join(", ")
            ))
        }
        _ => Err(format!("{} and {} differ", a.display(), b.display())),
    }
}

/// Reads and parses `path` as JSON, turning either failure mode into a
/// distinct, readable `Err` naming the path rather than panicking.
fn read_json(path: &Path) -> Result<Value, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("malformed JSON in {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A scratch file holding `contents`, cleaned up on drop.
    struct Fixture {
        path: PathBuf,
    }

    impl Fixture {
        fn new(tag: &str, contents: &str) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "playbook-json-jsoncmp-{tag}-{}-{n}.json",
                std::process::id()
            ));
            fs::write(&path, contents).expect("fixture file should be writable");
            Self { path }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn drop_top_level_keys_removes_named_key_leaves_others_unchanged() {
        // Arrange
        let mut value = json!({"a": 1, "b": 2, "c": 3});

        // Act
        super::drop_top_level_keys(&mut value, &["b"]);

        // Assert
        assert_eq!(value, json!({"a": 1, "c": 3}));
    }

    #[test]
    fn drop_top_level_keys_is_a_noop_when_key_is_absent() {
        // Arrange
        let mut value = json!({"a": 1, "b": 2});

        // Act
        super::drop_top_level_keys(&mut value, &["missing"]);

        // Assert
        assert_eq!(value, json!({"a": 1, "b": 2}));
    }

    #[test]
    fn drop_top_level_keys_is_a_noop_on_a_non_object_value() {
        // Arrange
        let mut value = json!([1, 2, 3]);

        // Act
        super::drop_top_level_keys(&mut value, &["a"]);

        // Assert
        assert_eq!(value, json!([1, 2, 3]));
    }

    #[test]
    fn json_equal_ok_when_content_matches_but_source_key_order_differs() {
        // Arrange
        let a = Fixture::new("key-order-a", r#"{"a": 1, "b": 2}"#);
        let b = Fixture::new("key-order-b", r#"{"b": 2, "a": 1}"#);

        // Act
        let got = super::json_equal(&a.path, &b.path, &[]);

        // Assert
        assert!(got.is_ok());
    }

    #[test]
    fn json_equal_ok_when_difference_is_only_in_an_ignored_key() {
        // Arrange
        let a = Fixture::new("ignored-key-a", r#"{"a": 1, "generated_at": "2026-01-01"}"#);
        let b = Fixture::new("ignored-key-b", r#"{"a": 1, "generated_at": "2026-09-28"}"#);

        // Act
        let got = super::json_equal(&a.path, &b.path, &["generated_at"]);

        // Assert
        assert!(got.is_ok());
    }

    #[test]
    fn json_equal_err_lists_every_differing_top_level_key() {
        // Arrange
        let a = Fixture::new("diff-keys-a", r#"{"a": 1, "b": 2, "c": 3}"#);
        let b = Fixture::new("diff-keys-b", r#"{"a": 1, "b": 20, "c": 30}"#);

        // Act
        let got = super::json_equal(&a.path, &b.path, &[]);

        // Assert
        let err = got.expect_err("differing non-ignored keys should error");
        assert!(err.contains('b'), "error should mention key 'b': {err}");
        assert!(err.contains('c'), "error should mention key 'c': {err}");
    }

    #[test]
    fn json_equal_err_when_one_file_path_does_not_exist() {
        // Arrange
        let a = Fixture::new("missing-file-a", r#"{"a": 1}"#);
        let missing = std::env::temp_dir().join(format!(
            "playbook-json-jsoncmp-missing-file-b-{}.json",
            std::process::id()
        ));

        // Act
        let got = super::json_equal(&a.path, &missing, &[]);

        // Assert
        assert!(got.is_err(), "a missing file should error, not panic");
    }

    #[test]
    fn json_equal_err_when_one_file_has_malformed_json() {
        // Arrange
        let a = Fixture::new("malformed-a", r#"{"a": 1}"#);
        let b = Fixture::new("malformed-b", "not json");

        // Act
        let got = super::json_equal(&a.path, &b.path, &[]);

        // Assert
        assert!(got.is_err(), "malformed JSON should error, not panic");
    }

    #[test]
    fn json_equal_ok_for_equal_top_level_arrays() {
        // Arrange
        let a = Fixture::new("array-equal-a", r#"[1, 2, 3]"#);
        let b = Fixture::new("array-equal-b", r#"[1, 2, 3]"#);

        // Act
        let got = super::json_equal(&a.path, &b.path, &["ignored"]);

        // Assert
        assert!(got.is_ok());
    }

    #[test]
    fn json_equal_err_for_unequal_top_level_arrays() {
        // Arrange
        let a = Fixture::new("array-unequal-a", r#"[1, 2, 3]"#);
        let b = Fixture::new("array-unequal-b", r#"[1, 2, 4]"#);

        // Act
        let got = super::json_equal(&a.path, &b.path, &[]);

        // Assert
        assert!(got.is_err());
    }

    #[test]
    fn json_equal_ok_for_equal_top_level_scalars() {
        // Arrange
        let a = Fixture::new("scalar-equal-a", "5");
        let b = Fixture::new("scalar-equal-b", "5");

        // Act
        let got = super::json_equal(&a.path, &b.path, &[]);

        // Assert
        assert!(got.is_ok());
    }

    #[test]
    fn json_equal_err_for_unequal_top_level_scalars() {
        // Arrange
        let a = Fixture::new("scalar-unequal-a", "5");
        let b = Fixture::new("scalar-unequal-b", "6");

        // Act
        let got = super::json_equal(&a.path, &b.path, &[]);

        // Assert
        assert!(got.is_err());
    }
}
