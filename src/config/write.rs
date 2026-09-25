// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Validated, locked writes to playbook's own tiered config files: the
//! write-side counterpart to `resolve` in `config::mod`.

use super::{keys, ConfigError};
use crate::common::atomic::with_dir_lock;
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Which tier's file `set` writes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Global,
    Org,
    Repo,
}

/// Validate `key` and `value` against `keys::KNOWN_KEYS`, then merge them
/// into `tier`'s config file under `home`, creating the file and its parent
/// directory if absent. `repo_slug` (as `<owner>/<repo>`) is required for
/// the org and repo tiers, since their paths are built from it; the global
/// tier ignores it. Every validation runs before any write happens, so a
/// rejected call never touches the file. Concurrent writers to the same
/// tier file are serialized through `common::atomic::with_dir_lock`.
pub fn set(
    tier: Tier,
    key: &str,
    value: Value,
    home: &Path,
    repo_slug: Option<&str>,
) -> Result<(), ConfigError> {
    validate_key_and_value(key, &value)?;
    let path = tier_path(tier, home, repo_slug)?;

    // Created up front, outside the lock: the lock directory is a sibling
    // of `path`, so `with_dir_lock`'s `mkdir` cannot itself succeed until
    // this parent exists. Doing this inside the locked closure instead
    // would make every writer's very first `mkdir` fail with "no such
    // file or directory", indistinguishable from "already locked", until
    // whichever writer happens to exhaust its retries first falls through
    // and creates it, a bootstrapping race that starves the others under
    // real contention. `create_dir_all` is safe to call unprotected: it
    // treats the directory already existing (created concurrently by
    // another writer) as success, not an error.
    let parent = path
        .parent()
        .expect("a tier path always has a parent directory");
    fs::create_dir_all(parent).map_err(|_| ConfigError::DirectoryUnwritable(parent.to_path_buf()))?;

    let lock_path = PathBuf::from(format!("{}.lock", path.display()));
    let (acquired, result) = with_dir_lock(&lock_path, 50, Duration::from_millis(10), || {
        merge_and_write(&path, key, value)
    });
    if acquired {
        let _ = fs::remove_dir(&lock_path);
    }
    result
}

/// Reject an unknown key, a value whose JSON type does not match
/// `keys::default_value(key)`, or an out-of-enum string, before any file is
/// touched.
fn validate_key_and_value(key: &str, value: &Value) -> Result<(), ConfigError> {
    let default = keys::default_value(key).ok_or_else(|| ConfigError::UnknownKey(key.to_string()))?;
    let expected = match default {
        Value::Bool(_) => "boolean",
        Value::String(_) => "string",
        _ => "value",
    };
    let type_matches = matches!(
        (&default, value),
        (Value::Bool(_), Value::Bool(_)) | (Value::String(_), Value::String(_))
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
    Ok(())
}

/// Build `tier`'s file path under `home`, the same construction `resolve`
/// uses for the repo and org tiers. `None` `repo_slug` on the org or repo
/// tier is an error rather than a guess, since there is no `<owner>/<repo>`
/// to build the path from.
fn tier_path(tier: Tier, home: &Path, repo_slug: Option<&str>) -> Result<PathBuf, ConfigError> {
    let root = crate::common::paths::playbook_root_from(home);
    match tier {
        Tier::Global => Ok(root.join("config.json")),
        Tier::Org => {
            let (owner, _) = repo_slug
                .and_then(|slug| slug.split_once('/'))
                .ok_or(ConfigError::MissingRepoContext)?;
            Ok(root.join("orgs").join(owner).join("config.json"))
        }
        Tier::Repo => {
            let (owner, repo) = repo_slug
                .and_then(|slug| slug.split_once('/'))
                .ok_or(ConfigError::MissingRepoContext)?;
            Ok(root
                .join("repos")
                .join(owner)
                .join(repo)
                .join(".config")
                .join("config.json"))
        }
    }
}

/// Read `path` (if any), merge `key`'s dotted-path value into it, and write
/// the result back atomically. Runs inside the caller's directory lock, so
/// the read-merge-write is not split across separately-locked steps. The
/// parent directory is already guaranteed to exist by `set` before this
/// runs.
fn merge_and_write(path: &Path, key: &str, value: Value) -> Result<(), ConfigError> {
    let parent = path
        .parent()
        .expect("a tier path always has a parent directory");

    let mut root = match fs::read_to_string(path) {
        Ok(raw) => {
            let parsed: Value = serde_json::from_str(&raw)
                .map_err(|_| ConfigError::Malformed(path.to_path_buf()))?;
            if !parsed.is_object() {
                return Err(ConfigError::Malformed(path.to_path_buf()));
            }
            parsed
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
        Err(_) => return Err(ConfigError::Malformed(path.to_path_buf())),
    };

    dotted_set(&mut root, path, key, value)?;

    let serialized =
        serde_json::to_string_pretty(&root).expect("a merged config object always serializes");
    write_atomically(path, &serialized).map_err(|_| ConfigError::DirectoryUnwritable(parent.to_path_buf()))
}

/// Set `key`'s dotted path inside `root`, creating any missing intermediate
/// object along the way and preserving every other existing key. Fails
/// rather than overwriting if an existing segment is present but is not
/// itself an object, since silently replacing it could destroy unrelated
/// tier content.
fn dotted_set(root: &mut Value, path: &Path, key: &str, value: Value) -> Result<(), ConfigError> {
    let mut segments = key.split('.');
    let last = segments
        .next_back()
        .expect("keys::KNOWN_KEYS entries are non-empty");
    let mut current = root;
    for segment in segments {
        let obj = current
            .as_object_mut()
            .ok_or_else(|| ConfigError::Malformed(path.to_path_buf()))?;
        current = obj
            .entry(segment.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    let obj = current
        .as_object_mut()
        .ok_or_else(|| ConfigError::Malformed(path.to_path_buf()))?;
    obj.insert(last.to_string(), value);
    Ok(())
}

/// Write `contents` to `path` via a temp file in the same directory plus a
/// rename, mirroring `common::counter::write_atomically`, so a reader never
/// observes a partially written tier file.
fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let parent = path
        .parent()
        .expect("a tier path always has a parent directory");
    let tmp_path = parent.join(format!(
        ".tmp-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let mut tmp_file = fs::File::create(&tmp_path)?;
    let result = tmp_file.write_all(contents.as_bytes());
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
        return result;
    }
    fs::rename(&tmp_path, path)
}
