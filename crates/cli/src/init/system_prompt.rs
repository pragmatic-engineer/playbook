// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Places `prompts/SYSTEM_PROMPT.md` under `$HOME/.config/playbook/prompts/`,
//! porting the retired `setup-local.sh`, the only prior installer of it.
//!
//! WU-14 deletes that script. Without this module the `--system-prompt` flag
//! and `/playbook:doctor`'s Layer 4 would both outlive the code that makes
//! them mean anything: a documented flag installing nothing, and a doctor
//! layer checking for a file no component places. That is the same shape as
//! the guard gap `init::guards` closes, and the same rule applies, the
//! component that names a path is the component that puts the file there.
//!
//! **The opt-in semantics are preserved deliberately.** `setup-local.sh`
//! places this file only under `--system-prompt`, and `commands/doctor.md`
//! labels Layer 4 "(opt-in)". Making `init` install it unconditionally would
//! quietly hand a system prompt to every existing user who had chosen not to
//! have one, which is a behaviour change well outside a port. So:
//!
//! - `opt_in` true (the user passed `playbook setup --system-prompt`): install
//!   or refresh.
//! - `opt_in` false but the file is already there: refresh it. A user who
//!   opted in once should not silently drift onto a stale copy just because
//!   later runs omitted the flag.
//! - `opt_in` false and no file: do nothing, and say so.
//!
//! A missing source is reported, not an error: the retired `setup-local.sh` warned and
//! continued, because an absent optional prompt must not fail an install that
//! is otherwise fine.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The per-user key that decides whether `pb cc` loads the prompt (ADR 0044).
pub const KEY: &str = "systemPrompt.enabled";

/// The setup answer for the system prompt part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Place or refresh the file and store `KEY` as true.
    Yes,
    /// Store `KEY` as false and leave an existing file in place.
    No,
    /// Nothing was asked: refresh an existing copy and store nothing.
    Unasked,
}

/// Where setup places the prompt and `pb cc` reads it.
pub fn path(home: &Path) -> PathBuf {
    crate::common::paths::playbook_root_from(home)
        .join("prompts")
        .join("SYSTEM_PROMPT.md")
}

/// Whether `pb cc` loads the prompt: `KEY` is not stored as false.
pub fn enabled(home: &Path) -> bool {
    !matches!(
        crate::config::resolve(KEY, home, None),
        Ok((serde_json::Value::Bool(false), _))
    )
}

/// Stores the answer in `KEY`, in the global tier.
pub fn record(home: &Path, on: bool) -> Result<(), String> {
    crate::config::write::set(
        crate::config::write::Tier::Global,
        KEY,
        serde_json::Value::Bool(on),
        home,
        None,
    )
    .map(|_| ())
    .map_err(|err| err.to_string())
}

/// Why a run that asks nothing leaves the prompt off, or `None` when it may
/// turn it on (ADR 0044 decision 5).
pub fn held_off(home: &Path, claude_home: &Path) -> Option<&'static str> {
    if !enabled(home) {
        Some("systemPrompt.enabled is false")
    } else if earlier_install_without_prompt(home, claude_home) {
        Some("this install had no system prompt before")
    } else {
        None
    }
}

/// `KEY` is not stored, the prompt file is missing, and settings.json shows
/// an earlier setup run: a playbook hook or the playbook status line.
pub fn earlier_install_without_prompt(home: &Path, claude_home: &Path) -> bool {
    let unset = matches!(
        crate::config::resolve(KEY, home, None),
        Ok((_, crate::config::Source::Default))
    );
    if !unset || path(home).exists() {
        return false;
    }
    let settings = fs::read_to_string(claude_home.join("settings.json")).unwrap_or_default();
    settings.contains(crate::init::wire::HOOK_COMMAND_PREFIX)
        || settings.contains(&format!("\"{}\"", crate::init::statusline::RUST_COMMAND))
}

/// How `place_system_prompt` landed. Every variant carries the path it acted
/// on (or would have) so the caller can render a line naming a real location
/// rather than a generic message.
#[derive(Debug)]
pub enum Placement {
    /// Written, either fresh or over a stale copy.
    Placed(PathBuf),
    /// Destination already byte-identical to the shipped copy.
    AlreadyCurrent(PathBuf),
    /// The shipped tree has no `prompts/SYSTEM_PROMPT.md` at this path.
    NotShipped(PathBuf),
    /// The user edited the installed copy, so it was left untouched.
    UserEdited(PathBuf),
    /// Not requested, and not already installed, so nothing to do.
    NotOptedIn,
}

/// Copy the shipped `prompts/SYSTEM_PROMPT.md` into
/// `$HOME/.config/playbook/prompts/`, under the opt-in rules above.
pub fn place_system_prompt(
    self_root: &Path,
    home: &Path,
    opt_in: bool,
) -> Result<Placement, io::Error> {
    let source = self_root.join("prompts").join("SYSTEM_PROMPT.md");
    let dest = path(home);

    if !opt_in && !dest.exists() {
        return Ok(Placement::NotOptedIn);
    }
    if !source.is_file() {
        return Ok(Placement::NotShipped(source));
    }
    if already_current(&source, &dest) {
        return Ok(Placement::AlreadyCurrent(dest));
    }

    if crate::init::migrate::user_edited(home, "system-prompt", &dest) {
        return Ok(Placement::UserEdited(dest));
    }

    copy_atomically(&source, &dest)?;
    Ok(Placement::Placed(dest))
}

/// Whether `dest` already holds exactly `src`'s bytes. Unlike
/// `init::guards`, no executable bit is checked: this is documentation read
/// by Claude Code, never executed, and the retired `setup-local.sh` compared with a
/// plain `cmp -s` for the same reason.
fn already_current(src: &Path, dest: &Path) -> bool {
    match (fs::read(src), fs::read(dest)) {
        (Ok(shipped), Ok(placed)) => shipped == placed,
        _ => false,
    }
}

/// Temp-file-plus-rename, matching `init::statusline` and `init::guards`, so
/// a concurrent reader never sees a half-written prompt.
fn copy_atomically(src: &Path, dest: &Path) -> io::Result<()> {
    let dir = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;

    let tmp_path = dir.join(format!(".SYSTEM_PROMPT-{}.tmp", std::process::id()));
    let result = fs::copy(src, &tmp_path).and_then(|_| fs::rename(&tmp_path, dest));
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result.map(|_| ())
}
