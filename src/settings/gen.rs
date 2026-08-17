// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Settings seed generator, ported from `shell/gen-shared-settings.py`. That
//! file's header comment is the specification: replace `.permissions` with a
//! canned permissions object, force `skipAutoPermissionPrompt: false`, strip
//! any pinned `.model`, drop the owner's personal keys, and filter `.hooks`
//! down to entries whose command matches the safety pattern. Everything else
//! passes through unchanged. Output is `json.dumps(result, indent=2)`
//! followed by the newline `print` adds.
//!
//! `PERSONAL_KEYS` below is a DENYLIST of five keys, so any personal key not
//! on the list still leaks into the generated seed. That is a known defect
//! of the python original, tracked and owned elsewhere; this port keeps the
//! same denylist unchanged rather than fixing it here.
//!
//! Three divergences from the python original, all deliberate:
//!
//! - **Non-ASCII output.** python's `json.dumps` defaults to
//!   `ensure_ascii=True` and escapes every non-ASCII character to `\uXXXX`;
//!   `serde_json::to_string_pretty` writes raw UTF-8 instead. No real
//!   settings file has a non-ASCII byte today; `tests/settings_gen.rs`
//!   pins the direction of the divergence with a dedicated fixture rather
//!   than leaving it to this comment.
//! - **`SAFETY_RE`.** `is_safe_hook_command` below reimplements python's
//!   `SAFETY_RE.fullmatch` as explicit string matching rather than adding a
//!   `regex` dependency for one fixed-shape pattern; the set of accepted
//!   strings is identical, only the mechanism differs.
//! - **`PERMS`'s default path.** python defaults an omitted `PERMS` argument
//!   to `<repo root>/permissions.shared.json`, computed from the running
//!   script's own path. An installed `playbook` binary has no script-relative
//!   repo root to infer, so this port makes `PERMS` a required argument
//!   instead of guessing a path that could silently be wrong. No ported
//!   scenario exercises the omitted-PERMS default, and the Makefile
//!   invocation this serves already passes both paths explicitly.

use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

/// Denylist of top-level keys treated as personal, not product, config: an
/// owner's model pin, UI preferences and notification routing. Ported
/// unchanged from `shell/gen-shared-settings.py`'s `PERSONAL_KEYS`; any key
/// NOT on this list still leaks into the generated seed, a known, separately
/// owned defect not fixed here.
const PERSONAL_KEYS: [&str; 5] = [
    "model",
    "effortLevel",
    "theme",
    "preferredNotifChannel",
    "prefersReducedMotion",
];

/// The four safety guards still on their legacy `~/.claude/hooks/<name>.sh`
/// script, ported unchanged from `shell/gen-shared-settings.py`'s
/// `SAFETY_RE`; see that file's header comment for why they are exempt from
/// the bare `playbook hook <name>` form until WU-13 ports their Rust bodies.
const LEGACY_GUARD_COMMANDS: [&str; 4] = [
    "~/.claude/hooks/rm-workspace-guard.sh",
    "~/.claude/hooks/bg-await-guard.sh",
    "~/.claude/hooks/no-dash-guard.sh",
    "~/.claude/hooks/precommit-check.sh",
];

/// Everything that can stop generation before it produces output, one
/// variant per `die()` call site in `shell/gen-shared-settings.py`. Every
/// python `die()` call there exits with code 2; `generate` below returns
/// `Err` instead and leaves picking an exit code to the caller (`playbook
/// settings gen`, wired in `src/main.rs`), the same split
/// `src/init/merge.rs` uses for its own `MergeError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenError(pub String);

