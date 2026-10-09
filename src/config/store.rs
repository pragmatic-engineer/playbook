// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! SQLite storage for playbook's tiered config: one `playbook.db` under the
//! playbook root replaces the three JSON file shapes (global, org, repo).
//!
//! Rows are `(tier, scope, key) -> value_json`, where `scope` is empty for the
//! global tier, the owner for an org and the whole slug for a repo. The first
//! open imports any legacy `config.json` files in one transaction and renames
//! them to `config.json.migrated`. WAL mode and a busy timeout replace the
//! directory lock the JSON files needed.

use super::write::Tier;
use super::{keys, ConfigError};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SCHEMA_SQL: &str = "\
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS config (
    tier TEXT NOT NULL,
    scope TEXT NOT NULL,
    key TEXT NOT NULL,
    value_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (tier, scope, key)
) WITHOUT ROWID;";

/// The meta row that records the legacy JSON import ran.
const IMPORTED_FLAG: &str = "legacy_imported";

/// Where the database lives under `root`.
pub fn db_path(root: &Path) -> PathBuf {
    root.join("playbook.db")
}

/// Whether there is anything to read: a database, or legacy JSON files that
/// the first open would import. When false, every key resolves to its default
/// and nothing needs creating.
pub fn has_data(root: &Path) -> bool {
    db_path(root).exists()
        || super::global_config_path(root).exists()
        || super::any_scoped_config(root)
}

fn tier_str(tier: Tier) -> &'static str {
    match tier {
        Tier::Global => "global",
        Tier::Org => "org",
        Tier::Repo => "repo",
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A database error is reported as the database path, the same shape a
/// malformed JSON file had, so callers keep one error case for "config store
/// unreadable".
fn broken(root: &Path) -> impl Fn(rusqlite::Error) -> ConfigError + '_ {
    move |_| ConfigError::Malformed(db_path(root))
}

/// Open (creating if needed) the database under `root`, apply the schema and
/// run the one-time legacy import.
pub fn open(root: &Path) -> Result<Connection, ConfigError> {
    fs::create_dir_all(root).map_err(|_| ConfigError::DirectoryUnwritable(root.to_path_buf()))?;
    let conn = Connection::open(db_path(root)).map_err(broken(root))?;
    // Before any statement that can contend for the write lock: the default
    // busy timeout is 0, so a concurrent first open would fail at once.
    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(broken(root))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(broken(root))?;
    conn.execute_batch(SCHEMA_SQL).map_err(broken(root))?;
    import_legacy_once(&conn, root)?;
    Ok(conn)
}

/// The value of `key` per tier, for the repo slug and its owner.
/// Precedence is the caller's: this only reports what each tier holds.
pub struct TierValues {
    pub repo: Option<Value>,
    pub org: Option<Value>,
    pub global: Option<Value>,
}

