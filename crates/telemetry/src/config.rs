// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `usage.*` settings: the keys, their defaults and the values they accept.
//! This module is the source of truth for them. playbook-telemetry reads them
//! from playbook's own config store, `~/.config/playbook/playbook.db` (the
//! global tier), so the standalone tool and playbook always see the same
//! settings. A playbook that predates that store keeps them in
//! `~/.config/playbook/config.json` as a nested JSON object, which is read
//! when the store does not exist. This module only reads; settings are changed
//! with `playbook config set`.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Every `usage.*` key.
pub const KEYS: &[&str] = &[
    "usage.theme",
    "usage.weekStart",
    "usage.timeZone",
    "usage.refreshSeconds",
    "usage.telemetry",
    "usage.telemetryRetentionDays",
    "usage.telemetryMaxRows",
    "usage.autoIngest",
    "usage.autoIngestMinutes",
    "usage.budget.monthlyUsd",
    "usage.budget.weeklyUsd",
];

/// The built-in default of `key`, `None` for a key that is not a usage key.
pub fn default_value(key: &str) -> Option<Value> {
    Some(match key {
        "usage.theme" => Value::from("default"),
        "usage.weekStart" => Value::from("monday"),
        "usage.timeZone" => Value::from("local"),
        "usage.refreshSeconds" => Value::from(5),
        "usage.telemetry" => Value::from("auto"),
        "usage.autoIngest" => Value::from(true),
        "usage.autoIngestMinutes" => Value::from(10),
        "usage.telemetryRetentionDays" => Value::from(90),
        "usage.telemetryMaxRows" => Value::from(500_000),
        "usage.budget.monthlyUsd" | "usage.budget.weeklyUsd" => Value::from(0),
        _ => return None,
    })
}

/// The accepted strings of an enum key.
pub fn allowed_values(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "usage.theme" => Some(&["default", "dark", "light", "mono"]),
        "usage.weekStart" => Some(&["monday", "sunday", "saturday"]),
        "usage.timeZone" => Some(&["local", "utc"]),
        "usage.telemetry" => Some(&["auto", "off"]),
        _ => None,
    }
}

/// `value` when it is valid for `key`, else the reason it is not.
pub fn validate(key: &str, value: &Value) -> Result<(), String> {
    if let Some(allowed) = allowed_values(key) {
        return match value.as_str() {
            Some(s) if allowed.contains(&s) => Ok(()),
            _ => Err(format!("valid values are: {}", allowed.join(", "))),
        };
    }
    match key {
        "usage.autoIngest" if !value.is_boolean() => Err("expected true or false".into()),
        "usage.autoIngest" => Ok(()),
        _ => match value.as_f64() {
            Some(n) if n >= 0.0 => Ok(()),
            _ => Err("expected a number of zero or more".into()),
        },
    }
}

fn warn_once(flag: &AtomicBool, message: &str) {
    if !flag.swap(true, Ordering::Relaxed) {
        crate::say::notice(message);
    }
}

static WARNED_STORE: AtomicBool = AtomicBool::new(false);
static WARNED_VALUE: AtomicBool = AtomicBool::new(false);
static WARNED_OLD_FILE: AtomicBool = AtomicBool::new(false);

/// `~/.config/playbook`, where playbook keeps its settings and this tool its data.
pub fn root(home: &Path) -> PathBuf {
    home.join(".config").join("playbook")
}

/// The global-tier value of `key` from playbook's config store, or from the
/// legacy nested `config.json`. `None` when unset or unreadable (an unreadable
/// source prints a warning once).
fn stored(home: &Path, key: &str) -> Option<Value> {
    let dir = root(home);
    let db = dir.join("playbook.db");
    if db.is_file() {
        return match read_store(&db, key) {
            Ok(v) => v,
            Err(e) => {
                warn_once(
                    &WARNED_STORE,
                    &format!("cannot read {}: {e}; using default settings", db.display()),
                );
                None
            }
        };
    }
    let json = dir.join("config.json");
    let text = std::fs::read_to_string(&json).ok()?;
    match serde_json::from_str::<Value>(&text) {
        Ok(doc) => key
            .split('.')
            .try_fold(&doc, |node, part| node.get(part))
            .cloned(),
        Err(e) => {
            warn_once(
                &WARNED_STORE,
                &format!(
                    "cannot read {}: {e}; using default settings",
                    json.display()
                ),
            );
            None
        }
    }
}