impl std::fmt::Display for GenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Read `path` as JSON, failing with the same two-way split
/// `shell/gen-shared-settings.py`'s `load_json` uses: unreadable file versus
/// readable-but-invalid JSON. Unlike that helper, the result is not yet
/// required to be a JSON object; SRC and PERMS validate that separately,
/// matching where python's own type checks happen.
fn load_json(path: &Path, label: &str) -> Result<Value, GenError> {
    let text = fs::read_to_string(path)
        .map_err(|_| GenError(format!("{label} not readable: {}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|_| GenError(format!("{label} is not valid JSON: {}", path.display())))
}

/// Ports `SAFETY_RE.fullmatch` from `shell/gen-shared-settings.py` as
/// explicit string matching: either a bare `playbook hook <name>`
/// invocation, where `<name>` starts with an ASCII lowercase letter and
/// continues with ASCII lowercase letters, digits or hyphens, or one of the
/// four legacy guard scripts.
fn is_safe_hook_command(command: &str) -> bool {
    if let Some(name) = command.strip_prefix("playbook hook ") {
        return is_valid_hook_name(name);
    }
    LEGACY_GUARD_COMMANDS.contains(&command)
}

/// `[a-z][a-z0-9-]*`: at least one character, the first an ASCII lowercase
/// letter, the rest ASCII lowercase letters, digits or hyphens.
fn is_valid_hook_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Ports `filter_hooks` from `shell/gen-shared-settings.py`: within each
/// event, keep only the hook groups that have at least one entry whose
/// command passes `is_safe_hook_command`, and within a kept group, only its
/// safe entries; drop an event entirely once none of its groups have any
/// safe entries left.
///
/// An event's groups not being a JSON array, or a group not being a JSON
/// object, has no real-world `settings.json` instance and is not exercised
/// by any ported scenario; python's `filter_hooks` would raise on either
/// shape and crash the whole process (nonzero exit, nothing on stdout, since
/// `print` never runs). This port fails safe instead: a malformed shape
/// contributes no hooks rather than crashing, so a shape this filter cannot
/// make sense of can never leak an unfiltered command into the seed.
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

/// The transform itself, ported from `shell/gen-shared-settings.py`'s
/// `main`: replace `.permissions`, force `skipAutoPermissionPrompt: false`,
/// drop the personal keys, and filter `.hooks` if present. `serde_json`'s
/// `Map::insert` updates a key already present in place, leaving its
/// position unchanged, and appends a genuinely new key at the end; the same
/// behaviour python's `dict[key] = value` has, so key order in `src` decides
/// key order in the result exactly as it does in the python original.
///
/// `shift_remove`, not `remove`, drops the personal keys: with
/// `preserve_order` enabled, plain `Map::remove` is `swap_remove` underneath
/// (moves the last key into the removed slot, an O(1) removal that perturbs
/// every other key's position), while python's `dict.pop(key, None)` leaves
/// every other key exactly where it was. `shift_remove` matches that.
fn build(mut src: Map<String, Value>, perms: Value) -> Map<String, Value> {
    src.insert("permissions".to_string(), perms);
    src.insert("skipAutoPermissionPrompt".to_string(), Value::Bool(false));
    for key in PERSONAL_KEYS {
        src.shift_remove(key);
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

/// Load `src_path` and `perms_path`, validate PERMS the way
/// `shell/gen-shared-settings.py`'s guard does (a JSON object with a
/// non-empty `allow` array), then run `build` and serialise the result as
/// `json.dumps(result, indent=2)` plus the trailing newline `print` adds.
///
/// `serde_json::to_string_pretty` reproduces python's `json.dumps(obj,
/// indent=2)` byte for byte on every ASCII fixture, including empty objects
/// and empty arrays; see `tests/settings_gen.rs`'s differential tests, which
/// run the real python generator as the oracle rather than hand-typing an
/// expected JSON blob.
pub fn generate(src_path: &Path, perms_path: &Path) -> Result<String, GenError> {
    let src_value = load_json(src_path, "source settings")?;
    let perms_value = load_json(perms_path, "permissions file")?;

    let perms_valid = perms_value
        .as_object()
        .and_then(|p| p.get("allow"))
        .and_then(Value::as_array)
        .is_some_and(|allow| !allow.is_empty());
    if !perms_valid {
        return Err(GenError(format!(
            "permissions file must be an object with a non-empty allow array: {}",
            perms_path.display()
        )));
    }

    // python's `dict(src)` raises an uncaught TypeError here if `src` is not
    // itself a JSON object, crashing with the same externally visible
    // outcome as every other guard: nonzero exit, nothing on stdout. This
    // port names that outcome explicitly instead of relying on an incidental
    // exception; no ported scenario exercises this shape.
    let src = match src_value {
        Value::Object(map) => map,
        _ => {
            return Err(GenError(format!(
                "source settings is not a JSON object: {}",
                src_path.display()
            )))
        }
    };

    let result = build(src, perms_value);
    let json = serde_json::to_string_pretty(&Value::Object(result))
        .expect("a JSON value parsed from valid JSON always re-serializes");
    Ok(format!("{json}\n"))
}