/// Look `key` up in the repo, org and global tiers with one query.
pub fn lookup(root: &Path, key: &str, repo_slug: Option<&str>) -> Result<TierValues, ConfigError> {
    let mut out = TierValues {
        repo: None,
        org: None,
        global: None,
    };
    if !has_data(root) {
        return Ok(out);
    }
    let conn = open(root)?;
    let (repo_scope, org_scope) = match repo_slug.and_then(|s| s.split_once('/')) {
        Some((owner, _)) => (repo_slug.unwrap_or_default(), owner),
        None => ("\u{0}", "\u{0}"),
    };
    let mut stmt = conn
        .prepare(
            "SELECT tier, value_json FROM config WHERE key = ?1 AND \
             ((tier = 'repo' AND scope = ?2) OR (tier = 'org' AND scope = ?3) \
              OR (tier = 'global' AND scope = ''))",
        )
        .map_err(broken(root))?;
    let rows = stmt
        .query_map(params![key, repo_scope, org_scope], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(broken(root))?;
    for row in rows {
        let (tier, raw) = row.map_err(broken(root))?;
        let value: Value =
            serde_json::from_str(&raw).map_err(|_| ConfigError::Malformed(db_path(root)))?;
        match tier.as_str() {
            "repo" => out.repo = Some(value),
            "org" => out.org = Some(value),
            _ => out.global = Some(value),
        }
    }
    Ok(out)
}

/// Whether any org or repo tier value exists. When none does, resolving with
/// or without a slug gives the same answer, so a caller can skip the slug
/// lookup (a `git` spawn).
pub fn any_scoped(root: &Path) -> bool {
    if db_path(root).exists() {
        let Ok(conn) = open(root) else { return false };
        return conn
            .query_row(
                "SELECT 1 FROM config WHERE tier != 'global' LIMIT 1",
                [],
                |_| Ok(()),
            )
            .optional()
            .ok()
            .flatten()
            .is_some();
    }
    super::any_scoped_config(root)
}

/// The scope string a write to `tier` uses for `repo_slug`.
fn scope_for(tier: Tier, repo_slug: Option<&str>) -> Result<String, ConfigError> {
    match tier {
        Tier::Global => Ok(String::new()),
        Tier::Org => repo_slug
            .and_then(|s| s.split_once('/'))
            .map(|(owner, _)| owner.to_string())
            .ok_or(ConfigError::MissingRepoContext),
        Tier::Repo => repo_slug
            .filter(|s| s.split_once('/').is_some())
            .map(str::to_string)
            .ok_or(ConfigError::MissingRepoContext),
    }
}

/// Store `value` for `key` in `tier`. The caller has validated both.
pub fn put(
    root: &Path,
    tier: Tier,
    repo_slug: Option<&str>,
    key: &str,
    value: &Value,
) -> Result<(), ConfigError> {
    let scope = scope_for(tier, repo_slug)?;
    let conn = open(root)?;
    upsert(&conn, tier_str(tier), &scope, key, value).map_err(broken(root))
}

fn upsert(
    conn: &Connection,
    tier: &str,
    scope: &str,
    key: &str,
    value: &Value,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO config (tier, scope, key, value_json, updated_at) VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT (tier, scope, key) DO UPDATE SET \
         value_json = excluded.value_json, updated_at = excluded.updated_at",
        params![tier, scope, key, value.to_string(), now()],
    )
    .map(|_| ())
}

/// Every stored value as one JSON document, for `playbook config export`:
/// `{"version": 1, "global": {key: value}, "orgs": {owner: {...}}, "repos": {slug: {...}}}`.
pub fn export(root: &Path) -> Result<Value, ConfigError> {
    let mut global = Map::new();
    let mut orgs: Map<String, Value> = Map::new();
    let mut repos: Map<String, Value> = Map::new();
    if has_data(root) {
        let conn = open(root)?;
        let mut stmt = conn
            .prepare("SELECT tier, scope, key, value_json FROM config ORDER BY tier, scope, key")
            .map_err(broken(root))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(broken(root))?;
        for row in rows {
            let (tier, scope, key, raw) = row.map_err(broken(root))?;
            let value: Value =
                serde_json::from_str(&raw).map_err(|_| ConfigError::Malformed(db_path(root)))?;
            let target = match tier.as_str() {
                "org" => orgs
                    .entry(scope)
                    .or_insert_with(|| Value::Object(Map::new())),
                "repo" => repos
                    .entry(scope)
                    .or_insert_with(|| Value::Object(Map::new())),
                _ => {
                    global.insert(key, value);
                    continue;
                }
            };
            if let Some(obj) = target.as_object_mut() {
                obj.insert(key, value);
            }
        }
    }
    Ok(serde_json::json!({
        "version": 1,
        "global": global,
        "orgs": orgs,
        "repos": repos,
    }))
}

