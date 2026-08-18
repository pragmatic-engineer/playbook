// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports `shell/shared/bust-cache.sh`: clears caches that would otherwise
//! freeze stale settings into a new session.
//!
//! Every removal is best-effort. This runs on the launch path, so a missing
//! directory or an unreadable file must not stop the session from starting.

use super::claude_dir;
use std::fs;
use std::path::Path;

pub fn bust() {
    let dir = claude_dir();

    // A snapshot can pin an old statusLine or environment into the session.
    remove_matching(&dir.join("shell-snapshots"), |name| {
        name.starts_with("snapshot-") && name.ends_with(".sh")
    });

    // config-hash lets Claude Code skip re-reading settings on resume, so it
    // has to go for settings.json, plugins and the status line to be re-read.
    // Nested one level under runtime/<session-id>/, hence the recursive walk.
    remove_named_recursive(&dir.join("runtime"), "config-hash");

    let _ = fs::remove_file(dir.join("plugins").join("plugin-catalog-cache.json"));

    remove_shallow_children(&dir.join("backups"));
}

fn remove_matching(dir: &Path, matches: impl Fn(&str) -> bool) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if matches(&name) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn remove_named_recursive(dir: &Path, target: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_named_recursive(&path, target);
        } else if entry.file_name() == target {
            let _ = fs::remove_file(path);
        }
    }
}

/// Files go, directories go only when already empty.
///
/// This mirrors `find backups -mindepth 1 -maxdepth 1 -delete`, which cannot
/// remove a non-empty directory: `-delete` uses rmdir and simply fails there.
/// So a populated `backups/install-<stamp>/` survives a launch, and using
/// `remove_dir_all` here would silently destroy backups the shell preserved.
fn remove_shallow_children(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let _ = fs::remove_dir(path);
        } else {
            let _ = fs::remove_file(path);
        }
    }
}
