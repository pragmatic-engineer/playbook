// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Walks the files a scan of shipped text reads, shared by the tests that
//! check what playbook ships.

use std::fs;
use std::path::{Path, PathBuf};

/// Every file under `dir` (or `dir` itself, when it is a file) whose
/// extension is one of `exts`. A missing path adds nothing.
pub fn files_under(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    if dir.is_file() {
        if dir
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e))
        {
            out.push(dir.to_path_buf());
        }
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        files_under(&entry.path(), exts, out);
    }
}
