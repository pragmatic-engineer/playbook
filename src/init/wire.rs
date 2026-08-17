// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Wires the 11 hooks Claude Code can invoke that are already ported to
//! Rust straight into `settings.json` as a bare `playbook hook <name>`
//! command, retiring `hooks/hooks.json` as the source of truth for them.
//! Before this module lands, `hooks/hooks.json` points all 15 hooks at
//! python scripts under `${CLAUDE_PLUGIN_ROOT}/hooks/`, so the Rust binary
//! this crate builds dispatches nothing in production; `wire` is the pivot
//! that makes the 11 ported hooks live.
//!
//! The remaining four hooks (`rm-workspace-guard`, `bg-await-guard`,
//! `no-dash-guard`, `precommit-check`) are deliberately NOT rewired here.
//! Their Rust modules (`src/hooks/*_guard.rs`) are still empty stubs
//! (`pub fn run(_payload: &Payload) {}`) until WU-13 ports them, two
//! Segments later; pointing `settings.json` at `playbook hook <name>` for
//! one of them before then would silently disable a live safety guard,
//! since the stub runs, emits no permission decision, and exits 0. This is
//! the exact defect the 2026-08-16 ADR amendment to WU-8 records, and this
//! module now avoids it: `wire` keeps writing their existing
//! `~/.claude/hooks/<name>.sh` command instead. WU-13 is the only unit that
//! should flip `GUARD_SPECS`'s `ported` field to `true`; do not "tidy" this
//! ahead of that unit landing.
//!
//! `wire` is an upsert into whatever `settings.json` already has, never a
//! wholesale replace: a hand-added hook entry a user placed alongside one of
//! ours, or an entire event/matcher group we do not manage, survives
//! untouched. This is the same clobber risk `src/init/merge.rs`'s mandatory
//! fixture 6 pins at the settings-merge layer; `wire` pins it again at the
//! narrower hook-wiring layer, since it edits `.hooks` directly rather than
//! going through a three-way merge.
//!
//! The bare-name form (`playbook hook session-init`, not an absolute path)
//! is deliberate: Claude Code already accepts a bare command resolved on
//! PATH for a hand-written hook entry (`~/.claude/settings.json.bak` line
//! 167: `"command": "rtk hook claude"`), so `playbook`, installed the same
//! way, resolves the same way. `tests/init_wire.rs` asserts this rather than
//! trusting it.

use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// One hook registration `wire` ensures exists in `settings.json`: which
/// event it fires on, the matcher grouping it belongs to (`None` for events
/// hooks.json never gave a matcher, such as `SessionStart`), the hook name
/// to invoke, any `if` guard or `timeout` carried over unchanged from the
/// entry it replaces, and whether its Rust port is real (`ported`), which
/// decides whether `wire` may target the compiled binary for it at all.
struct HookSpec {
    event: &'static str,
    matcher: Option<&'static str>,
    name: &'static str,
    if_cond: Option<&'static str>,
    timeout: Option<u64>,
    ported: bool,
}

