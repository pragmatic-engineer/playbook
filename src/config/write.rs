// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Validated writes to playbook's own tiered config store: the
//! write-side counterpart to `resolve` in `config::mod`.

use super::{keys, ConfigError};
use serde_json::Value;
use std::path::Path;

/// Which tier `set` writes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Global,
    Org,
    Repo,
}

/// Validate `key` and `value` against `keys::KNOWN_KEYS`, then store them in
/// `tier` of the config database under `home`'s playbook root, creating the
/// database if absent. `repo_slug` (as `<owner>/<repo>`) is required for the
/// org and repo tiers; the global tier ignores it. Every validation runs
/// before any write, so a rejected call never touches the database. SQLite's
/// own locking serializes concurrent writers.
pub fn set(
    tier: Tier,
    key: &str,
    value: Value,
    home: &Path,
    repo_slug: Option<&str>,
) -> Result<(), ConfigError> {
    validate_key_and_value(key, &value)?;
    let root = crate::common::paths::playbook_root_from(home);
    super::store::put(&root, tier, repo_slug, key, &value)
}

/// Reject an unknown key, a value whose JSON type does not match
/// `keys::default_value(key)`, or an out-of-enum string, before any file is
/// touched.
pub(crate) fn validate_key_and_value(key: &str, value: &Value) -> Result<(), ConfigError> {
    let default =
        keys::default_value(key).ok_or_else(|| ConfigError::UnknownKey(key.to_string()))?;
    let expected = match default {
        Value::Bool(_) => "boolean",
        Value::String(_) => "string",
        Value::Number(_) => "number",
        Value::Array(_) => "array of paths",
        _ => "value",
    };
    let type_matches = matches!(
        (&default, value),
        (Value::Bool(_), Value::Bool(_))
            | (Value::String(_), Value::String(_))
            | (Value::Number(_), Value::Number(_))
            | (Value::Array(_), Value::Array(_))
    );
    if !type_matches {
        return Err(ConfigError::WrongType {
            key: key.to_string(),
            expected,
        });
    }
    if let (Some(allowed), Value::String(s)) = (keys::allowed_enum_values(key), value) {
        if !allowed.contains(&s.as_str()) {
            return Err(ConfigError::InvalidEnumValue {
                key: key.to_string(),
                value: s.clone(),
                allowed,
            });
        }
    }
    if let Value::String(s) = value {
        if keys::model_override_error(key, s).is_some() {
            return Err(ConfigError::InvalidEnumValue {
                key: key.to_string(),
                value: s.clone(),
                allowed: &["empty", "claude-<tier>-<major>[-<minor>]"],
            });
        }
    }
    if matches!(default, Value::Number(_)) {
        validate_number(key, value)?;
    }
    if key == "hooks.rmGuard.allowedRoots" {
        validate_root_list(key, value)?;
    }
    Ok(())
}

/// `hooks.rmGuard.allowedRoots` takes strings that are an absolute path or a
/// `~/` path, with no `..` segment and no glob or quote characters, naming a
/// directory at least two levels deep. A root such as `/`, `~` or `/Users` would
/// hand the guard's allowance to far more than a scratch tree.
fn validate_root_list(key: &str, value: &Value) -> Result<(), ConfigError> {
    const CONSTRAINT: &str =
        "absolute or ~/ paths, at least two levels deep, with no .., glob or quote characters";
    let bad = || ConfigError::OutOfRange {
        key: key.to_string(),
        value: value.to_string(),
        constraint: CONSTRAINT,
    };
    let Some(items) = value.as_array() else {
        return Err(bad());
    };
    for item in items {
        let text = item.as_str().ok_or_else(bad)?;
        let rest = match text.strip_prefix("~/") {
            Some(rest) => rest,
            None if text.starts_with('/') => text,
            None => return Err(bad()),
        };
        let segments: Vec<&str> = rest
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .collect();
        let depth_ok = if text.starts_with('~') {
            !segments.is_empty()
        } else {
            segments.len() >= 2
        };
        let clean = !text.contains(['*', '?', '[', '{', '$', '`', '"', '\'', '\\'])
            && segments.iter().all(|s| *s != "..");
        if !depth_ok || !clean {
            return Err(bad());
        }
    }
    Ok(())
}

/// `auto.budgetUsd` and `auto.agentBudgetUsd` take any number of at least one cent, since the spend cap
/// is held in whole cents, so a fractional budget works.
/// `usage.budget.*Usd` takes any number from 0 (no budget).
/// `auto.warnPct` takes a whole number within 1 to 100, `usage.refreshSeconds` one within 1 to
/// 3600, `hygiene.thresholdTokens` one of at least 10000, the `fix.` and `review.stack` limits a whole number of at least 1, and every other
/// numeric key 0 or more.
fn validate_number(key: &str, value: &Value) -> Result<(), ConfigError> {
    let whole = value.as_i64();
    let (valid, constraint) = match key {
        "auto.budgetUsd" | "auto.agentBudgetUsd" => (
            value.as_f64().is_some_and(|n| n.is_finite() && n >= 0.01),
            "a number of at least 0.01",
        ),
        "auto.sessionCeilingUsd" => (
            value
                .as_f64()
                .is_some_and(|n| n.is_finite() && (n == 0.0 || n >= 0.01)),
            "0 (ten times auto.budgetUsd) or a number of at least 0.01",
        ),
        "usage.budget.monthlyUsd" | "usage.budget.weeklyUsd" => (
            value
                .as_f64()
                .is_some_and(|n| n.is_finite() && (0.0..=1_000_000_000.0).contains(&n)),
            "a number of dollars, 0 (no budget) to 1000000000",
        ),
        "auto.warnPct" => (
            whole.is_some_and(|n| (1..=100).contains(&n)),
            "a whole number between 1 and 100",
        ),
        "usage.refreshSeconds" => (
            whole.is_some_and(|n| (1..=3600).contains(&n)),
            "a whole number between 1 and 3600",
        ),
        "sessions.staleAfterDays" | "sessions.recentDays" => {
            (whole.is_some_and(|n| n >= 1), "a positive integer")
        }
        "hygiene.thresholdTokens" => (
            whole.is_some_and(|n| n >= 10_000),
            "a whole number of at least 10000",
        ),
        "fix.maxFiles" | "fix.maxLines" | "review.stackMaxPrs" | "review.stackMaxLines" => {
            (whole.is_some_and(|n| n >= 1), "a positive integer")
        }
        "docs.maxBytes" | "docs.timeoutSeconds" | "docs.maxOutputChars" | "docs.maxMemoryMb" => {
            (whole.is_some_and(|n| n >= 1), "a positive integer")
        }
        _ => {
            return match whole {
                Some(n) if n >= 0 => Ok(()),
                _ => Err(ConfigError::InvalidNumber {
                    key: key.to_string(),
                    value: value.to_string(),
                }),
            }
        }
    };
    if valid {
        return Ok(());
    }
    Err(ConfigError::OutOfRange {
        key: key.to_string(),
        value: value.to_string(),
        constraint,
    })
}
