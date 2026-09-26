// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Playbook's own tiered (repo < org < global < default) configuration
//! system for keys such as `autoReview.enabled` and `autoReview.type`.
//!
//! Distinct from `common::config_hash` (drift detection on Claude Code's own
//! `settings.json`) and `settings::keys` (the `settings.shared.json` seed
//! allowlist): this module governs playbook's own config keys and their
//! defaults, not Claude Code's.

pub mod keys;
pub mod write;

use serde_json::Value;
use std::path::{Path, PathBuf};

/// Which tier of the repo/org/global/default chain actually supplied a
/// resolved value, so a caller (or a future `playbook config get` command)
/// can report where a setting came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Repo,
    Org,
    Global,
    Default,
}

/// Everything that can stop `resolve` or `write::set` from producing or
/// storing a value. A missing tier file is not one of these cases when
/// reading, since it is a normal "no override here" outcome handled by
/// falling through to the next tier; `write::set` treats it the same way,
/// as "nothing to merge into yet".
#[derive(Debug)]
pub enum ConfigError {
    /// A tier file exists but is not readable as a JSON object: either it
    /// fails to parse, or it parses to a non-object value.
    Malformed(PathBuf),
    /// `key` is not in `keys::KNOWN_KEYS`, so no tier and no default can
    /// ever supply it.
    UnknownKey(String),
    /// `value`'s JSON type does not match what `keys::default_value(key)`
    /// implies for `key`.
    WrongType { key: String, expected: &'static str },
    /// `value` is a string outside `keys::allowed_enum_values(key)`.
    InvalidEnumValue {
        key: String,
        value: String,
        allowed: &'static [&'static str],
    },
    /// `value` is a JSON number but not a non-negative integer (negative,
    /// or fractional such as `3.5`).
    InvalidNumber { key: String, value: String },
    /// A write targeted the org or repo tier but no `repo_slug` was
    /// available to build that tier's path from.
    MissingRepoContext,
    /// A tier's directory could not be created or written into: its parent
    /// path exists as a non-directory, or is not writable.
    DirectoryUnwritable(PathBuf),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Malformed(path) => write!(
                f,
                "config file is not a valid JSON object: {}",
                path.display()
            ),
            ConfigError::UnknownKey(key) => write!(
                f,
                "unknown config key: {key}, valid keys are: {}",
                keys::KNOWN_KEYS.join(", ")
            ),
            ConfigError::WrongType { key, expected } => {
                write!(f, "config key {key} expects a {expected} value")
            }
            ConfigError::InvalidEnumValue {
                key,
                value,
                allowed,
            } => write!(
                f,
                "config key {key} does not accept '{value}', valid values are: {}",
                allowed.join(", ")
            ),
            ConfigError::InvalidNumber { key, value } => write!(
                f,
                "config key {key} does not accept '{value}', must be a non-negative integer"
            ),
            ConfigError::MissingRepoContext => write!(
                f,
                "could not resolve a repo-scoped config location; repo_slug is unresolved, \
                 refusing to write an org or repo tier config value"
            ),
            ConfigError::DirectoryUnwritable(path) => write!(
                f,
                "config directory could not be created or written into: {}",
                path.display()
            ),
        }
    }
}

/// Resolve `key` through the repo, org, global, then built-in default tier,
/// in that order, returning the value and which tier supplied it. `home`
/// stands in for `$HOME` so callers (and tests) can point resolution at a
/// scratch directory instead of the real one, matching
/// `common::session::home_dir`'s injection convention. The repo and org
/// tiers are skipped entirely when `repo_slug` is `None`, since there is no
/// `<owner>/<repo>` to build their paths from.
pub fn resolve(
    key: &str,
    home: &Path,
    repo_slug: Option<&str>,
) -> Result<(Value, Source), ConfigError> {
    // Checked up front, not just as the post-tiers fallback: a tier file can
    // legitimately contain an arbitrary dotted path (a stale key from a
    // retired setting, a typo, a future key written by a newer binary), and
    // that must not let an unknown key resolve successfully just because
    // some file happens to have it.
    keys::default_value(key).ok_or_else(|| ConfigError::UnknownKey(key.to_string()))?;

    let root = crate::common::paths::playbook_root_from(home);

    if let Some((owner, repo)) = repo_slug.and_then(|slug| slug.split_once('/')) {
        if let Some(value) = lookup_tier(&repo_config_path(&root, owner, repo), key)? {
            return Ok((value, Source::Repo));
        }

        if let Some(value) = lookup_tier(&org_config_path(&root, owner), key)? {
            return Ok((value, Source::Org));
        }
    }

    if let Some(value) = lookup_tier(&global_config_path(&root), key)? {
        return Ok((value, Source::Global));
    }

    // Never `None` here: the top-of-function check already proved `key` is
    // known, so a default value always exists.
    Ok((
        keys::default_value(key).expect("key was already validated as known"),
        Source::Default,
    ))
}