/// The 11 functional hooks `hooks/hooks.json` used to register, already
/// ported to Rust, rewired here to the bare `playbook hook <name>` form
/// (one event/matcher shape each, ported line for line from that file, 12
/// entries since `session-clean-exit` fires on both `Stop` and
/// `SessionEnd`). Together with `GUARD_SPECS` below these are exactly the
/// 15 `HookName` variants `src/lib.rs` declares.
///
/// Divergence from `hooks/hooks.json`'s literal shape: that file ships
/// `Stop`'s two hooks (`session-clean-exit`, `memory-capture`) as two
/// separate single-hook groups rather than one two-hook group. `wire` groups
/// purely by (event, matcher), so it merges them into one `Stop` group here.
/// This is a deliberate shape simplification, not a behaviour change: a
/// matcher only selects which tool invocations a hook fires on, and `Stop`
/// entries have no matcher to select by, so two groups versus one dispatches
/// identically.
const PORTED_HOOK_SPECS: &[HookSpec] = &[
    HookSpec {
        event: "SessionStart",
        matcher: None,
        name: "session-init",
        if_cond: None,
        timeout: None,
        ported: true,
    },
    HookSpec {
        event: "PreToolUse",
        matcher: Some("Read"),
        name: "preread-edit-check",
        if_cond: None,
        timeout: None,
        ported: true,
    },
    HookSpec {
        event: "PreToolUse",
        matcher: Some("Read"),
        name: "preread-size-check",
        if_cond: None,
        timeout: None,
        ported: true,
    },
    HookSpec {
        event: "PreToolUse",
        matcher: Some("Read|Grep|Glob|Edit|Write|NotebookEdit"),
        name: "search-counter",
        if_cond: None,
        timeout: None,
        ported: true,
    },
    HookSpec {
        event: "PreToolUse",
        matcher: Some("Edit|Write"),
        name: "memory-anchors",
        if_cond: None,
        timeout: None,
        ported: true,
    },
    HookSpec {
        event: "PostToolUse",
        matcher: Some("Edit|Write|NotebookEdit"),
        name: "post-edit-track",
        if_cond: None,
        timeout: None,
        ported: true,
    },
    HookSpec {
        event: "PostToolUse",
        matcher: Some("Edit|Write|NotebookEdit"),
        name: "rebuild-memory-graph",
        if_cond: None,
        timeout: None,
        ported: true,
    },
    HookSpec {
        event: "UserPromptSubmit",
        matcher: None,
        name: "auto-model-detect",
        if_cond: None,
        timeout: None,
        ported: true,
    },
    HookSpec {
        event: "PreCompact",
        matcher: None,
        name: "precompact-warn",
        if_cond: None,
        timeout: None,
        ported: true,
    },
    HookSpec {
        event: "Stop",
        matcher: None,
        name: "session-clean-exit",
        if_cond: None,
        timeout: None,
        ported: true,
    },
    HookSpec {
        event: "Stop",
        matcher: None,
        name: "memory-capture",
        if_cond: None,
        timeout: None,
        ported: true,
    },
    HookSpec {
        event: "SessionEnd",
        matcher: None,
        name: "session-clean-exit",
        if_cond: None,
        timeout: None,
        ported: true,
    },
];

/// The 4 always-on safety guards `settings.json` already wires directly
/// today, deliberately kept off the binary form `PORTED_HOOK_SPECS` gets.
/// Their Rust modules (`src/hooks/rm_workspace_guard.rs` and the three
/// siblings beside it) are still the empty stub
/// `pub fn run(_payload: &Payload) {}`; WU-13, two Segments later, is the
/// unit that fills them in, and is the only unit that should flip `ported`
/// to `true` here. Until then `wire` keeps these four pointed at their
/// working `~/.claude/hooks/<name>.sh` script, self-healing the entry back
/// to that form even if a prior run (or a hand edit) left it on the binary
/// form instead. See the 2026-08-16 amendment to this Work Unit in
/// `docs/adr/0007-rust-binary-for-hooks-and-launcher-blueprint.md`: wiring
/// a guard to a stub silently disables it.
const GUARD_SPECS: &[HookSpec] = &[
    HookSpec {
        event: "PreToolUse",
        matcher: Some("Bash"),
        name: "rm-workspace-guard",
        if_cond: Some("Bash(rm:*)"),
        timeout: Some(10),
        ported: false,
    },
    HookSpec {
        event: "PreToolUse",
        matcher: Some("Bash"),
        name: "bg-await-guard",
        if_cond: None,
        timeout: Some(10),
        ported: false,
    },
    HookSpec {
        event: "PreToolUse",
        matcher: Some("Bash"),
        name: "no-dash-guard",
        if_cond: None,
        timeout: Some(10),
        ported: false,
    },
    HookSpec {
        event: "PreToolUse",
        matcher: Some("Bash"),
        name: "precommit-check",
        if_cond: Some("Bash(git commit:*)"),
        timeout: Some(10),
        ported: false,
    },
];

