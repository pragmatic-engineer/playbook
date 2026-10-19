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
    require_global_for_user_key(key, tier == Tier::Global)?;
    check_cap_order(tier, key, &value, home, repo_slug)?;
    let root = crate::common::paths::playbook_root_from(home);
    super::store::put(&root, tier, repo_slug, key, &normalize(key, value))
}

/// `value` in the form it is stored: a root list loses each trailing `/**`
/// or `/*`, so an older binary that refuses globs still reads it.
pub fn normalize(key: &str, value: Value) -> Value {
    if key != "hooks.rmGuard.allowedRoots" {
        return value;
    }
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| match item {
                    Value::String(text) => Value::String(strip_root_glob(&text).to_string()),
                    other => other,
                })
                .collect(),
        ),
        other => other,
    }
}

/// Reject a per-user key bound for the org or repo tier.
pub(crate) fn require_global_for_user_key(key: &str, global: bool) -> Result<(), ConfigError> {
    if keys::is_user_key(key) && !global {
        return Err(ConfigError::GlobalOnly(key.to_string()));
    }
    Ok(())
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
    if key == "pr.ticketPatterns" {
        let ok = value.as_array().is_some_and(|items| {
            items.iter().all(|i| {
                i.as_str()
                    .is_some_and(|p| crate::pr::body_rules::compile_pattern(p).is_ok())
            })
        });
        if !ok {
            return Err(ConfigError::OutOfRange {
                key: key.to_string(),
                value: value.to_string(),
                constraint: "a list of valid regular expressions",
            });
        }
    }
    if let ("usage.ui.layout", Value::String(text)) = (key, value) {
        if !text.is_empty() && playbook_telemetry::tui::layout::Layout::parse(text).is_err() {
            return Err(ConfigError::OutOfRange {
                key: key.to_string(),
                value: value.to_string(),
                constraint: "tabs like Charts=spend,tokens;Numbers=models (1 to 9 tabs; panels: spend, tokens, models, breakdown, live, sessions, budget, projection; a name is 1 to 16 characters)",
            });
        }
    }
    if let ("usage.ui.state", Value::String(text)) = (key, value) {
        if text.len() > playbook_telemetry::config::MAX_STATE_BYTES {
            return Err(ConfigError::OutOfRange {
                key: key.to_string(),
                value: "(too long)".to_string(),
                constraint: "text of at most 4096 bytes, written by pbtop",
            });
        }
    }
    if key == "hooks.rmGuard.allowedRoots" {
        validate_root_list(key, value)?;
    }
    if key == "disk.paths" {
        if let Some(constraint) = crate::disk::policy::path_list_error(value) {
            return Err(ConfigError::OutOfRange {
                key: key.to_string(),
                value: value.to_string(),
                constraint,
            });
        }
    }
    Ok(())
}