/// Load an `export` document back in one transaction. Every key and value is
/// validated first, so a bad document changes nothing.
pub fn import(root: &Path, doc: &Value) -> Result<usize, ConfigError> {
    let mut rows: Vec<(&'static str, String, String, Value)> = Vec::new();
    let mut take = |tier: &'static str, scope: &str, map: &Value| -> Result<(), ConfigError> {
        let Some(obj) = map.as_object() else {
            return Ok(());
        };
        for (key, value) in obj {
            super::write::validate_key_and_value(key, value)?;
            rows.push((tier, scope.to_string(), key.clone(), value.clone()));
        }
        Ok(())
    };
    take("global", "", doc.get("global").unwrap_or(&Value::Null))?;
    for (tier, field) in [("org", "orgs"), ("repo", "repos")] {
        if let Some(scopes) = doc.get(field).and_then(Value::as_object) {
            for (scope, map) in scopes {
                if tier == "repo" && scope.split_once('/').is_none() {
                    return Err(ConfigError::MissingRepoContext);
                }
                take(tier, scope, map)?;
            }
        }
    }
    let mut conn = open(root)?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(broken(root))?;
    for (tier, scope, key, value) in &rows {
        upsert(&tx, tier, scope, key, value).map_err(broken(root))?;
    }
    tx.commit().map_err(broken(root))?;
    Ok(rows.len())
}

/// One legacy file: the tier and scope it held, and where it is.
struct LegacyFile {
    tier: &'static str,
    scope: String,
    path: PathBuf,
}

