// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Copies a finished migration left behind (ADR 0047 decision 5): old
//! databases renamed to `playbook.db.migrated`, `.migrated.1` and so on with
//! their `-wal` and `-shm` files, the old `memory.graph.json.migrated`
//! files, worktree sweep markers renamed to `.migrated`, and a `.DS_Store`
//! in the playbook folder. `disk clean` removes them once they are 7 days
//! old, and only while the database that took their data is in place. The
//! newest database backup and the newest graph backup always stay.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a backup stays before the cleanup may remove it.
pub const BACKUP_IDLE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

const DB_BACKUP: &str = "playbook.db.migrated";
const GRAPH_BACKUP: &str = "memory.graph.json.migrated";
const SWEEP_MARKER: &str = "worktree-sweep-marker-";

/// One leftover file and why it must stay, if it must.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leftover {
    pub path: PathBuf,
    pub keep: Option<String>,
    /// A `.DS_Store`, removed at any age.
    pub junk: bool,
    /// Inside the memory store, which the cleanup otherwise never enters.
    pub in_memory_store: bool,
}

/// Every leftover under the playbook folder of `home`, regular files only.
pub fn leftovers(home: &Path) -> Vec<Leftover> {
    let root = crate::common::paths::playbook_root_from(home);
    let data = playbook_data::data_dir_from(home);
    // Opened only when a backup is there, so a home without one is not
    // touched at all.
    let config_ok = std::cell::OnceCell::new();
    let config_ok =
        || *config_ok.get_or_init(|| config_db_current(&playbook_data::config::db_in(&data)));
    let graph_ok = std::cell::OnceCell::new();
    let graph_ok =
        || *graph_ok.get_or_init(|| memory_db_has_graph(&playbook_data::memory::db_in(&data)));
    let mut found = Vec::new();
    let root_files = files(&root);
    let db_newest = newest_family(&root_files, db_family);
    for path in &root_files {
        let name = file_name(path);
        if name == ".DS_Store" {
            found.push(leftover(path, None, true, false));
        } else if let Some(family) = db_family(&name) {
            let keep = if !config_ok() {
                Some("data/config.db is missing or not current".to_string())
            } else if db_newest.as_deref() == Some(family.as_str()) {
                Some("the newest backup".to_string())
            } else {
                None
            };
            found.push(leftover(path, keep, false, false));
        } else if name.starts_with(SWEEP_MARKER) && name.contains(".migrated") {
            let keep =
                (!config_ok()).then(|| "data/config.db is missing or not current".to_string());
            found.push(leftover(path, keep, false, false));
        }
    }
    let memory_files = files(&root.join("memory"));
    let graph_newest = newest_family(&memory_files, graph_family);
    for path in &memory_files {
        let name = file_name(path);
        if let Some(family) = graph_family(&name) {
            let keep = if !graph_ok() {
                Some("data/memory.db holds no graph".to_string())
            } else if graph_newest.as_deref() == Some(family.as_str()) {
                Some("the newest backup".to_string())
            } else {
                None
            };
            found.push(leftover(path, keep, false, true));
        }
    }
    found
}

fn leftover(path: &Path, keep: Option<String>, junk: bool, in_memory_store: bool) -> Leftover {
    Leftover {
        path: path.to_path_buf(),
        keep,
        junk,
        in_memory_store,
    }
}

/// The regular files directly in `dir`. Symlinks and folders are skipped.
fn files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `playbook.db.migrated` or `playbook.db.migrated.<n>`, with or without a
/// `-wal` or `-shm` suffix, as the family name without the suffix.
fn db_family(name: &str) -> Option<String> {
    numbered(strip_sidecar(name), DB_BACKUP)
}

/// `memory.graph.json.migrated` or `memory.graph.json.migrated.<n>`.
fn graph_family(name: &str) -> Option<String> {
    numbered(name, GRAPH_BACKUP)
}

fn strip_sidecar(name: &str) -> &str {
    name.strip_suffix("-wal")
        .or_else(|| name.strip_suffix("-shm"))
        .unwrap_or(name)
}

fn numbered(name: &str, base: &str) -> Option<String> {
    let rest = name.strip_prefix(base)?;
    let ok = rest.is_empty()
        || rest
            .strip_prefix('.')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    ok.then(|| name.to_string())
}

/// The family whose main file changed last. A family with no main file
/// counts by its newest member.
fn newest_family(paths: &[PathBuf], family: fn(&str) -> Option<String>) -> Option<String> {
    paths
        .iter()
        .filter_map(|p| {
            let name = file_name(p);
            let fam = family(&name)?;
            let at = std::fs::metadata(p).and_then(|m| m.modified()).ok()?;
            let main = name == fam;
            Some((main, at, fam))
        })
        .max_by_key(|(main, at, _)| (*main, *at))
        .map(|(_, _, fam)| fam)
}

/// `config.db` exists and has every table of the current schema, so the
/// old database's rows have a home.
fn config_db_current(db: &Path) -> bool {
    tables_present(db, &["meta", "config", "state"])
}

