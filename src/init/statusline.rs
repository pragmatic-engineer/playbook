// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Moves a legacy `statusLine.command` that runs the old `statusline.sh` over
//! to `playbook statusline`. Init no longer places the script.

use crate::common::paths::playbook_root_from;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// `$HOME/.config/playbook/statusline.sh`, where older installs placed the script.
pub fn playbook_statusline_path(home: &Path) -> PathBuf {
    playbook_root_from(home).join("statusline.sh")
}

/// The command that renders the status line with the Rust binary.
pub const RUST_COMMAND: &str = "playbook statusline";

/// Whether `command` runs the playbook-installed `statusline.sh` through an
/// interpreter. A custom script elsewhere, or any extra argument, is not it.
pub(crate) fn is_legacy_command(command: &str, home: &Path) -> bool {
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
/// With `script_edited` the user owns the script, so the command stays and the
/// merge baseline drops its `statusLine` so the template cannot repoint it.
pub fn migrate_legacy_command(
    settings_path: &Path,
    home: &Path,
    script_edited: bool,
) -> io::Result<bool> {
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
    if script_edited {
        unpin_base_status_line(&settings_path.with_file_name(".settings.base.json"))?;
        return Ok(false);
    }
    *command = serde_json::Value::String(RUST_COMMAND.to_string());
    let body = serde_json::to_string_pretty(&value).map_err(io::Error::other)?;
    // A symlinked settings.json (stow, chezmoi) stays a symlink.
    let target = fs::canonicalize(settings_path)?;
    crate::common::atomic::write_atomic(&target, &format!("{body}\n"))?;
    Ok(true)
}

/// Drop `statusLine` from the merge baseline so the user's value counts as a customisation.
fn unpin_base_status_line(base_path: &Path) -> io::Result<()> {
    let Ok(text) = fs::read_to_string(base_path) else {
        return Ok(());
    };
    let Ok(serde_json::Value::Object(mut base)) = serde_json::from_str(&text) else {
        return Ok(());
    };
    if base.remove("statusLine").is_none() {
        return Ok(());
    }
    let body = serde_json::to_string_pretty(&base).map_err(io::Error::other)?;
    crate::common::atomic::write_atomic(base_path, &format!("{body}\n"))
}
