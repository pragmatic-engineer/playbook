// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Three-way merge for Claude Code's settings.json, ported from
//! shell/merge-settings.py. That file's header comment is the merge
//! specification: the per-key policy, the NEWBASE_OUT partial base refresh
//! rule (a contested key freezes the OLD base value, never the template's),
//! and validation rules N2 (TEMPLATE and USER must be JSON objects), N3
//! (SKIP_OUT is optional; omitting it discards the skip info rather than
//! erroring), and N4 (a missing or invalid BASE degrades to `{}` with a
//! warning rather than failing the merge).
//!
//! Divergence from python: `main()` there calls `die()`, which prints to
//! stderr and calls `sys.exit(1)` directly, so both a validation failure and
//! an I/O failure during the write terminate the process from inside a
//! function with no way for a caller to intervene. `merge` below returns
//! `Result` instead, and returns any N4 warning as data rather than printing
//! it, leaving turning either into a process exit or a stderr line to the
//! caller (`playbook init`, wired in WU-8). The validation OUTCOME this
//! preserves exactly: TEMPLATE or USER that is not a JSON object fails the
//! whole merge before anything is computed or written (N2); a write failure
//! never lets a partially written file replace the original, since every
//! write goes through `atomic_write` below.
//!
//! `json.dumps(..., indent=2)` on the python side also defaults to
//! `ensure_ascii=True`, escaping every non-ASCII character to `\uXXXX`.
//! `serde_json::to_string_pretty` writes UTF-8 directly instead. Settings
//! values here are hook commands and filesystem paths, which are ASCII in
//! every real-world case, so this divergence is left unaddressed rather than
//! adding an ASCII-escaping serializer for a shape no fixture needs; flagged
//! here rather than left silent.

use serde::Serialize;
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

/// python's `dict.get(k)` returns `None` both when `k` is absent and when
/// `base[k]` is itself JSON `null`, since `json.load` maps `null` to `None`
/// with no way to tell the two apart from the return value alone. Every
/// comparison against BASE in `shell/merge-settings.py` goes through that
/// same `.get(k)`, so a key missing from BASE and a key explicitly `null` in
/// BASE behave identically there. `JSON_NULL` lets `base_lookup` mirror that
/// conflation exactly: a key absent from `base` compares as `Value::Null`,
/// not as "no value".
const JSON_NULL: Value = Value::Null;

fn base_lookup<'a>(base: &'a Map<String, Value>, key: &str) -> &'a Value {
    base.get(key).unwrap_or(&JSON_NULL)
}

/// N2: TEMPLATE or USER failed to load as a JSON object. Carries the same
/// wording shell/merge-settings.py's `die()` would have printed to stderr,
/// so a caller that surfaces this to a user sees the same words the shell
/// tooling always has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError(pub String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Everything that can stop a merge before it produces output. `Validation`
/// is N2: a clean, expected failure with a human-readable message. `Io` is
/// everything else, chiefly a failed atomic write, which must abort the
/// merge rather than let a caller believe a write happened when it did not.
#[derive(Debug)]
pub enum MergeError {
    Validation(ValidationError),
    Io(std::io::Error),
}

impl From<ValidationError> for MergeError {
    fn from(err: ValidationError) -> Self {
        MergeError::Validation(err)
    }
}

impl From<std::io::Error> for MergeError {
    fn from(err: std::io::Error) -> Self {
        MergeError::Io(err)
    }
}

/// One entry in the skip report: a top-level key where the template shipped
/// an update but the user had already customised it away from base, so the
/// update was withheld. Field order matches the `{key, template_had, yours}`
/// object shell/merge-settings.py:108 appends to `skipped`, since `Serialize`
/// on a struct writes fields in declaration order, the same way
/// `src/common/emit.rs` relies on struct field order to pin JSON key order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkippedEntry {
    pub key: String,
    pub template_had: Value,
    pub yours: Value,
}

/// N2: load `path` as a JSON object, failing the merge if it is missing,
/// unparsable, or not a JSON object. Used for TEMPLATE and USER, the two
/// inputs a caller controls and must supply validly; unlike BASE (see
/// `load_base`), there is no safe fallback for either.
fn load_required(path: &Path, label: &str) -> Result<Map<String, Value>, ValidationError> {
    if !path.is_file() {
        return Err(ValidationError(format!(
            "{label} not found: {}",
            path.display()
        )));
    }
    let parsed = fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    match parsed {
        Some(Value::Object(map)) => Ok(map),
        _ => Err(ValidationError(format!(
            "{label} is not a JSON object: {}",
            path.display()
        ))),
    }
}

/// N4: load BASE as a JSON object, falling back to `{}` when the file is
/// missing or invalid, since a corrupt or absent BASE must never block a
/// merge; BASE only informs which keys count as "unchanged", it holds no
/// user-authored data of its own. Returns the fallback warning as a string
/// instead of printing it, so the caller decides where warnings go, the same
/// way `merge` returns `Result` instead of exiting the process itself.
fn load_base(path: &Path) -> (Map<String, Value>, Option<String>) {
    if !path.is_file() {
        return (
            Map::new(),
            Some(format!(
                "warning: BASE not found; treating as {{}}: {}",
                path.display()
            )),
        );
    }
    let parsed = fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    match parsed {
        Some(Value::Object(map)) => (map, None),
        _ => (
            Map::new(),
            Some(format!(
                "warning: BASE is not a valid JSON object; treating as {{}}: {}",
                path.display()
            )),
        ),
    }
}

