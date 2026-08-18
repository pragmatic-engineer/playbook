// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Launcher internals, ported from `shell/shared/*.sh` for WU-16.
//!
//! These run before and after every `cc` launch, so each one degrades to a
//! no-op rather than failing: a launcher that refuses to start because a cache
//! directory is missing is worse than a stale cache.

pub mod bust_cache;
pub mod retention;
pub mod sessions;

use std::path::PathBuf;

/// Per-project directory name: every non-alphanumeric character becomes `-`.
/// Matches the `${PWD//[^a-zA-Z0-9]/-}` expansion the shell modules share, and
/// the slug must stay identical or the binary would look in a different
/// directory than the shell launcher wrote to.
pub fn project_slug(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

pub fn claude_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude")
}

/// The LOGICAL working directory, preferring `$PWD` over `current_dir()`.
///
/// The two disagree wherever a path component is a symlink, and macOS ships
/// `/tmp` and `/var` as symlinks into `/private`. The shell slug is built from
/// `$PWD`, so resolving the path here would produce a different project
/// directory than the launcher wrote to, and retention would silently prune
/// nothing.
pub fn logical_cwd() -> String {
    match std::env::var("PWD") {
        Ok(pwd) if !pwd.is_empty() => pwd,
        _ => std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
    }
}