fn read_store(db: &Path, key: &str) -> Result<Option<Value>, String> {
    use rusqlite::{Connection, OpenFlags, OptionalExtension};
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_millis(2000))
        .map_err(|e| e.to_string())?;
    let raw: Option<String> = conn
        .query_row(
            "SELECT value_json FROM config WHERE tier = 'global' AND scope = '' AND key = ?1",
            [key],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    raw.map(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
        .transpose()
}

/// Names the pre-0.1.5 standalone config file when it still exists, once.
fn old_file_notice(home: &Path) {
    for dir in ["playbook-telemetry", "playbook-usage"] {
        let old = home.join(".config").join(dir).join("config.json");
        if old.is_file() {
            warn_once(
                &WARNED_OLD_FILE,
                &format!(
                    "{} is no longer read; settings now come from {} (set them with `playbook config set usage.<key> <value>`)",
                    old.display(),
                    root(home).display()
                ),
            );
            return;
        }
    }
}

/// The value of a usage key for the user at `home`: the stored value when it
/// is valid, else the default. `None` for a key that is not a usage key.
pub fn resolve(home: &Path, key: &str) -> Option<Value> {
    let default = default_value(key)?;
    old_file_notice(home);
    let Some(value) = stored(home, key) else {
        return Some(default);
    };
    match validate(key, &value) {
        Ok(()) => Some(value),
        Err(why) => {
            warn_once(
                &WARNED_VALUE,
                &format!("ignoring invalid value {value} for {key}: {why}"),
            );
            Some(default)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::scratch_dir;

    fn home(tag: &str) -> PathBuf {
        let h = scratch_dir(tag);
        std::fs::create_dir_all(root(&h)).unwrap();
        h
    }

    fn put_store(home: &Path, key: &str, json: &str) {
        let conn = rusqlite::Connection::open(root(home).join("playbook.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS config (tier TEXT NOT NULL, scope TEXT NOT NULL, key TEXT NOT NULL, value_json TEXT NOT NULL, updated_at INTEGER NOT NULL, PRIMARY KEY (tier, scope, key)) WITHOUT ROWID;",
        )
        .unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO config VALUES ('global', '', ?1, ?2, 0)",
            [key, json],
        )
        .unwrap();
    }

    #[test]
    fn defaults_apply_when_nothing_is_stored() {
        let h = home("cfg-none");
        assert_eq!(resolve(&h, "usage.theme"), Some(Value::from("default")));
        assert_eq!(
            resolve(&h, "usage.autoIngestMinutes"),
            Some(Value::from(10))
        );
        assert_eq!(resolve(&h, "model.default"), None);
    }

    #[test]
    fn the_playbook_store_is_read_and_unrelated_keys_are_ignored() {
        let h = home("cfg-store");
        put_store(&h, "usage.theme", "\"dark\"");
        put_store(&h, "usage.budget.monthlyUsd", "200");
        put_store(&h, "models.default", "\"opus\"");
        assert_eq!(resolve(&h, "usage.theme"), Some(Value::from("dark")));
        assert_eq!(
            resolve(&h, "usage.budget.monthlyUsd"),
            Some(Value::from(200))
        );
        assert_eq!(resolve(&h, "usage.weekStart"), Some(Value::from("monday")));
    }

    #[test]
    fn the_legacy_nested_file_is_read_when_there_is_no_store() {
        let h = home("cfg-legacy");
        std::fs::write(
            root(&h).join("config.json"),
            r#"{"usage":{"weekStart":"sunday","budget":{"weeklyUsd":50}},"other":1}"#,
        )
        .unwrap();
        assert_eq!(resolve(&h, "usage.weekStart"), Some(Value::from("sunday")));
        assert_eq!(resolve(&h, "usage.budget.weeklyUsd"), Some(Value::from(50)));
        assert_eq!(resolve(&h, "usage.theme"), Some(Value::from("default")));
    }

    #[test]
    fn an_invalid_value_falls_back_to_the_default() {
        let h = home("cfg-invalid");
        put_store(&h, "usage.theme", "\"neon\"");
        put_store(&h, "usage.refreshSeconds", "-3");
        assert_eq!(resolve(&h, "usage.theme"), Some(Value::from("default")));
        assert_eq!(resolve(&h, "usage.refreshSeconds"), Some(Value::from(5)));
    }

    #[test]
    fn a_corrupt_file_or_store_falls_back_to_defaults() {
        let h = home("cfg-corrupt-json");
        std::fs::write(root(&h).join("config.json"), "{ not json").unwrap();
        assert_eq!(resolve(&h, "usage.theme"), Some(Value::from("default")));
        let h2 = home("cfg-corrupt-db");
        std::fs::write(
            root(&h2).join("playbook.db"),
            b"this is not a database at all",
        )
        .unwrap();
        assert_eq!(resolve(&h2, "usage.theme"), Some(Value::from("default")));
    }

    #[test]
    fn every_key_has_a_valid_default() {
        for key in KEYS {
            let d = default_value(key).unwrap();
            assert!(validate(key, &d).is_ok(), "{key}");
        }
    }
}