/// The merge policy this implements is specified in full in
/// shell/merge-settings.py's header comment; this is a line-for-line port of
/// its `three_way_merge`, over the sorted union of TEMPLATE's and USER's
/// top-level keys (`sorted(set(...))` there, matching jq's `unique()`).
fn three_way_merge(
    base: &Map<String, Value>,
    template: &Map<String, Value>,
    user: &Map<String, Value>,
) -> (Map<String, Value>, Map<String, Value>, Vec<SkippedEntry>) {
    let mut keys: Vec<&str> = template
        .keys()
        .map(String::as_str)
        .chain(user.keys().map(String::as_str))
        .collect();
    keys.sort_unstable();
    keys.dedup();

    let mut merged = Map::new();
    for key in &keys {
        match user.get(*key) {
            None => {
                if let Some(t) = template.get(*key) {
                    merged.insert((*key).to_string(), t.clone());
                }
            }
            Some(uv) if uv == base_lookup(base, key) => {
                if let Some(t) = template.get(*key) {
                    merged.insert((*key).to_string(), t.clone());
                }
            }
            Some(uv) => {
                merged.insert((*key).to_string(), uv.clone());
            }
        }
    }

    let mut newbase = Map::new();
    for key in &keys {
        match user.get(*key) {
            None => {
                if let Some(t) = template.get(*key) {
                    newbase.insert((*key).to_string(), t.clone());
                }
            }
            Some(uv) if uv != base_lookup(base, key) => {
                if let Some(b) = base.get(*key) {
                    newbase.insert((*key).to_string(), b.clone());
                }
            }
            Some(_) => {
                if let Some(t) = template.get(*key) {
                    newbase.insert((*key).to_string(), t.clone());
                }
            }
        }
    }

    let mut skipped = Vec::new();
    for key in &keys {
        if let Some(uv) = user.get(*key) {
            if uv != base_lookup(base, key) {
                if let Some(t) = template.get(*key) {
                    if t != uv {
                        skipped.push(SkippedEntry {
                            key: (*key).to_string(),
                            template_had: t.clone(),
                            yours: uv.clone(),
                        });
                    }
                }
            }
        }
    }

    (merged, newbase, skipped)
}

/// Write `content` to `path` atomically: write to a sibling temp file in the
/// same directory, then rename it into place, mirroring
/// `tempfile.mkstemp` plus `os.replace` in shell/merge-settings.py's own
/// `atomic_write`. Renaming onto an existing path is atomic on the same
/// filesystem, so a reader of `path`, or a crash between the temp write and
/// the rename, never observes a partially written file; on failure the temp
/// file is removed and `path` is left exactly as it was.
///
/// Not built on `common::atomic::atomic_append`: that helper serialises
/// concurrent writers behind an mkdir lock and only ever APPENDS a line to
/// an existing file, the wrong shape for replacing a whole file's contents.
/// `src/hooks/rebuild_memory_graph.rs`'s `write_graph_atomically` already
/// establishes this same temp-file-plus-rename pattern for exactly this
/// shape of write; unlike that one, this returns the failure instead of
/// swallowing it, since a caller here must know a settings write did not
/// happen rather than silently proceeding as if it had.
fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp_path = dir.join(format!(
        ".merge-settings-{}-{:?}.tmp",
        std::process::id(),
        std::thread::current().id()
    ));
    if let Err(err) = fs::write(&tmp_path, content) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    Ok(())
}

/// What a caller (eventually `playbook init`, wired in WU-8) does with a
/// successful merge: print `stdout` where `shell/merge-settings.py` would
/// have printed it (its own trailing newline not included here, matching
/// how `src/common/emit.rs`'s emitters hand back an unterminated string for
/// `println!` to terminate); warn with `base_warning` if it is `Some`; act
/// on `skipped` to surface withheld customisations without re-reading
/// SKIP_OUT off disk. NEWBASE_OUT, and SKIP_OUT when the caller asked for
/// one, are already written to disk by the time this returns.
#[derive(Debug)]
pub struct MergeOutcome {
    pub stdout: String,
    pub skipped: Vec<SkippedEntry>,
    pub base_warning: Option<String>,
}

/// Three-way merge BASE, TEMPLATE and USER into MERGED, refresh NEWBASE_OUT,
/// and optionally write SKIP_OUT. Ports shell/merge-settings.py's `main`;
/// see this module's header comment for the merge policy and validation
/// rules, and for why this returns `Result` where python calls
/// `sys.exit(1)` or lets an exception crash the process.
pub fn merge(
    base_path: &Path,
    template_path: &Path,
    user_path: &Path,
    newbase_out: &Path,
    skip_out: Option<&Path>,
) -> Result<MergeOutcome, MergeError> {
    // N2: TEMPLATE and USER are validated before BASE is even read, matching
    // shell/merge-settings.py's main() order; nothing is written on failure.
    let template = load_required(template_path, "TEMPLATE")?;
    let user = load_required(user_path, "USER")?;
    // N4: BASE degrades to {} with a warning rather than failing the merge.
    let (base, base_warning) = load_base(base_path);

    let (merged, newbase, skipped) = three_way_merge(&base, &template, &user);

    let newbase_json = serde_json::to_string_pretty(&Value::Object(newbase))
        .expect("a JSON value parsed from valid JSON always re-serializes");
    atomic_write(newbase_out, &format!("{newbase_json}\n"))?;

    if let Some(skip_path) = skip_out {
        let skip_json = serde_json::to_string_pretty(&skipped)
            .expect("a JSON value parsed from valid JSON always re-serializes");
        atomic_write(skip_path, &format!("{skip_json}\n"))?;
    }

    let stdout = serde_json::to_string_pretty(&Value::Object(merged))
        .expect("a JSON value parsed from valid JSON always re-serializes");

    Ok(MergeOutcome {
        stdout,
        skipped,
        base_warning,
    })
}
