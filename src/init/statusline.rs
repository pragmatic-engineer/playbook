// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Places `statusline.sh` at whatever path `settings.json`'s
//! `statusLine.command` names, then confirms the placement actually
//! resolves rather than assuming the write succeeded.
//!
//! This closes the gap behind the 2026-08-12 outage: the ADR 0006
//! relocation moved `statusline.sh` out of `~/.claude` while
//! `settings.shared.json` still pointed `statusLine.command` at the old
//! path. Two clean `/playbook:setup` runs and a four-layer `/playbook:doctor`
//! pass all reported healthy while the status line rendered nothing, because
//! nothing in the install path ever read the path back out of
//! `settings.json` and checked it. `place_statusline` reads the destination
//! FROM `settings.json` rather than assuming a fixed location, so the script
//! and the setting naming it cannot drift apart the way they did then.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Everything that can stop `place_statusline` before the script is placed
/// and confirmed readable. `Settings` covers every way `settings.json` can
/// fail to name a usable destination (missing file, invalid JSON, no
/// `statusLine.command` string, or a command with no resolvable path);
/// `Io` is a filesystem failure copying the script or reading back the
/// result.
#[derive(Debug)]
pub enum StatuslineError {
    Settings(String),
    Io(io::Error),
}

impl std::fmt::Display for StatuslineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatuslineError::Settings(msg) => write!(f, "{msg}"),
            StatuslineError::Io(err) => write!(f, "{err}"),
        }
    }
}

impl From<io::Error> for StatuslineError {
    fn from(err: io::Error) -> Self {
        StatuslineError::Io(err)
    }
}

/// Read `settings.json` at `settings_path` and resolve the filesystem path
/// its `statusLine.command` names, expanding a literal `$HOME` token to
/// `home`. Exposed separately from `place_statusline` so a caller (or a
/// test regression-pinning the 2026-08-12 outage) can ask "where would the
/// statusline end up" without triggering a write, the same read/act split
/// `init::merge` keeps between computing a result and acting on it.
pub fn resolve_statusline_path(
    settings_path: &Path,
    home: &Path,
) -> Result<PathBuf, StatuslineError> {
    let command = read_statusline_command(settings_path)?;
    resolve_command_path(&command, home).ok_or_else(|| {
        StatuslineError::Settings(format!(
            "could not resolve a file path from statusLine.command: {command:?}"
        ))
    })
}

/// Place `statusline.sh` (shipped at `self_root/statusline.sh`) at the path
/// `settings.json` (at `settings_path`) actually names, then read that
/// destination back and confirm it is a readable regular file rather than
/// trusting the copy above succeeded. `home` expands a literal `$HOME` token
/// in the command string. Returns the destination path on success.
pub fn place_statusline(
    self_root: &Path,
    settings_path: &Path,
    home: &Path,
) -> Result<PathBuf, StatuslineError> {
    let dest = resolve_statusline_path(settings_path, home)?;
    copy_statusline_atomically(&self_root.join("statusline.sh"), &dest)?;
    verify_placed(&dest)?;
    Ok(dest)
}

/// N2-shaped validation for `settings.json`: load it as a JSON object and
/// pull `statusLine.command` out as a string, failing with a human-readable
/// reason on anything short of a clean read. There is no safe fallback the
/// way `init::merge::load_base` has one for a missing BASE: an absent or
/// malformed `statusLine.command` means there is no destination to place the
/// script at, so the caller must be told rather than guessing one.
fn read_statusline_command(settings_path: &Path) -> Result<String, StatuslineError> {
    let text = fs::read_to_string(settings_path).map_err(|err| {
        StatuslineError::Settings(format!("cannot read {settings_path:?}: {err}"))
    })?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|err| {
        StatuslineError::Settings(format!("{settings_path:?} is not valid JSON: {err}"))
    })?;
    value
        .get("statusLine")
        .and_then(|status_line| status_line.get("command"))
        .and_then(|command| command.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            StatuslineError::Settings(format!(
                "{settings_path:?} has no statusLine.command string"
            ))
        })
}

/// Extract the script path a `statusLine.command` shell command invokes,
/// expanding a literal `$HOME` token to `home`. The command is a plain
/// interpreter invocation (`"bash $HOME/.claude/statusline.sh"`, confirmed
/// in both `settings.shared.json:135` and the live `~/.claude/settings.json`
/// this module regression-pins), never a quoted argument or a multi-token
/// argument list, so the last whitespace-separated token is the path. A
/// general shell command-line parser is not built for a shape nothing here
/// exercises.
fn resolve_command_path(command: &str, home: &Path) -> Option<PathBuf> {
    let last_token = command.split_whitespace().last()?;
    let expanded = last_token.replace("$HOME", &home.to_string_lossy());
    if expanded.is_empty() {
        None
    } else {
        Some(PathBuf::from(expanded))
    }
}

/// Copy `src` to `dest` by writing a sibling temp file and renaming it into
/// place: the same shape `init::merge`'s `atomic_write` uses for
/// `settings.json`, so a reader of `dest`, or a crash mid-copy, never
/// observes a partially written script. `fs::copy` carries the source
/// file's permission bits to the destination (`statusline.sh` ships mode
/// 0755), so the placed script stays executable with no separate chmod.
fn copy_statusline_atomically(src: &Path, dest: &Path) -> io::Result<()> {
    let dir = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp_path = dir.join(format!(".statusline-{}.tmp", std::process::id()));
    if let Err(err) = fs::copy(src, &tmp_path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    if let Err(err) = fs::rename(&tmp_path, dest) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err);
    }
    Ok(())
}

/// Confirm `path` is a regular, readable file. Stats and then opens it,
/// rather than assuming the copy above did its job: a stat can succeed on a
/// file this process lacks permission to read, so only an actual open
/// proves "readable".
fn verify_placed(path: &Path) -> Result<(), StatuslineError> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(StatuslineError::Settings(format!(
            "{path:?} is not a regular file after placement"
        )));
    }
    fs::File::open(path)?;
    Ok(())
}