/// Every legacy `config.json` under `root`: global, `orgs/<owner>`, and
/// `repos/<slug>/.config` (a slug can have more than two segments).
fn legacy_files(root: &Path) -> Vec<LegacyFile> {
    let mut out = Vec::new();
    let global = super::global_config_path(root);
    if global.exists() {
        out.push(LegacyFile {
            tier: "global",
            scope: String::new(),
            path: global,
        });
    }
    if let Ok(entries) = fs::read_dir(root.join("orgs")) {
        for entry in entries.flatten() {
            let path = entry.path().join("config.json");
            if path.exists() {
                out.push(LegacyFile {
                    tier: "org",
                    scope: entry.file_name().to_string_lossy().into_owned(),
                    path,
                });
            }
        }
    }
    let repos = root.join("repos");
    let mut stack: Vec<(PathBuf, usize)> = vec![(repos.clone(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if dir.file_name().is_some_and(|n| n == ".config") {
            let path = dir.join("config.json");
            if path.exists() {
                let slug = dir
                    .parent()
                    .and_then(|p| p.strip_prefix(&repos).ok())
                    .map(|p| {
                        p.components()
                            .map(|c| c.as_os_str().to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                            .join("/")
                    })
                    .unwrap_or_default();
                out.push(LegacyFile {
                    tier: "repo",
                    scope: slug,
                    path,
                });
            }
            continue;
        }
        if depth < 8 {
            if let Ok(rd) = fs::read_dir(&dir) {
                for e in rd.flatten().filter(|e| e.path().is_dir()) {
                    stack.push((e.path(), depth + 1));
                }
            }
        }
    }
    out
}

/// Import the legacy JSON files once. The flag is checked again inside the
/// write transaction, so two processes opening a fresh database at the same
/// time import exactly once. A file that is not a JSON object fails the whole
/// import with its path and is left untouched, as it failed reads before.
fn import_legacy_once(conn: &Connection, root: &Path) -> Result<(), ConfigError> {
    let flagged = |c: &Connection| -> Result<bool, ConfigError> {
        c.query_row(
            "SELECT 1 FROM meta WHERE key = ?1",
            params![IMPORTED_FLAG],
            |_| Ok(()),
        )
        .optional()
        .map(|r| r.is_some())
        .map_err(broken(root))
    };
    if flagged(conn)? {
        return Ok(());
    }
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(broken(root))?;
    let outcome = (|| -> Result<Vec<PathBuf>, ConfigError> {
        if flagged(conn)? {
            return Ok(Vec::new());
        }
        let mut done = Vec::new();
        for file in legacy_files(root) {
            let raw = fs::read_to_string(&file.path)
                .map_err(|_| ConfigError::Malformed(file.path.clone()))?;
            let parsed: Value = serde_json::from_str(&raw)
                .map_err(|_| ConfigError::Malformed(file.path.clone()))?;
            if !parsed.is_object() {
                return Err(ConfigError::Malformed(file.path.clone()));
            }
            for &key in keys::KNOWN_KEYS {
                if let Some(value) = super::dotted_lookup(&parsed, key) {
                    upsert(conn, file.tier, &file.scope, key, value).map_err(broken(root))?;
                }
            }
            done.push(file.path);
        }
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, '1')",
            params![IMPORTED_FLAG],
        )
        .map_err(broken(root))?;
        Ok(done)
    })();
    match outcome {
        Ok(done) => {
            conn.execute_batch("COMMIT").map_err(broken(root))?;
            for path in done {
                let _ = fs::rename(&path, path.with_extension("json.migrated"));
            }
            Ok(())
        }
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use serde_json::json;

    fn root(tag: &str) -> PathBuf {
        let dir = scratch_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_json(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn nothing_is_created_when_there_is_nothing_to_read() {
        let r = root("store-none");
        let got = lookup(&r, "mode", None).unwrap();
        assert!(got.repo.is_none() && got.org.is_none() && got.global.is_none());
        assert!(!db_path(&r).exists());
    }

    #[test]
    fn a_value_round_trips_per_tier_and_scope() {
        let r = root("store-rt");
        put(&r, Tier::Global, None, "mode", &json!("auto")).unwrap();
        put(&r, Tier::Org, Some("acme/widgets"), "mode", &json!("ask")).unwrap();
        put(&r, Tier::Repo, Some("acme/widgets"), "mode", &json!("auto")).unwrap();
        let got = lookup(&r, "mode", Some("acme/widgets")).unwrap();
        assert_eq!(got.global, Some(json!("auto")));
        assert_eq!(got.org, Some(json!("ask")));
        assert_eq!(got.repo, Some(json!("auto")));
        let other = lookup(&r, "mode", Some("other/thing")).unwrap();
        assert!(other.org.is_none() && other.repo.is_none());
        let none = lookup(&r, "mode", None).unwrap();
        assert!(none.org.is_none() && none.repo.is_none());
    }

    #[test]
    fn a_second_put_replaces_the_value() {
        let r = root("store-replace");
        put(&r, Tier::Global, None, "pr.draft", &json!(true)).unwrap();
        put(&r, Tier::Global, None, "pr.draft", &json!(false)).unwrap();
        assert_eq!(
            lookup(&r, "pr.draft", None).unwrap().global,
            Some(json!(false))
        );
    }

    #[test]
    fn org_and_repo_writes_need_a_slug() {
        let r = root("store-slug");
        assert!(matches!(
            put(&r, Tier::Org, None, "mode", &json!("ask")),
            Err(ConfigError::MissingRepoContext)
        ));
        assert!(matches!(
            put(&r, Tier::Repo, Some("noslash"), "mode", &json!("ask")),
            Err(ConfigError::MissingRepoContext)
        ));
    }

    #[test]
    fn legacy_files_are_imported_once_and_renamed() {
        let r = root("store-import");
        write_json(
            &r.join("config.json"),
            r#"{"mode":"auto","autoReview":{"type":"deep"}}"#,
        );
        write_json(
            &r.join("orgs/acme/config.json"),
            r#"{"pr":{"draft":false}}"#,
        );
        write_json(
            &r.join("repos/acme/widgets/.config/config.json"),
            r#"{"commit":{"signOff":false},"stale.unknown":1}"#,
        );
        write_json(
            &r.join("repos/group/sub/repo/.config/config.json"),
            r#"{"mode":"ask"}"#,
        );

        let got = lookup(&r, "autoReview.type", None).unwrap();
        assert_eq!(got.global, Some(json!("deep")));
        assert_eq!(
            lookup(&r, "pr.draft", Some("acme/x")).unwrap().org,
            Some(json!(false))
        );
        assert_eq!(
            lookup(&r, "commit.signOff", Some("acme/widgets"))
                .unwrap()
                .repo,
            Some(json!(false))
        );
        assert_eq!(
            lookup(&r, "mode", Some("group/sub/repo")).unwrap().repo,
            Some(json!("ask"))
        );
        assert!(!r.join("config.json").exists());
        assert!(r.join("config.json.migrated").exists());
        assert!(r.join("orgs/acme/config.json.migrated").exists());
        assert!(r
            .join("repos/acme/widgets/.config/config.json.migrated")
            .exists());
    }

    #[test]
    fn a_legacy_file_appearing_after_the_import_is_left_alone() {
        let r = root("store-late");
        put(&r, Tier::Global, None, "mode", &json!("ask")).unwrap();
        write_json(&r.join("config.json"), r#"{"mode":"auto"}"#);
        assert_eq!(lookup(&r, "mode", None).unwrap().global, Some(json!("ask")));
        assert!(r.join("config.json").exists());
    }

    #[test]
    fn a_malformed_legacy_file_fails_with_its_path_and_changes_nothing() {
        let r = root("store-bad");
        write_json(&r.join("config.json"), r#"{"mode":"auto"}"#);
        write_json(&r.join("orgs/acme/config.json"), "[1,2]");
        let err = lookup(&r, "mode", None).err().unwrap();
        match err {
            ConfigError::Malformed(p) => assert!(p.ends_with("orgs/acme/config.json"), "{p:?}"),
            other => panic!("{other:?}"),
        }
        assert!(r.join("config.json").exists(), "nothing renamed on failure");
    }

    #[test]
    fn a_corrupt_database_reports_its_path() {
        let r = root("store-corrupt");
        fs::write(
            db_path(&r),
            b"this is not a database file at all, not even close",
        )
        .unwrap();
        assert!(
            matches!(lookup(&r, "mode", None), Err(ConfigError::Malformed(p)) if p == db_path(&r))
        );
    }

    #[test]
    fn any_scoped_sees_org_and_repo_rows_but_not_global() {
        let r = root("store-scoped");
        put(&r, Tier::Global, None, "mode", &json!("ask")).unwrap();
        assert!(!any_scoped(&r));
        put(&r, Tier::Org, Some("a/b"), "mode", &json!("ask")).unwrap();
        assert!(any_scoped(&r));
    }

    #[test]
    fn export_then_import_round_trips_and_validates() {
        let a = root("store-export-a");
        put(&a, Tier::Global, None, "mode", &json!("auto")).unwrap();
        put(&a, Tier::Org, Some("acme/w"), "pr.draft", &json!(false)).unwrap();
        put(&a, Tier::Repo, Some("acme/w"), "fix.maxFiles", &json!(5)).unwrap();
        let doc = export(&a).unwrap();
        assert_eq!(doc["version"], 1);
        assert_eq!(doc["global"]["mode"], "auto");
        assert_eq!(doc["orgs"]["acme"]["pr.draft"], false);
        assert_eq!(doc["repos"]["acme/w"]["fix.maxFiles"], 5);

        let b = root("store-export-b");
        assert_eq!(import(&b, &doc).unwrap(), 3);
        assert_eq!(export(&b).unwrap(), doc);

        let bad = json!({"global": {"mode": "chaos"}});
        assert!(import(&b, &bad).is_err());
        let unknown = json!({"global": {"nope": 1}});
        assert!(matches!(
            import(&b, &unknown),
            Err(ConfigError::UnknownKey(_))
        ));
    }

    #[test]
    fn concurrent_first_opens_import_exactly_once() {
        let r = root("store-race");
        write_json(&r.join("config.json"), r#"{"mode":"auto"}"#);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let r = r.clone();
                std::thread::spawn(move || lookup(&r, "mode", None).unwrap().global)
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), Some(json!("auto")));
        }
        assert!(r.join("config.json.migrated").exists());
    }
}
