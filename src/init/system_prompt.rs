// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Places `prompts/SYSTEM_PROMPT.md` under `~/.claude/prompts/`, porting
//! `shell/setup-local.sh:278-295`, the only thing that has ever installed it.
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
//! - `opt_in` true (the user passed `playbook init --system-prompt`): install
//!   or refresh.
//! - `opt_in` false but the file is already there: refresh it. A user who
//!   opted in once should not silently drift onto a stale copy just because
//!   later runs omitted the flag.
//! - `opt_in` false and no file: do nothing, and say so.
//!
//! A missing source is reported, not an error: `setup-local.sh:293` warns and
//! continues, because an absent optional prompt must not fail an install that
//! is otherwise fine.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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
    /// Not requested, and not already installed, so nothing to do.
    NotOptedIn,
}

/// Copy the shipped `prompts/SYSTEM_PROMPT.md` into `claude_home/prompts/`
/// under the opt-in rules in this module's doc comment.
pub fn place_system_prompt(
    self_root: &Path,
    claude_home: &Path,
    opt_in: bool,
) -> Result<Placement, io::Error> {
    let source = self_root.join("prompts").join("SYSTEM_PROMPT.md");
    let dest = claude_home.join("prompts").join("SYSTEM_PROMPT.md");

    if !opt_in && !dest.exists() {
        return Ok(Placement::NotOptedIn);
    }
    if !source.is_file() {
        return Ok(Placement::NotShipped(source));
    }
    if already_current(&source, &dest) {
        return Ok(Placement::AlreadyCurrent(dest));
    }

    copy_atomically(&source, &dest)?;
    Ok(Placement::Placed(dest))
}

/// Whether `dest` already holds exactly `src`'s bytes. Unlike
/// `init::guards`, no executable bit is checked: this is documentation read
/// by Claude Code, never executed, and `setup-local.sh:287` compares with a
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