/// `settings.json` (or a value nested under it) was not shaped the way
/// `wire` needs to safely upsert into it. Carries a human-readable message
/// rather than a structured payload, matching `merge::ValidationError`'s
/// shape for the same reason: a caller surfacing this to a person needs
/// words, not a variant to match on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError(pub String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Everything that can stop `wire` before it finishes. `Validation` is a
/// `settings.json` (or nested `.hooks` value) that is not an object where
/// one is required; failing closed here beats guessing a shape and silently
/// discarding whatever was actually there. `Io` covers a failed read, a
/// failed backup copy, or a failed atomic write.
#[derive(Debug)]
pub enum WireError {
    Validation(ValidationError),
    Io(std::io::Error),
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::Validation(err) => write!(f, "{err}"),
            WireError::Io(err) => write!(f, "{err}"),
        }
    }
}

impl From<ValidationError> for WireError {
    fn from(err: ValidationError) -> Self {
        WireError::Validation(err)
    }
}

impl From<std::io::Error> for WireError {
    fn from(err: std::io::Error) -> Self {
        WireError::Io(err)
    }
}

/// What `wire` did. `changed` is `false` when every registration already
/// matched the target shape byte for byte, in which case nothing was
/// written and no backup was taken; `backup_path` is `Some` only when a
/// pre-existing `settings.json` was actually overwritten.
#[derive(Debug)]
pub struct WireOutcome {
    pub changed: bool,
    pub backup_path: Option<PathBuf>,
}

/// Writes every hook in `PORTED_HOOK_SPECS` to its bare `playbook hook
/// <name>` command and every hook in `GUARD_SPECS` to its legacy
/// `~/.claude/hooks/<name>.sh` command, creating `.hooks` and any event or
/// matcher group it needs from scratch, and leaving every other key in the
/// file, and every hook entry not managed here, untouched. Idempotent:
/// calling this twice in a row writes nothing the second time, since the
/// second call's freshly rendered content is compared byte for byte against
/// what is already on disk before anything is written.
///
/// Backs `settings_path` up first, timestamped, whenever a change is about
/// to land; a no-op call takes no backup. This guards the same failure the
/// `hook-rename-lockstep-settings` incident recorded: a settings/hook name
/// mismatch went unnoticed for 28 hours and produced roughly 110 silent
/// errors, because there was no pre-change snapshot to recover from or diff
/// against.
pub fn wire(settings_path: &Path) -> Result<WireOutcome, WireError> {
    let (mut root, original) = load_settings(settings_path)?;

    {
        let hooks_value = root
            .entry("hooks")
            .or_insert_with(|| Value::Object(Map::new()));
        let hooks = hooks_value.as_object_mut().ok_or_else(|| {
            ValidationError(format!(
                "'hooks' in {} is not a JSON object",
                settings_path.display()
            ))
        })?;
        for spec in PORTED_HOOK_SPECS.iter().chain(GUARD_SPECS.iter()) {
            upsert_hook(hooks, spec, settings_path)?;
        }
    }

    let rendered = serde_json::to_string_pretty(&Value::Object(root))
        .expect("a JSON value built from valid JSON always re-serializes");
    let rendered = format!("{rendered}\n");

    if rendered == original {
        return Ok(WireOutcome {
            changed: false,
            backup_path: None,
        });
    }

    let backup_path = if settings_path.is_file() {
        let backup = timestamped_backup_path(settings_path);
        fs::copy(settings_path, &backup)?;
        Some(backup)
    } else {
        None
    };

    atomic_write(settings_path, &rendered)?;

    Ok(WireOutcome {
        changed: true,
        backup_path,
    })
}

