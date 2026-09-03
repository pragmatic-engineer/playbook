// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports `shell/shared/config-drift.sh`: tracks whether runtime config changed
//! since a project last launched, so the default resume can fork to reload
//! settings, plugins and hooks only when it has to.
//!
//! The baseline is one file per project under `~/.config/playbook/cc-state/`.

use super::{claude_dir, project_slug};
use crate::common::{config_hash, paths};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The shell ran the hash unbounded. A wedged `git` inside config-hash.sh would
/// have stalled every launch, so the port bounds it and treats a timeout as an
/// unknown hash.
const HASH_TIMEOUT: Duration = Duration::from_secs(5);

pub fn marker_path(cwd: &str) -> PathBuf {
    paths::cc_state_dir().join(project_slug(cwd))
}

/// Records the current config as this project's baseline. Called when launching
/// a session that already runs current config: fresh, clean, or new.
pub fn stamp(cwd: &str) {
    let marker = marker_path(cwd);
    write_marker(&marker, &current_hash());
}

/// True when config changed since this project's baseline.
///
/// **Always re-stamps, match or not.** That is the shell's behaviour and it is
/// load-bearing: the caller uses the answer to decide `--fork-session` once,
/// and leaving the old baseline in place would make every subsequent launch
/// report drift until something else happened to stamp it.
pub fn drifted(cwd: &str) -> bool {
    let marker = marker_path(cwd);
    let stored = fs::read_to_string(&marker).unwrap_or_default();
    let current = current_hash();
    write_marker(&marker, &current);
    stored.trim() != current.trim()
}

fn current_hash() -> String {
    config_hash(&claude_dir(), HASH_TIMEOUT)
}

/// Trailing newline included, because the shell wrote with `printf '%s\n'` and
/// `cat` on the way back in, so a stored value always ends in one. Comparisons
/// trim, but the bytes on disk should still match.
fn write_marker(marker: &Path, hash: &str) {
    if let Some(parent) = marker.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(marker, format!("{hash}\n"));
}