/// `memory.db` exists and holds at least one node.
fn memory_db_has_graph(db: &Path) -> bool {
    open(db).is_some_and(|conn| {
        conn.query_row("SELECT count(*) FROM nodes", [], |r| r.get::<_, i64>(0))
            .is_ok_and(|n| n > 0)
    })
}

fn tables_present(db: &Path, names: &[&str]) -> bool {
    open(db).is_some_and(|conn| {
        names.iter().all(|t| {
            conn.query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [t],
                |_| Ok(()),
            )
            .is_ok()
        })
    })
}

fn open(db: &Path) -> Option<rusqlite::Connection> {
    if !db.is_file() {
        return None;
    }
    rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()
}

/// The total size of the leftovers, for the doctor and `disk status`.
pub fn total_bytes(found: &[Leftover]) -> u64 {
    found
        .iter()
        .filter_map(|l| std::fs::metadata(&l.path).ok())
        .map(|m| m.len())
        .sum()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::SystemTime;

    fn scratch(tag: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!(
            "pb-disk-backups-{}-{tag}",
            crate::testing::run_id()
        ));
        fs::create_dir_all(home.join(".config/playbook/memory")).unwrap();
        fs::create_dir_all(home.join(".config/playbook/data")).unwrap();
        home
    }

    fn db(path: &Path, sql: &str) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(sql).unwrap();
    }

    fn set_mtime(path: &Path, secs_ago: u64) {
        let at = SystemTime::now() - Duration::from_secs(secs_ago);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    fn reasons(found: &[Leftover]) -> Vec<(String, Option<String>)> {
        found
            .iter()
            .map(|l| (file_name(&l.path), l.keep.clone()))
            .collect()
    }

    #[test]
    fn names_match_only_the_backup_families() {
        assert_eq!(db_family("playbook.db.migrated"), Some(DB_BACKUP.into()));
        assert_eq!(
            db_family("playbook.db.migrated.15-wal"),
            Some("playbook.db.migrated.15".into())
        );
        assert_eq!(db_family("playbook.db"), None);
        assert_eq!(db_family("playbook.db-wal"), None);
        assert_eq!(db_family("playbook.db.migrated.x"), None);
        assert_eq!(db_family("playbook.db.migrated."), None);
        assert_eq!(
            graph_family("memory.graph.json.migrated.1"),
            Some("memory.graph.json.migrated.1".into())
        );
        assert_eq!(graph_family("memory.graph.json"), None);
    }

    #[test]
    fn the_newest_backups_stay_and_nothing_goes_without_its_database() {
        let home = scratch("families");
        let root = home.join(".config/playbook");
        for (name, ago) in [
            ("playbook.db.migrated", 900_000),
            ("playbook.db.migrated-wal", 900_000),
            ("playbook.db.migrated.1", 800_000),
            ("playbook.db.migrated.1-shm", 100),
            ("playbook.db", 10),
            (".DS_Store", 10),
            ("worktree-sweep-marker-x.migrated", 900_000),
            ("memory/memory.graph.json.migrated", 900_000),
            ("memory/memory.graph.json.migrated.1", 950_000),
            ("memory/fact.md", 900_000),
        ] {
            fs::write(root.join(name), b"x").unwrap();
            set_mtime(&root.join(name), ago);
        }
        let without = reasons(&leftovers(&home));
        assert!(
            without
                .iter()
                .filter(|(n, _)| n != ".DS_Store")
                .all(|(_, keep)| keep.is_some()),
            "{without:?}"
        );
        db(
            &root.join("data/config.db"),
            playbook_data::config::SCHEMA_SQL,
        );
        db(
            &root.join("data/memory.db"),
            &format!(
                "{}; INSERT INTO nodes (id, file, scope, type) VALUES ('a', 'a.md', 'global', 'fact');",
                playbook_data::memory::SCHEMA_SQL
            ),
        );
        let with = reasons(&leftovers(&home));
        let keep = |name: &str| {
            with.iter()
                .find(|(n, _)| n == name)
                .unwrap_or_else(|| panic!("{name} missing: {with:?}"))
                .1
                .clone()
        };
        assert_eq!(keep("playbook.db.migrated"), None);
        assert_eq!(keep("playbook.db.migrated-wal"), None);
        assert_eq!(
            keep("playbook.db.migrated.1").as_deref(),
            Some("the newest backup")
        );
        assert_eq!(
            keep("playbook.db.migrated.1-shm").as_deref(),
            Some("the newest backup")
        );
        assert_eq!(keep("worktree-sweep-marker-x.migrated"), None);
        assert_eq!(
            keep("memory.graph.json.migrated").as_deref(),
            Some("the newest backup")
        );
        assert_eq!(keep("memory.graph.json.migrated.1"), None);
        assert_eq!(keep(".DS_Store"), None);
        assert!(with
            .iter()
            .all(|(n, _)| n != "playbook.db" && n != "fact.md"));
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_symlink_named_like_a_backup_is_not_listed() {
        let home = scratch("symlink");
        let root = home.join(".config/playbook");
        fs::write(home.join("victim"), b"keep").unwrap();
        std::os::unix::fs::symlink(home.join("victim"), root.join("playbook.db.migrated.3"))
            .unwrap();
        assert!(leftovers(&home).is_empty());
        fs::remove_dir_all(&home).unwrap();
    }
}
