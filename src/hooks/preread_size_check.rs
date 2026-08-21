// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports hooks/preread-size-check.py: a PreToolUse hook on Read that denies
//! a full-file Read of a large file when no offset/limit was given, pushing
//! Claude toward Grep-first, then a targeted Read. Allowlists a small set of
//! config/docs files that are usually needed whole.
//!
//! The only hook in the toolkit that returns a deny decision, so its output
//! must match hooks/lib/common.py's `emit_pre_deny` byte for byte; see
//! `crate::common::emit::emit_pre_deny`.

use crate::common::emit_pre_deny;
use crate::common::payload::Payload;
use std::fs;
use std::path::Path;

/// Matches LINE_LIMIT in hooks/preread-size-check.py:16.
const LINE_LIMIT: u64 = 1000;
/// Matches BYTE_LIMIT in hooks/preread-size-check.py:17 (200 KB).
const BYTE_LIMIT: u64 = 204_800;

/// Matches ALLOWLIST in hooks/preread-size-check.py:20-27.
const ALLOWLIST: [&str; 26] = [
    "package.json",
    "tsconfig.json",
    "tsconfig.*.json",
    "pyproject.toml",
    "go.mod",
    "go.sum",
    "Cargo.toml",
    "Cargo.lock",
    "Gemfile",
    "Gemfile.lock",
    "requirements.txt",
    "CLAUDE.md",
    "README.md",
    "README",
    "CHANGELOG.md",
    "LICENSE",
    ".gitignore",
    ".dockerignore",
    "Dockerfile",
    "docker-compose.yml",
    "docker-compose.yaml",
    "Makefile",
    "justfile",
    ".env.example",
    "settings.json",
    "settings.local.json",
];

pub fn run(payload: &Payload) {
    let path = payload.field(".tool_input.file_path");
    if path.is_empty() {
        return;
    }
    let file_path = Path::new(&path);
    if !file_path.is_file() {
        return;
    }

    // Honour explicit offset/limit: caller already knows what it is doing.
    if !payload.field(".tool_input.offset").is_empty()
        || !payload.field(".tool_input.limit").is_empty()
    {
        return;
    }

    let base = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if ALLOWLIST.iter().any(|pattern| glob_match(pattern, base)) {
        return;
    }

    // Line count = newline count (matches `wc -l`); byte size from stat.
    // Two independent reads, matching the python port: a read failure
    // (an unreadable target) never panics and defaults the line count to 0,
    // but `fs::metadata` is a separate call that still succeeds for an
    // existing, unreadable file, so a large unreadable file is still
    // correctly denied on byte size, matching python.
    let lines = count_newlines(file_path);
    let num_bytes = fs::metadata(file_path).map(|meta| meta.len()).unwrap_or(0);

    // Files at or below either threshold pass.
    if lines <= LINE_LIMIT && num_bytes <= BYTE_LIMIT {
        return;
    }

    // Built as one literal line (rather than backslash-newline continuations)
    // so the significant leading spaces on the numbered list below cannot be
    // stripped by rustfmt reindenting a multi-line string literal.
    let reason = format!(
        "This file is {lines} lines / {num_bytes} bytes, too large to Read in full.\n\nCheaper approaches:\n  1. Grep the file first to find the relevant line ranges.\n  2. Re-call Read with offset:<line> and limit:<rows> for the section you need.\n  3. If you really need the whole file (e.g. a small minified bundle), re-issue\n     with explicit offset:0, limit:9999 to override this guard.\n\nWhy this matters: full Reads on large files burn input tokens that almost never\npay back. Most callers only use 10-20% of the content."
    );
    emit_pre_deny(&reason);
}

fn count_newlines(path: &Path) -> u64 {
    fs::read(path)
        .map(|data| data.iter().filter(|&&byte| byte == b'\n').count() as u64)
        .unwrap_or(0)
}

/// Minimal case-sensitive glob match supporting `*` (any run of characters,
/// including none) and `?` (any single character): the only fnmatch
/// metacharacters ALLOWLIST above actually uses. Not a general fnmatch
/// implementation (no `[seq]` support), since the allowlist never needs one.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_text_pos = 0usize;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            star_text_pos = ti;
            pi += 1;
        } else if let Some(star_pi) = star {
            pi = star_pi + 1;
            star_text_pos += 1;
            ti = star_text_pos;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}