/// The global tier's config file, directly under `root`. Shared by `resolve`
/// and `write::tier_path` so the two never drift on where this file lives.
pub(crate) fn global_config_path(root: &Path) -> PathBuf {
    root.join("config.json")
}

/// The org tier's config file for `owner`. Shared by `resolve` and
/// `write::tier_path` so the two never drift on where this file lives.
pub(crate) fn org_config_path(root: &Path, owner: &str) -> PathBuf {
    root.join("orgs").join(owner).join("config.json")
}

/// The repo tier's config file for `owner`/`repo`, at the `RepoScope::Config`
/// slot `src/common/paths.rs` reserves. Shared by `resolve` and
/// `write::tier_path` so the two never drift on where this file lives.
pub(crate) fn repo_config_path(root: &Path, owner: &str, repo: &str) -> PathBuf {
    root.join("repos")
        .join(owner)
        .join(repo)
        .join(".config")
        .join("config.json")
}

/// Read one tier file and look up `key` in it. `Ok(None)` means "no
/// override here", covering both a missing file and a file that parses
/// fine but does not contain `key`; both fall through to the next tier the
/// same way. A file that fails to parse, or that parses to something other
/// than a JSON object, is `Err` instead: a malformed file is never silently
/// treated as absent.
fn lookup_tier(path: &Path, key: &str) -> Result<Option<Value>, ConfigError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ConfigError::Malformed(path.to_path_buf())),
    };
    let parsed: Value =
        serde_json::from_str(&raw).map_err(|_| ConfigError::Malformed(path.to_path_buf()))?;
    if !parsed.is_object() {
        return Err(ConfigError::Malformed(path.to_path_buf()));
    }
    Ok(dotted_lookup(&parsed, key).cloned())
}

/// Look up a dotted path (`"autoReview.enabled"`) inside a JSON object,
/// descending one segment at a time. `None` means some segment along the
/// path is absent, distinct from a `Some(Value::Null)` found at the full
/// path, which is a present value.
fn dotted_lookup<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in key.split('.') {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::write::Tier;
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A fresh scratch directory standing in for `$HOME`, unique per call so
    /// parallel tests never collide.
    fn scratch_home(tag: &str) -> PathBuf {
        let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "playbook-config-mod-{}-{tag}-{n}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("scratch home should be creatable");
        dir
    }

    #[test]
    fn stale_after_days_defaults_to_thirty_with_no_override() {
        // Arrange
        let home = scratch_home("stale-after-days-default");

        // Act
        let result = resolve("worktreeCleanup.staleAfterDays", &home, None);

        // Assert
        assert_eq!(result.unwrap(), (Value::Number(30.into()), Source::Default));

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn stale_after_days_round_trips_through_write_and_resolve() {
        // Arrange
        let home = scratch_home("stale-after-days-round-trip");
        write::set(
            Tier::Repo,
            "worktreeCleanup.staleAfterDays",
            Value::Number(14.into()),
            &home,
            Some("owner/repo"),
        )
        .expect("write should succeed once staleAfterDays is a known key");

        // Act
        let result = resolve("worktreeCleanup.staleAfterDays", &home, Some("owner/repo"));

        // Assert
        assert_eq!(result.unwrap(), (Value::Number(14.into()), Source::Repo));

        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn rejects_invalid_stale_after_days_values() {
        // Each case pairs an invalid value with substrings its rejection
        // error must contain.
        let cases: Vec<(Value, Vec<&str>)> = vec![
            (
                Value::String("not-a-number".to_string()),
                vec!["worktreeCleanup.staleAfterDays", "number"],
            ),
            (
                Value::Number((-5).into()),
                vec!["worktreeCleanup.staleAfterDays", "-5"],
            ),
            (
                Value::Number(serde_json::Number::from_f64(3.5).unwrap()),
                vec!["worktreeCleanup.staleAfterDays", "3.5"],
            ),
        ];

        for (value, expected_substrings) in cases {
            // Arrange
            let home = scratch_home("stale-after-days-rejects");

            // Act
            let result = write::set(
                Tier::Repo,
                "worktreeCleanup.staleAfterDays",
                value,
                &home,
                Some("owner/repo"),
            );

            // Assert
            let message = result
                .expect_err("invalid value should be rejected")
                .to_string();
            for expected in expected_substrings {
                assert!(
                    message.contains(expected),
                    "expected error message {message:?} to contain {expected:?}"
                );
            }

            let _ = fs::remove_dir_all(&home);
        }
    }
}