/// Reads `path` as a JSON object, returning it alongside the exact bytes
/// read so `wire` can compare its own re-serialization against them to
/// decide whether anything actually changed. A missing file starts from an
/// empty object and an empty baseline, matching a fresh `playbook init` run
/// before any `settings.json` exists yet.
fn load_settings(path: &Path) -> Result<(Map<String, Value>, String), WireError> {
    if !path.is_file() {
        return Ok((Map::new(), String::new()));
    }
    let original = fs::read_to_string(path)?;
    match serde_json::from_str::<Value>(&original) {
        Ok(Value::Object(map)) => Ok((map, original)),
        Ok(_) => Err(ValidationError(format!("{} is not a JSON object", path.display())).into()),
        Err(err) => {
            Err(ValidationError(format!("{} is not valid JSON: {err}", path.display())).into())
        }
    }
}

/// Ensures `hooks[spec.event]` contains one group whose matcher equals
/// `spec.matcher`, and that group's `hooks` array contains exactly one entry
/// for `spec.name`, rewritten to its canonical form (see `target_command`).
/// Every other entry already in that group, and every other group on the
/// same event, is left exactly as found: this upserts into the existing
/// structure, it never replaces it wholesale.
fn upsert_hook(
    hooks: &mut Map<String, Value>,
    spec: &HookSpec,
    settings_path: &Path,
) -> Result<(), WireError> {
    let groups = hooks
        .entry(spec.event)
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| {
            ValidationError(format!(
                "'hooks.{}' in {} is not a JSON array",
                spec.event,
                settings_path.display()
            ))
        })?;

    let existing_group_idx = groups
        .iter()
        .position(|group| group_matcher(group) == spec.matcher);

    let group_map = match existing_group_idx {
        Some(idx) => groups[idx].as_object_mut().ok_or_else(|| {
            ValidationError(format!(
                "a 'hooks.{}' entry in {} is not a JSON object",
                spec.event,
                settings_path.display()
            ))
        })?,
        None => {
            let mut new_group = Map::new();
            if let Some(matcher) = spec.matcher {
                new_group.insert("matcher".to_string(), Value::String(matcher.to_string()));
            }
            new_group.insert("hooks".to_string(), Value::Array(Vec::new()));
            groups.push(Value::Object(new_group));
            groups
                .last_mut()
                .expect("just pushed")
                .as_object_mut()
                .expect("just built as an object")
        }
    };

    let hooks_array = group_map
        .entry("hooks")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| {
            ValidationError(format!(
                "a 'hooks.{}' group's 'hooks' field in {} is not a JSON array",
                spec.event,
                settings_path.display()
            ))
        })?;

    let entry = canonical_entry(spec);
    match hooks_array.iter().position(|h| entry_targets(h, spec.name)) {
        Some(idx) => hooks_array[idx] = entry,
        None => hooks_array.push(entry),
    }
    Ok(())
}

/// The `matcher` field of a `.hooks.<event>` array entry, or `None` when the
/// entry has no `matcher` key at all (hooks.json's shape for events that
/// never take one, such as `SessionStart`) or is not an object.
fn group_matcher(group: &Value) -> Option<&str> {
    group.as_object()?.get("matcher")?.as_str()
}

/// Whether `entry`'s `command` already targets hook `name`, in either its
/// legacy form (a python script under `hooks/`, or a shell guard under
/// `~/.claude/hooks/`) or the bare `playbook hook <name>` form `wire`
/// writes. Used both to find the entry to rewrite and, on a second `wire`
/// call, to recognise that it is already in its target form.
fn entry_targets(entry: &Value, name: &str) -> bool {
    match entry
        .as_object()
        .and_then(|e| e.get("command"))
        .and_then(Value::as_str)
    {
        Some(cmd) => is_legacy_command(cmd, name) || cmd == bare_command(name),
        None => false,
    }
}

