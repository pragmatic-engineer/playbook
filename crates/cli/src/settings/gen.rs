// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Settings seed generator behind `playbook settings gen SRC SEED`. It
//! reads a live `settings.json` (SRC) and the current seed (SEED), and prints
//! a new seed: the shippable keys of SRC with `.hooks` filtered down to
//! `playbook hook <name>` entries, and the `permissions` block of SEED. The
//! seed is the only source of the shipped permissions (ADR 0047), so the
//! maintainer's own permissions in SRC never ship.
//!
//! Top-level keys go through `settings::keys::SHIPPABLE_KEYS`, an allowlist:
//! any key not on it is dropped and named on stderr.
//!
//! Output is pretty JSON with two-space indent and a trailing newline. Non
//! ASCII text is written as raw UTF-8; `tests/settings_gen.rs` pins this.

use crate::settings::keys::{shipped_permissions, SHIPPABLE_KEYS};
use crate::HookName;
use clap::ValueEnum;
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

/// Why generation stopped before it produced output. The caller in
/// `src/main.rs` prints it and exits 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenError(pub String);

impl std::fmt::Display for GenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Read `path` as JSON, with one error for an unreadable file and another
/// for invalid JSON. The shape is checked by the caller.
fn load_json(path: &Path, label: &str) -> Result<Value, GenError> {
    let text = fs::read_to_string(path)
        .map_err(|_| GenError(format!("{label} not readable: {}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|_| GenError(format!("{label} is not valid JSON: {}", path.display())))
}

/// True for a bare `playbook hook <name>` command whose name is a real hook,
/// the only hook command form `wire::wire` writes.
fn is_safe_hook_command(command: &str) -> bool {
    command
        .strip_prefix(crate::init::wire::HOOK_COMMAND_PREFIX)
        .is_some_and(is_valid_hook_name)
}

/// `[a-z][a-z0-9-]*` shape, AND a real `HookName`: a shape-valid but
/// nonexistent hook name (a typo, a dangling reference) is filtered out too.
fn is_valid_hook_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let shape_ok = first.is_ascii_lowercase()
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    shape_ok && HookName::from_str(name, true).is_ok()
}

/// Within each event, keep the hook groups that have at least one entry
/// whose command passes `is_safe_hook_command`, and within a kept group only
/// those entries. An event with no group left is dropped. The `continue`
/// arms cannot run: `validate_hooks_shape` rejects a malformed shape first.
fn filter_hooks(hooks: &Map<String, Value>) -> Map<String, Value> {
    let mut result = Map::new();
    for (event, groups_value) in hooks {
        let Some(groups) = groups_value.as_array() else {
            continue;
        };
        let mut new_groups = Vec::new();
        for group_value in groups {
            let Some(group) = group_value.as_object() else {
                continue;
            };
            let safe: Vec<Value> = group
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|hook| {
                    let command = hook
                        .as_object()
                        .and_then(|h| h.get("command"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    is_safe_hook_command(command)
                })
                .cloned()
                .collect();
            if !safe.is_empty() {
                let mut new_group = group.clone();
                new_group.insert("hooks".to_string(), Value::Array(safe));
                new_groups.push(Value::Object(new_group));
            }
        }
        if !new_groups.is_empty() {
            result.insert(event.clone(), Value::Array(new_groups));
        }
    }
    result
}

/// Removes every key not in `allowed`, returning the dropped keys' names in
/// their original order. `shift_remove` keeps the order of the keys that
/// stay; with `preserve_order`, plain `Map::remove` swaps the last key into
/// the gap.
fn keep_only(map: &mut Map<String, Value>, allowed: &[&str]) -> Vec<String> {
    let dropped: Vec<String> = map
        .keys()
        .filter(|k| !allowed.contains(&k.as_str()))
        .cloned()
        .collect();
    for key in &dropped {
        map.shift_remove(key);
    }
    dropped
}

/// Set `.permissions` to the seed's block, keep only the shippable top-level
/// keys, and filter `.hooks` when present. `Map::insert` keeps a present key
/// in place and appends a new one, so the key order of `src` decides the key
/// order of the result.
fn build(mut src: Map<String, Value>, perms: Value) -> Map<String, Value> {
    src.insert("permissions".to_string(), perms);

    let dropped = keep_only(&mut src, SHIPPABLE_KEYS);
    if !dropped.is_empty() {
        eprintln!("gen-shared-settings: dropped {}", dropped.join(", "));
    }

    if let Some(hooks_value) = src.get("hooks").cloned() {
        let filtered = match hooks_value {
            Value::Object(hooks_map) => filter_hooks(&hooks_map),
            _ => Map::new(),
        };
        src.insert("hooks".to_string(), Value::Object(filtered));
    }
    src
}

/// Reject a malformed `.hooks` shape with an error. The `justfile` recipe
/// writes the output to a temp file and moves it over the seed only on exit
/// 0, so an error keeps the committed seed. Emitting a seed with no hooks on
/// exit 0 would replace it with one that wires nothing.
fn validate_hooks_shape(src: &Map<String, Value>, src_path: &Path) -> Result<(), GenError> {
    let Some(hooks) = src.get("hooks") else {
        return Ok(());
    };
    let bad = |what: &str| {
        Err(GenError(format!(
            "{what} in {}; refusing to emit a seed that would silently wire no hooks",
            src_path.display()
        )))
    };
    let Some(events) = hooks.as_object() else {
        return bad(".hooks is present but is not an object");
    };
    for (event, groups) in events {
        let Some(groups) = groups.as_array() else {
            return bad(&format!(".hooks.{event} is not an array"));
        };
        for (i, group) in groups.iter().enumerate() {
            if !group.is_object() {
                return bad(&format!(".hooks.{event}[{i}] is not an object"));
            }
        }
    }
    Ok(())
}

/// Load `src_path` and `seed_path`, take the seed's `permissions` block (an
/// object with a non-empty `allow` array, else an error), then run `build`
/// and serialise the result as pretty JSON plus a trailing newline. The
/// permissions of SRC are never used, even when the seed has none.
pub fn generate(src_path: &Path, seed_path: &Path) -> Result<String, GenError> {
    let src_value = load_json(src_path, "source settings")?;
    let seed_value = load_json(seed_path, "seed")?;

    let Some(perms_value) = shipped_permissions(&seed_value).cloned() else {
        return Err(GenError(format!(
            "seed must hold a permissions object with a non-empty allow array: {}",
            seed_path.display()
        )));
    };

    let Value::Object(src) = src_value else {
        return Err(GenError(format!(
            "source settings is not a JSON object: {}",
            src_path.display()
        )));
    };

    validate_hooks_shape(&src, src_path)?;

    let result = build(src, perms_value);
    Ok(format!("{:#}\n", Value::Object(result)))
}
