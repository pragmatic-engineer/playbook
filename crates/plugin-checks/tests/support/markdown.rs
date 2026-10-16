// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The markdown file walker shared by the plugin checks.

use std::fs;
use std::path::{Path, PathBuf};

/// Every `.md` file under `dir`, at any depth. Panics when `dir` cannot be
/// read, so a moved folder fails the check instead of passing it empty.
pub fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "md") {
            out.push(path);
        }
    }
}
