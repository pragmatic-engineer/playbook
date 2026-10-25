// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `outline` and `read`: views of the current file on disk. Neither uses the
//! index, so both work before the first `index` run and never show old text.

use super::lang::{self, Lang};
use super::location;
use super::walker::Limits;
use std::io::Read;
use std::path::Path;

/// Most declarations an outline prints.
pub const OUTLINE_CAP: usize = 500;
/// Most lines and bytes a read prints.
pub const READ_MAX_LINES: usize = 400;
pub const READ_MAX_BYTES: usize = 32 * 1024;

/// What the supported languages are, for error messages.
pub const SUPPORTED: &str =
    "rust (rs), typescript (ts, tsx, mts, cts), javascript (js, jsx, mjs, cjs), python (py, pyi), go (go)";

/// One declaration in an outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Nesting depth, 0 for top level.
    pub depth: usize,
    pub kind: &'static str,
    pub name: String,
    pub qualified_name: String,
    pub signature: Option<String>,
    pub start_line: usize,
    pub end_line: usize,
    pub exported: bool,
}

#[derive(Debug, Default)]
pub struct Outline {
    pub path: String,
    pub lang: &'static str,
    pub entries: Vec<Entry>,
    pub truncated: bool,
    pub omitted: usize,
    /// The file has syntax errors, so the outline may miss declarations.
    pub partial: bool,
}

