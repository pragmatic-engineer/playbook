// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Places `statusline.sh` at `$HOME/.config/playbook/statusline.sh`, a fixed
//! path. The committed `statusLine.command` is now `playbook statusline`;
//! the script stays as a fallback for one release, then init stops placing it.

use crate::common::paths::playbook_root_from;
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

/// `$HOME/.config/playbook/statusline.sh`, the fixed destination both this
/// module and `settings.shared.json`'s `statusLine.command` derive from.
pub fn playbook_statusline_path(home: &Path) -> PathBuf {
    playbook_root_from(home).join("statusline.sh")
}

/// The command that renders the status line with the Rust binary.
pub const RUST_COMMAND: &str = "playbook statusline";

/// Whether `command` runs the playbook-installed `statusline.sh` through an
/// interpreter. A custom script elsewhere, or any extra argument, is not it.
fn is_legacy_command(command: &str, home: &Path) -> bool {
    let mut tokens = command.split_whitespace();
    let (Some(interp), Some(path), None) = (tokens.next(), tokens.next(), tokens.next()) else {
        return false;
    };
    let home_str = home.to_string_lossy();
    let expanded = match path.strip_prefix("~/") {
        Some(rest) => format!("{home_str}/{rest}"),
        None => path.replace("$HOME", &home_str),
    };
    interp == "bash" && Path::new(&expanded) == playbook_statusline_path(home)
}

/// Point a `statusLine.command` that runs the playbook `statusline.sh` at the
/// Rust renderer, leaving every other value alone. `Ok(true)` when it changed.
pub fn migrate_legacy_command(settings_path: &Path, home: &Path) -> io::Result<bool> {
    let Ok(text) = fs::read_to_string(settings_path) else {
        return Ok(false);
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(false);
    };
    let Some(command) = value
        .pointer_mut("/statusLine/command")
        .filter(|c| c.as_str().is_some_and(|c| is_legacy_command(c, home)))
    else {
        return Ok(false);
    };
    *command = serde_json::Value::String(RUST_COMMAND.to_string());
    let body = serde_json::to_string_pretty(&value).map_err(io::Error::other)?;
    // A symlinked settings.json (stow, chezmoi) stays a symlink.
    let target = fs::canonicalize(settings_path)?;
    crate::common::atomic::write_atomic(&target, &format!("{body}\n"))?;
    Ok(true)
}

/// Place `statusline.sh` at `playbook_statusline_path(home)`, then read it
/// back to confirm it is readable rather than trusting the copy succeeded.
pub fn place_statusline(self_root: &Path, home: &Path) -> Result<PathBuf, StatuslineError> {
    let dest = playbook_statusline_path(home);
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

/// Extract the script path a plain interpreter `statusLine.command` invokes,
/// expanding a literal `$HOME` token to `home`. Last token wins; not a parser.
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