/// Whether `cmd` is the pre-wiring command for hook `name`: a python script
/// `hooks/hooks.json` pointed at (`.../hooks/<name>.py`, generally wrapped
/// in literal quote characters for the shell) or a shell guard `settings.json`
/// pointed at directly (`~/.claude/hooks/<name>.sh`). The hook's kebab-case
/// `HookName` and its script's file stem are identical by construction, so
/// comparing the trailing path segment against `<name>.py` / `<name>.sh` is
/// sufficient without needing to know which of the two legacy forms it is.
fn is_legacy_command(cmd: &str, name: &str) -> bool {
    let trimmed = cmd.trim_matches('"');
    let file_name = trimmed.rsplit('/').next().unwrap_or(trimmed);
    file_name == format!("{name}.py") || file_name == format!("{name}.sh")
}

/// The bare command `wire` writes for an already-ported hook named `name`:
/// no path, resolved on `PATH` the same way a hand-written `rtk hook claude`
/// entry already is. Callers building a `HookSpec`'s entry should go through
/// `target_command`, which also handles the still-unported guards; this is
/// exposed separately only because `entry_targets` needs to recognise the
/// bare form on its own.
fn bare_command(name: &str) -> String {
    format!("playbook hook {name}")
}

/// The pre-Rust shell command `wire` keeps an unported hook pointed at:
/// `~/.claude/hooks/<name>.sh`, the same path `settings.json` already wires
/// directly today for each of the four guards in `GUARD_SPECS`.
fn legacy_shell_command(name: &str) -> String {
    format!("~/.claude/hooks/{name}.sh")
}

/// The command `wire` writes for `spec`: the bare `playbook hook <name>`
/// binary form when its Rust port is real (`spec.ported`), or the legacy
/// `~/.claude/hooks/<name>.sh` script when it is not, so an unported hook
/// (today, the four guards in `GUARD_SPECS`) never gets pointed at a stub.
fn target_command(spec: &HookSpec) -> String {
    if spec.ported {
        bare_command(spec.name)
    } else {
        legacy_shell_command(spec.name)
    }
}

/// The exact JSON object `wire` writes for one `HookSpec`: `type` and
/// `command` always, `if` and `timeout` only when the spec carries them, in
/// that fixed key order every time, so re-running `wire` against its own
/// prior output reproduces the identical object rather than merely an
/// equivalent one. `command` comes from `target_command`, so it is only the
/// bare binary form when the spec is actually ported.
fn canonical_entry(spec: &HookSpec) -> Value {
    let mut entry = Map::new();
    entry.insert("type".to_string(), Value::String("command".to_string()));
    entry.insert("command".to_string(), Value::String(target_command(spec)));
    if let Some(if_cond) = spec.if_cond {
        entry.insert("if".to_string(), Value::String(if_cond.to_string()));
    }
    if let Some(timeout) = spec.timeout {
        entry.insert("timeout".to_string(), Value::Number(timeout.into()));
    }
    Value::Object(entry)
}

/// `settings.json.bak.<unix-seconds>` beside the original, so a bad wiring
/// run is always recoverable, multiple runs never collide on the same
/// backup name, and the timestamp itself shows how stale a given backup is.
fn timestamped_backup_path(settings_path: &Path) -> PathBuf {
    let epoch_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let file_name = settings_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "settings.json".to_string());
    settings_path.with_file_name(format!("{file_name}.bak.{epoch_secs}"))
}

/// Writes `content` to `path` via a sibling temp file plus rename, so a
/// reader of `path` never observes a half-written `settings.json`. Mirrors
/// `merge::atomic_write`'s approach exactly, but is not shared with it:
/// `src/init/merge.rs` is outside this Work Unit's file plan (WU-7 owns it,
/// concurrent with this one), so promoting its private helper to `pub(crate)`
/// would be an edit outside scope. Duplicating a five-line function is
/// cheaper than that.
fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp_path = dir.join(format!(
        ".wire-settings-{}-{:?}.tmp",
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
