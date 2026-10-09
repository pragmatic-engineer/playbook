// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The only code that opens Claude Code's auto memory, and it only reads.
//! Nothing here may write, create, rename, copy or delete: `tests/memory_isolation.rs`
//! fails if a write API appears in this file.

use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::Path;

/// One markdown note from Claude Code's auto memory directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeFact {
    /// File name, such as `user-role.md`.
    pub file_name: String,
    pub content: String,
    /// SHA-256 of the content, as lowercase hex.
    pub hash: String,
}

/// Every `.md` note in `dir`, sorted by file name, except the `MEMORY.md`
/// index. A missing directory is an empty list. A file that is not UTF-8 text
/// is skipped.
pub fn read_facts(dir: &Path) -> io::Result<Vec<ClaudeFact>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut facts = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !path.is_file() || !name.ends_with(".md") || name.eq_ignore_ascii_case("MEMORY.md") {
            continue;
        }
        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };
        let hash = format!("{:x}", Sha256::digest(content.as_bytes()));
        facts.push(ClaudeFact {
            file_name: name.to_string(),
            content,
            hash,
        });
    }
    facts.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Ok(facts)
}