/// Resolves a user path to a repo relative path and its bytes. Refuses paths
/// outside the repo, non-files and files over the indexer's size cap.
fn read_file(root: &Path, cwd: &Path, input: &str) -> Result<(String, Vec<u8>), String> {
    let rel = location::to_repo_relative(root, cwd, input).map_err(|e| e.to_string())?;
    let full = root.join(&rel);
    let meta = std::fs::metadata(&full).map_err(|e| format!("cannot read {rel}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("not a file: {rel}"));
    }
    if meta.len() > Limits::default().max_file_bytes {
        return Err(format!("file is too large to read: {rel}"));
    }
    let cap = Limits::default().max_file_bytes;
    let mut bytes = Vec::new();
    std::fs::File::open(&full)
        .and_then(|f| f.take(cap + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("cannot read {rel}: {e}"))?;
    if bytes.len() as u64 > cap {
        return Err(format!("file is too large to read: {rel}"));
    }
    Ok((rel, bytes))
}

/// Parses the file on disk and lists its declarations, at most `cap`.
pub fn outline(root: &Path, cwd: &Path, input: &str, cap: usize) -> Result<Outline, String> {
    let (rel, bytes) = read_file(root, cwd, input)?;
    let Some(language) = Lang::for_path(&rel) else {
        return Err(format!(
            "cannot outline {rel}: unsupported language; supported: {SUPPORTED}"
        ));
    };
    let parsed = lang::parse(language, &bytes)?;
    let found = lang::extract(language, &parsed, &bytes);
    let mut depths: Vec<usize> = Vec::with_capacity(found.symbols.len());
    let mut entries = Vec::new();
    for sym in &found.symbols {
        let depth = sym.parent.and_then(|p| depths.get(p)).map_or(0, |d| d + 1);
        depths.push(depth);
        entries.push(Entry {
            depth,
            kind: sym.kind,
            name: sym.name.clone(),
            qualified_name: sym.qualified_name.clone(),
            signature: sym.signature.clone(),
            start_line: sym.start_line,
            end_line: sym.end_line,
            exported: sym.exported,
        });
    }
    let omitted = entries.len().saturating_sub(cap);
    entries.truncate(cap);
    Ok(Outline {
        path: rel,
        lang: language.name(),
        entries,
        truncated: omitted > 0,
        omitted,
        partial: parsed.partial,
    })
}

/// Caps on one read.
#[derive(Debug, Clone, Copy)]
pub struct Caps {
    pub lines: usize,
    pub bytes: usize,
}

impl Default for Caps {
    fn default() -> Self {
        Self {
            lines: READ_MAX_LINES,
            bytes: READ_MAX_BYTES,
        }
    }
}

/// A slice of a file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Slice {
    pub path: String,
    /// First and last line returned, 1 based. `end < start` when nothing returned.
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub total_lines: usize,
    pub truncated: bool,
    /// The requested end was past the end of the file.
    pub clamped: bool,
    /// The last returned line was cut by the byte cap.
    pub last_line_partial: bool,
}

/// Parses `START:END`, both 1 based and inclusive.
pub fn parse_range(spec: &str) -> Result<(usize, usize), String> {
    let bad = || format!("invalid line range `{spec}`: use START:END, for example 10:40");
    let (a, b) = spec.split_once(':').ok_or_else(bad)?;
    let start: usize = a.parse().map_err(|_| bad())?;
    let end: usize = b.parse().map_err(|_| bad())?;
    if start == 0 || end == 0 {
        return Err(format!("invalid line range `{spec}`: lines start at 1"));
    }
    if end < start {
        return Err(format!("invalid line range `{spec}`: END is before START"));
    }
    Ok((start, end))
}

/// Cuts `s` to at most `max` bytes, never inside a character.
pub(crate) fn cut_at_boundary(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Reads lines `start..=end` of the file on disk.
pub fn read_lines(
    root: &Path,
    cwd: &Path,
    input: &str,
    range: (usize, usize),
    caps: Caps,
) -> Result<Slice, String> {
    let (rel, bytes) = read_file(root, cwd, input)?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let (start, requested_end) = range;
    if start > lines.len() {
        return Err(format!(
            "line {start} is past the end of {rel}, which has {} lines",
            lines.len()
        ));
    }
    let clamped = requested_end > lines.len();
    let end = requested_end.min(lines.len());
    let mut out = String::new();
    let mut last = start - 1;
    let mut truncated = false;
    let mut partial = false;
    for (offset, line) in lines[start - 1..end].iter().enumerate() {
        let n = start + offset;
        if offset >= caps.lines {
            truncated = true;
            break;
        }
        let sep = usize::from(offset > 0);
        if out.len() + sep + line.len() > caps.bytes {
            let room = caps.bytes.saturating_sub(out.len() + sep);
            let piece = cut_at_boundary(line, room);
            if !piece.is_empty() {
                if offset > 0 {
                    out.push('\n');
                }
                out.push_str(piece);
                last = n;
                partial = true;
            }
            truncated = true;
            break;
        }
        if offset > 0 {
            out.push('\n');
        }
        out.push_str(line);
        last = n;
    }
    Ok(Slice {
        path: rel,
        start,
        end: last,
        text: out,
        total_lines: lines.len(),
        truncated,
        clamped,
        last_line_partial: partial,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{fx, pinned, Fx};

    fn read(f: &Fx, path: &str, range: (usize, usize), caps: Caps) -> Result<Slice, String> {
        read_lines(&f.root, &f.root, path, range, caps)
    }

    #[test]
    fn outline_lists_nested_declarations_without_an_index() {
        let _g = pinned();
        // Arrange
        let f = fx("src-outline");
        f.write("a.ts", "class A {\n  run() {}\n}\nfunction top() {}\n");

        // Act
        let got = outline(&f.root, &f.root, "a.ts", OUTLINE_CAP).expect("outline");

        // Assert
        let shown: Vec<(usize, &str, &str)> = got
            .entries
            .iter()
            .map(|e| (e.depth, e.kind, e.qualified_name.as_str()))
            .collect();
        assert_eq!(
            shown,
            vec![
                (0, "class", "A"),
                (1, "method", "A.run"),
                (0, "function", "top")
            ]
        );
        assert_eq!(got.lang, "typescript");
    }

    #[test]
    fn outline_of_a_go_file_nests_methods_under_their_receiver_types() {
        let _g = pinned();
        // Arrange
        let f = fx("src-outline-go");
        f.write("store.go", crate::lang::go::FIXTURE);

        // Act
        let got = outline(&f.root, &f.root, "store.go", OUTLINE_CAP).expect("outline");

        // Assert
        let shown: Vec<(usize, &str, &str)> = got
            .entries
            .iter()
            .map(|e| (e.depth, e.kind, e.qualified_name.as_str()))
            .collect();
        assert_eq!(
            shown,
            vec![
                (0, "const", "MaxItems"),
                (0, "variable", "ErrFull"),
                (0, "struct", "Item"),
                (0, "interface", "Store"),
                (1, "method", "Store.Put"),
                (0, "struct", "memory"),
                (0, "function", "New"),
                (1, "method", "memory.Put"),
            ]
        );
        assert_eq!(got.lang, "go");
    }

    #[test]
    fn outline_shows_the_current_file_not_an_older_index() {
        let _g = pinned();
        // Arrange
        let f = fx("src-outline-edit");
        f.write("a.rs", "fn old_name() {}\n");
        f.run();
        f.write("a.rs", "fn new_name() {}\n");

        // Act
        let got = outline(&f.root, &f.root, "a.rs", OUTLINE_CAP).expect("outline");

        // Assert
        let names: Vec<&str> = got.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["new_name"]);
    }

    #[test]
    fn outline_of_an_unsupported_file_lists_the_supported_languages() {
        let _g = pinned();
        // Arrange
        let f = fx("src-outline-txt");
        f.write("a.txt", "hello\n");

        // Act
        let err = outline(&f.root, &f.root, "a.txt", OUTLINE_CAP).expect_err("unsupported");

        // Assert
        assert!(err.contains("supported:"), "{err}");
        assert!(err.contains("python"), "{err}");
    }

    #[test]
    fn outline_caps_of_499_500_and_501_declarations() {
        let _g = pinned();
        for (count, shown, truncated) in [
            (499usize, 499usize, false),
            (500, 500, false),
            (501, 500, true),
        ] {
            // Arrange
            let f = fx(&format!("src-outline-cap-{count}"));
            let body: String = (0..count).map(|i| format!("fn f{i}() {{}}\n")).collect();
            f.write("a.rs", &body);

            // Act
            let got = outline(&f.root, &f.root, "a.rs", OUTLINE_CAP).expect("outline");

            // Assert
            assert_eq!(got.entries.len(), shown, "{count}");
            assert_eq!(got.truncated, truncated, "{count}");
        }
    }

    #[test]
    fn range_specs_reject_zero_reversed_and_non_numeric_parts() {
        // Arrange, Act, Assert
        assert_eq!(parse_range("1:1"), Ok((1, 1)));
        assert_eq!(parse_range("5:9"), Ok((5, 9)));
        for bad in ["5:3", "0:2", "2:0", "a:b", "7", "1:", ":4", "-1:3"] {
            assert!(parse_range(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn earlier_lines_filling_the_cap_leave_no_stray_newline() {
        let _g = pinned();
        // Arrange: two 4-byte lines make 9 bytes with the newline, and the cap is 9.
        let f = fx("src-fill");
        f.write("a.txt", "aaaa\nbbbb\ncccc\n");
        let caps = Caps {
            lines: 400,
            bytes: 9,
        };

        // Act
        let got = read(&f, "a.txt", (1, 3), caps).expect("read");

        // Assert
        assert_eq!(got.text, "aaaa\nbbbb");
        assert_eq!(
            (got.end, got.truncated, got.last_line_partial),
            (2, true, false)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_file_outside_the_repo_is_refused() {
        let _g = pinned();
        // Arrange
        let f = fx("src-link-out");
        let outside = playbook_common::test_support::scratch_dir("src-link-target");
        std::fs::create_dir_all(&outside).expect("dir");
        std::fs::write(outside.join("secret.txt"), "private\n").expect("write");
        std::os::unix::fs::symlink(outside.join("secret.txt"), f.root.join("link.txt"))
            .expect("link");

        // Act
        let err = read(&f, "link.txt", (1, 1), Caps::default()).expect_err("outside");

        // Assert
        assert!(err.contains("outside the repository"), "{err}");
    }

    #[test]
    fn a_path_outside_the_repo_is_refused() {
        let _g = pinned();
        // Arrange
        let f = fx("src-outside");
        f.write("a.txt", "x\n");

        // Act
        let err = read(&f, "../outside", (1, 1), Caps::default()).expect_err("outside");

        // Assert
        assert!(err.contains("outside the repository"), "{err}");
    }

    #[test]
    fn reads_return_exact_lines_clamp_at_eof_and_refuse_a_start_past_the_end() {
        let _g = pinned();
        // Arrange
        let f = fx("src-read");
        f.write("a.txt", "one\ntwo\nthree");

        // Act
        let first = read(&f, "a.txt", (1, 1), Caps::default()).expect("read");
        let past = read(&f, "a.txt", (2, 99), Caps::default()).expect("read");
        let err = read(&f, "a.txt", (4, 4), Caps::default()).expect_err("past end");

        // Assert
        assert_eq!((first.text.as_str(), first.start, first.end), ("one", 1, 1));
        assert_eq!(
            (past.text.as_str(), past.end, past.clamped),
            ("two\nthree", 3, true)
        );
        assert!(err.contains("past the end"), "{err}");
    }

    #[test]
    fn crlf_endings_and_a_missing_final_newline_keep_exact_lines() {
        let _g = pinned();
        // Arrange
        let f = fx("src-crlf");
        f.write("a.txt", "a\r\nb\r\nc");

        // Act
        let got = read(&f, "a.txt", (1, 3), Caps::default()).expect("read");

        // Assert
        assert_eq!(got.text, "a\nb\nc");
        assert_eq!(got.total_lines, 3);
    }

    #[test]
    fn line_caps_of_399_400_and_401_lines() {
        let _g = pinned();
        for (count, shown, truncated) in [
            (399usize, 399usize, false),
            (400, 400, false),
            (401, 400, true),
        ] {
            // Arrange
            let f = fx(&format!("src-lines-{count}"));
            let body: String = (1..=count).map(|i| format!("l{i}\n")).collect();
            f.write("a.txt", &body);

            // Act
            let got = read(&f, "a.txt", (1, count), Caps::default()).expect("read");

            // Assert
            assert_eq!(got.end, shown, "{count}");
            assert_eq!(got.truncated, truncated, "{count}");
        }
    }

    #[test]
    fn byte_caps_of_1023_1024_and_1025_bytes() {
        let _g = pinned();
        let caps = Caps {
            lines: 400,
            bytes: 1024,
        };
        for (len, truncated) in [(1023usize, false), (1024, false), (1025, true)] {
            // Arrange
            let f = fx(&format!("src-bytes-{len}"));
            f.write("a.txt", &format!("{}\n", "x".repeat(len)));

            // Act
            let got = read(&f, "a.txt", (1, 1), caps).expect("read");

            // Assert
            assert_eq!(got.truncated, truncated, "{len}");
            assert_eq!(got.text.len(), len.min(1024), "{len}");
        }
    }

    #[test]
    fn a_byte_cap_inside_a_multibyte_character_ends_at_the_previous_boundary() {
        let _g = pinned();
        // Arrange: each e-acute is two bytes, so a cap of 5 lands inside the third one.
        let f = fx("src-multibyte");
        f.write("a.txt", "\u{e9}\u{e9}\u{e9}\u{e9}\n");
        let caps = Caps {
            lines: 400,
            bytes: 5,
        };

        // Act
        let got = read(&f, "a.txt", (1, 1), caps).expect("read");

        // Assert
        assert_eq!(got.text, "\u{e9}\u{e9}");
        assert!(got.truncated);
    }

    #[test]
    fn a_read_shows_an_edit_made_after_indexing() {
        let _g = pinned();
        // Arrange
        let f = fx("src-read-edit");
        f.write("a.txt", "before\n");
        f.run();
        f.write("a.txt", "after\n");

        // Act
        let got = read(&f, "a.txt", (1, 1), Caps::default()).expect("read");

        // Assert
        assert_eq!(got.text, "after");
    }
}
