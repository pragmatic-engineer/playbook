// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports the retired shell original: a PreToolUse hook on Read that denies
//! a full-file Read of a large file when no offset/limit was given, pushing
//! Claude toward Grep-first, then a targeted Read. Allowlists a small set of
//! config/docs files that are usually needed whole.
//!
//! The only hook in the toolkit that returns a deny decision, so its output
//! must match the retired shell original's `emit_pre_deny` byte for byte; see
//! `crate::common::emit::emit_pre_deny`.

use crate::common::emit_pre_deny;
use crate::common::payload::Payload;
use std::fs;
use std::io::Read;
use std::path::Path;

/// Matches LINE_LIMIT in the retired shell original.
const LINE_LIMIT: u64 = 1000;
/// Matches BYTE_LIMIT in the retired shell original (200 KB).
const BYTE_LIMIT: u64 = 204_800;
/// Most bytes scanned for a file already over `BYTE_LIMIT`, where the count
/// only fills the message. Files at or under the limit are always counted whole.
const SCAN_CAP: u64 = 4 * 1024 * 1024;

/// Matches ALLOWLIST in the retired shell original.
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

    // Line count = newline count (matches `wc -l`); byte size from stat. A
    // read failure never panics and counts 0 lines, but `fs::metadata` still
    // succeeds for an unreadable file, so a large one is still denied on bytes.
    let num_bytes = fs::metadata(file_path).map(|meta| meta.len()).unwrap_or(0);
    let counted = count_newlines(file_path, num_bytes);
    let lines = counted.lines;

    // Files at or below either threshold pass.
    if lines <= LINE_LIMIT && num_bytes <= BYTE_LIMIT {
        return;
    }
    let lines = match (counted.complete, lines > LINE_LIMIT) {
        (true, _) => lines.to_string(),
        (false, true) => format!("over {LINE_LIMIT}"),
        (false, false) => format!("at least {lines}"),
    };

    // Built as one literal line (rather than backslash-newline continuations)
    // so the significant leading spaces on the numbered list below cannot be
    // stripped by rustfmt reindenting a multi-line string literal.
    let reason = format!(
        "This file is {lines} lines / {num_bytes} bytes, too large to Read in full.\n\nCheaper approaches:\n  1. Grep the file first to find the relevant line ranges.\n  2. Re-call Read with offset:<line> and limit:<rows> for the section you need.\n  3. If you really need the whole file (e.g. a small minified bundle), re-issue\n     with explicit offset:0, limit:9999 to override this guard.\n\nWhy this matters: full Reads on large files burn input tokens that almost never\npay back. Most callers only use 10-20% of the content."
    );
    emit_pre_deny(&reason);
}

struct Counted {
    lines: u64,
    /// False when the scan stopped early, so `lines` is a lower bound.
    complete: bool,
}

/// Newlines in `path`, streamed in constant memory. Under `BYTE_LIMIT` the
/// count is exact; above it the scan stops once the line limit is passed or
/// `SCAN_CAP` bytes are read, since the file is denied either way.
fn count_newlines(path: &Path, size: u64) -> Counted {
    let Ok(file) = fs::File::open(path) else {
        return Counted {
            lines: 0,
            complete: true,
        };
    };
    if size <= BYTE_LIMIT {
        return count_newlines_in(file, None, u64::MAX);
    }
    count_newlines_in(file, Some(LINE_LIMIT), SCAN_CAP)
}

/// Streams `reader` in 64 KiB chunks, stopping early once the count passes
/// `stop_after` or `byte_cap` bytes were read.
fn count_newlines_in(mut reader: impl Read, stop_after: Option<u64>, byte_cap: u64) -> Counted {
    let mut buf = [0u8; 64 * 1024];
    let (mut lines, mut read) = (0u64, 0u64);
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => {
                return Counted {
                    lines,
                    complete: true,
                }
            }
            Ok(n) => n,
        };
        read += n as u64;
        lines += buf[..n].iter().filter(|&&b| b == b'\n').count() as u64;
        if stop_after.is_some_and(|limit| lines > limit) || read >= byte_cap {
            return Counted {
                lines,
                complete: false,
            };
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Serves `total` bytes of 80-byte lines and records how many it handed out.
    struct Counting {
        total: u64,
        served: u64,
    }

    impl Read for Counting {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let n = out.len().min((self.total - self.served) as usize);
            for (i, b) in out[..n].iter_mut().enumerate() {
                *b = if (self.served + i as u64) % 80 == 79 {
                    b'\n'
                } else {
                    b'a'
                };
            }
            self.served += n as u64;
            Ok(n)
        }
    }

    #[test]
    fn counts_every_newline_exactly_without_a_stop() {
        // Arrange
        let mut reader = Counting {
            total: 80 * 3000,
            served: 0,
        };

        // Act
        let got = count_newlines_in(&mut reader, None, u64::MAX);

        // Assert
        assert_eq!((got.lines, got.complete), (3000, true));
        assert_eq!(reader.served, 80 * 3000);
    }

    #[test]
    fn stops_reading_soon_after_the_line_limit_is_passed() {
        // Arrange: a virtual 10 GB file that must never be read in full.
        let mut reader = Counting {
            total: 10 * 1024 * 1024 * 1024,
            served: 0,
        };

        // Act
        let got = count_newlines_in(&mut reader, Some(LINE_LIMIT), SCAN_CAP);

        // Assert
        assert!(!got.complete);
        assert!(got.lines > LINE_LIMIT);
        assert!(reader.served <= 128 * 1024, "read {} bytes", reader.served);
    }

    #[test]
    fn a_scan_that_never_hits_the_line_stop_ends_at_the_byte_cap() {
        // Arrange
        let mut reader = Counting {
            total: 1 << 40,
            served: 0,
        };

        // Act
        let got = count_newlines_in(&mut reader, Some(u64::MAX), SCAN_CAP);

        // Assert
        assert!(!got.complete);
        assert!(reader.served < SCAN_CAP + 64 * 1024);
    }
}