/// `hooks.rmGuard.allowedRoots` takes strings that are an absolute path or a
/// `~/` path, with no `..` segment and no glob or quote characters, naming a
/// directory at least two levels deep. A root such as `/`, `~` or `/Users` would
/// hand the guard's allowance to far more than a scratch tree. A trailing `/**`
/// or `/*` is accepted and means the same as the plain path, since a root
/// always covers everything under it.
fn validate_root_list(key: &str, value: &Value) -> Result<(), ConfigError> {
    const CONSTRAINT: &str = "directories given as absolute or ~/ paths, at least two levels deep (one under ~/), with no .., quote or glob characters. A root covers everything under it, so a trailing /** is optional. Example: playbook config set --global hooks.rmGuard.allowedRoots '[\"~/code/acme/.worktrees\",\"/opt/scratch\"]'";
    let bad = || ConfigError::OutOfRange {
        key: key.to_string(),
        value: value.to_string(),
        constraint: CONSTRAINT,
    };
    let Some(items) = value.as_array() else {
        return Err(bad());
    };
    for item in items {
        let text = strip_root_glob(item.as_str().ok_or_else(bad)?);
        let rest = match text.strip_prefix("~/") {
            Some(rest) if rest.starts_with('/') => return Err(bad()),
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

/// `root` without a trailing `/**` or `/*`, which a root implies anyway.
pub(crate) fn strip_root_glob(root: &str) -> &str {
    root.strip_suffix("/**")
        .or_else(|| root.strip_suffix("/*"))
        .unwrap_or(root)
}

const CEILING_KEY: &str = "auto.sessionCeilingUsd";
const BUDGET_KEYS: [(&str, u32); 2] = [
    ("auto.budgetUsd", keys::AUTO_BUDGET_USD),
    ("auto.agentBudgetUsd", keys::AUTO_AGENT_BUDGET_USD),
];

/// Keep a set `auto.sessionCeilingUsd` at least as large as `auto.budgetUsd`
/// and `auto.agentBudgetUsd`, since a lower ceiling parks every agent before
/// its own cap is reached. Setting the ceiling checks it against both
/// budgets; setting a budget checks only that budget against the ceiling. A
/// ceiling of 0 means ten times `auto.budgetUsd` and is not checked. The
/// other keys are read the way the spend hook reads them, as they resolve
/// for `repo_slug`, and for a global write also as they resolve outside any
/// repo, since a global value reaches every repo.
fn check_cap_order(
    tier: Tier,
    key: &str,
    value: &Value,
    home: &Path,
    repo_slug: Option<&str>,
) -> Result<(), ConfigError> {
    let Some(budget_default) = BUDGET_KEYS
        .iter()
        .find(|(budget_key, _)| *budget_key == key)
        .map(|(_, default)| *default)
        .or((key == CEILING_KEY).then_some(0))
    else {
        return Ok(());
    };
    let mut slugs = vec![repo_slug];
    if tier == Tier::Global && repo_slug.is_some() {
        slugs.push(None);
    }
    for slug in slugs {
        let cents = |other: &str| {
            super::resolve(other, home, slug)
                .ok()
                .and_then(|(value, _)| super::auto_cap_cents(&value))
        };
        let reason = if key == CEILING_KEY {
            let Some(ceiling) = super::auto_cap_cents(value) else {
                return Ok(());
            };
            BUDGET_KEYS.iter().find_map(|(budget_key, default)| {
                let budget = cents(budget_key).unwrap_or(i64::from(*default) * 100);
                (budget > ceiling).then(|| {
                    format!(
                        "it is below {budget_key} ({}). Set it to 0 (ten times auto.budgetUsd) or to at least {}",
                        usd(budget),
                        usd(budget)
                    )
                })
            })
        } else {
            let budget = super::auto_cap_cents(value).unwrap_or(i64::from(budget_default) * 100);
            cents(CEILING_KEY).filter(|ceiling| budget > *ceiling).map(|ceiling| {
                format!(
                    "it is above auto.sessionCeilingUsd ({}). Raise the ceiling first with `playbook config set auto.sessionCeilingUsd <usd>`, or set it to 0 (ten times auto.budgetUsd)",
                    usd(ceiling)
                )
            })
        };
        if let Some(reason) = reason {
            return Err(ConfigError::CapOrder {
                key: key.to_string(),
                value: value.to_string(),
                reason,
            });
        }
    }
    Ok(())
}

/// Whole cents as dollars, without trailing zeros: 20000 is `200`, 250 `2.5`.
fn usd(cents: i64) -> String {
    let text = format!("{:.2}", cents as f64 / 100.0);
    text.trim_end_matches('0').trim_end_matches('.').to_string()
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
            "0 (ten times auto.budgetUsd) or a number of at least 0.01 that is not below auto.budgetUsd or auto.agentBudgetUsd",
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
        "disk.warnFreeGb" | "disk.blockFreeGb" => (
            whole.is_some_and(|n| (0..=crate::disk::policy::MAX_THRESHOLD_GB).contains(&n)),
            "a whole number of GB between 0 and 10000",
        ),
        "disk.maxTargetDirs" => (
            value
                .as_u64()
                .is_some_and(|n| (1..=crate::disk::policy::MAX_TARGET_DIRS).contains(&n)),
            "a whole number between 1 and 1000",
        ),
        "disk.maxTargetGb" => (
            whole.is_some_and(|n| (1..=crate::disk::policy::MAX_THRESHOLD_GB).contains(&n)),
            "a whole number of GB between 1 and 10000",
        ),
        "sessions.staleAfterDays" | "sessions.recentDays" => {
            (whole.is_some_and(|n| n >= 1), "a positive integer")
        }
        "hygiene.thresholdTokens" => (
            whole.is_some_and(|n| n >= 10_000),
            "a whole number of at least 10000",
        ),
        "fix.maxFiles"
        | "fix.maxLines"
        | "review.stackMaxPrs"
        | "review.stackMaxLines"
        | "review.suggest.deepLines"
        | "review.suggest.quickLines"
        | "review.suggest.deepFiles"
        | "review.suggest.packageSpan"
        | "review.suggest.untestedLines"
        | "update.checkIntervalHours" => (whole.is_some_and(|n| n >= 1), "a positive integer"),
        "pr.bodyMaxChars"
        | "docs.maxBytes"
        | "docs.timeoutSeconds"
        | "docs.maxOutputChars"
        | "docs.maxMemoryMb" => (whole.is_some_and(|n| n >= 1), "a positive integer"),
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

#[cfg(test)]
mod root_list_tests {
    use super::validate_root_list;
    use serde_json::json;

    #[test]
    fn the_root_list_accepts_only_deep_clean_absolute_or_home_paths() {
        for good in [
            json!([]),
            json!(["~/a"]),
            json!(["~/a/b/"]),
            json!(["/opt/data"]),
            json!(["/private/tmp"]),
            json!(["/a/b/*", "/a/b/**", "~/w/.worktrees/**"]),
        ] {
            assert!(validate_root_list("k", &good).is_ok(), "{good}");
        }
        for bad in [
            json!("~/a"),
            json!([1]),
            json!(["/"]),
            json!(["/tmp"]),
            json!(["~"]),
            json!(["~/"]),
            json!(["~/."]),
            json!(["~//etc"]),
            json!(["rel/dir"]),
            json!(["/a/b/../c"]),
            json!(["/tmp/**"]),
            json!(["~/**"]),
            json!(["/a/*/b"]),
            json!(["/a/b/***"]),
            json!(["/a/it's"]),
            json!(["/a/b\\c"]),
            json!(["/a/$X"]),
        ] {
            assert!(validate_root_list("k", &bad).is_err(), "{bad}");
        }
    }
}
