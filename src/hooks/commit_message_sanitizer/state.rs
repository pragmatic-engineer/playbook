// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The HEAD each repository had before a Bash call that commits ran, kept per
//! session in a small JSON file keyed by the repository's top directory, so the
//! PostToolUse backstop can tell a commit this call made from one that was
//! already there. A repository with no commit yet is recorded with an empty HEAD.

use crate::common::atomic::write_atomic;
use crate::common::payload::Payload;
use crate::common::session_dir;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

const FILE: &str = "commit-sanitizer-heads";

/// Remembers the HEAD of the repository `dir` is in. Nothing is kept when the
/// call has no session or `dir` is not in a repository.
pub fn record_head(payload: &Payload, dir: &Path) {
    let (Some(file), Some(root)) = (state_file(payload), repo_root(dir)) else {
        return;
    };
    let head = crate::common::gitfacts::head_sha(dir)
        .or_else(|| crate::common::git::raw(dir, &["rev-parse", "--verify", "-q", "HEAD"]))
        .unwrap_or_default();
    let mut heads = read(&file);
    heads.insert(root, Value::String(head.trim().to_string()));
    let _ = write_atomic(&file, Value::Object(heads).to_string());
}

/// The HEAD recorded for the repository `dir` is in, which is then forgotten
/// so a later call cannot take it for its own.
pub fn take_head(payload: &Payload, dir: &Path) -> Option<String> {
    let (file, root) = (state_file(payload)?, repo_root(dir)?);
    let mut heads = read(&file);
    let head = heads.remove(&root)?;
    let _ = write_atomic(&file, Value::Object(heads).to_string());
    head.as_str().map(str::to_string)
}

fn state_file(payload: &Payload) -> Option<PathBuf> {
    let dir = session_dir(payload);
    (!dir.is_empty()).then(|| Path::new(&dir).join(FILE))
}

fn repo_root(dir: &Path) -> Option<String> {
    if let Some(top) = crate::common::gitfacts::toplevel(dir) {
        return Some(top.to_string_lossy().into_owned());
    }
    Some(
        crate::common::git::raw(dir, &["rev-parse", "--show-toplevel"])?
            .trim()
            .to_string(),
    )
}

fn read(file: &Path) -> Map<String, Value> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or_default()
}
