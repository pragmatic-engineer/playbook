// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Display slicing for long converted documents: a section index, line
//! bounded slices, and the private cache that holds the full Markdown.

use super::tmp::random_hex;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Most sections the frame header lists.
pub const MAX_SECTIONS: usize = 200;

const MAX_HEADING_LEVEL: usize = 6;
const MAX_TITLE_CHARS: usize = 100;
const NO_SECTIONS_LINE: &str = "sections: none, use --offset";
const CACHE_SUBDIR: [&str; 2] = ["doc", "cache"];
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
const HASH_CHUNK_BYTES: usize = 64 * 1024;
const HASH_HEX_CHARS: usize = 16;
const TEMP_SUFFIX_CHARS: usize = 8;
const UNKNOWN_VERSION: &str = "unknown";
const TRUNCATION_SUFFIX: &str = ".trunc";

fn is_fence(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("```") || line.starts_with("~~~")
}

/// The title when `line` is an ATX heading of level 1 to 6.
fn heading_title(line: &str) -> Option<String> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > MAX_HEADING_LEVEL {
        return None;
    }
    let rest = line[hashes..].trim_end();
    let title = rest.strip_prefix(' ')?.trim();
    Some(title.chars().take(MAX_TITLE_CHARS).collect())
}

/// Each Markdown heading outside a fenced block as (number from 1, title,
/// character offset of the heading line).
pub fn section_index(md: &str) -> Vec<(usize, String, usize)> {
    let mut found = Vec::new();
    let mut in_fence = false;
    let mut offset = 0;
    for line in md.split_inclusive('\n') {
        if is_fence(line) {
            in_fence = !in_fence;
        } else if !in_fence {
            if let Some(title) = heading_title(line) {
                found.push((found.len() + 1, title, offset));
            }
        }
        offset += line.chars().count();
    }
    found
}

/// The header listing: one line per section, capped at `MAX_SECTIONS` with a
/// "N more" line for the rest.
pub fn section_listing(md: &str) -> String {
    let index = section_index(md);
    if index.is_empty() {
        return NO_SECTIONS_LINE.to_string();
    }
    let mut out = String::from("sections (read one with --section N):\n");
    for (n, title, offset) in index.iter().take(MAX_SECTIONS) {
        out.push_str(&format!("{n}. {title} (offset {offset})\n"));
    }
    if index.len() > MAX_SECTIONS {
        out.push_str(&format!("{} more\n", index.len() - MAX_SECTIONS));
    }
    out
}

/// The byte index of character `n`, or the text length when it is past the end.
fn byte_index(text: &str, n: usize) -> usize {
    text.char_indices().nth(n).map_or(text.len(), |(i, _)| i)
}

/// Up to `limit` characters from character `offset`, ending on a line
/// boundary, or on a character boundary inside a line longer than `limit`.
/// The second value is the offset of the next slice, `None` at the end.
pub fn slice(md: &str, offset: usize, limit: usize) -> (String, Option<usize>) {
    let start = byte_index(md, offset);
    let rest = &md[start..];
    let window_end = byte_index(rest, limit.max(1));
    if window_end == rest.len() {
        return (rest.to_string(), None);
    }
    let window = &rest[..window_end];
    let end = window.rfind('\n').map_or(window_end, |i| i + 1);
    let text = &rest[..end];
    (text.to_string(), Some(offset + text.chars().count()))
}

/// The cache directory under the playbook root.
pub fn cache_dir(root: &Path) -> PathBuf {
    CACHE_SUBDIR
        .iter()
        .fold(root.to_path_buf(), |p, s| p.join(s))
}

/// The cache file name: the first 16 hex of the input's SHA-256, the converter
/// version and the output cap, so a change to any of them is a new entry.
pub fn cache_name(input: &Path, version: &str, max_output_chars: usize) -> io::Result<String> {
    let mut file = fs::File::open(input)?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; HASH_CHUNK_BYTES];
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        hasher.update(&chunk[..n]);
    }
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let version: String = version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let version = if version.is_empty() {
        UNKNOWN_VERSION.to_string()
    } else {
        version
    };
    Ok(format!(
        "{}-{version}-{max_output_chars}.md",
        &hex[..HASH_HEX_CHARS]
    ))
}

/// Write `content` as `name` in `dir` (directory 0700, file 0600) through a
/// unique temp name and a rename, so concurrent writers never collide.
pub fn write_cache(dir: &Path, name: &str, content: &str) -> io::Result<PathBuf> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE))?;
    let temp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        random_hex(TEMP_SUFFIX_CHARS)
    ));
    let target = dir.join(name);
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(FILE_MODE)
        .open(&temp)
        .and_then(|mut f| f.write_all(content.as_bytes()))
        .and_then(|()| fs::rename(&temp, &target));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written.map(|()| target)
}

/// The sidecar name that records a truncated entry's original length.
pub fn truncation_name(name: &str) -> String {
    format!("{name}{TRUNCATION_SUFFIX}")
}

/// Record that the entry `name` was cut from `total_chars` characters. Write
/// it before the entry itself, so an entry that exists never lacks its record.
pub fn write_truncation(dir: &Path, name: &str, total_chars: usize) -> io::Result<PathBuf> {
    write_cache(dir, &truncation_name(name), &total_chars.to_string())
}

/// The original length recorded for the entry at `path`, `None` when the
/// entry was not truncated. Keeps the sidecar alive like `read_cache`.
pub fn read_truncation(path: &Path) -> Option<usize> {
    let mut sidecar = path.as_os_str().to_owned();
    sidecar.push(TRUNCATION_SUFFIX);
    read_cache(Path::new(&sidecar))?.trim().parse().ok()
}

/// The cached text, with its modified time pushed to now so the sweep keeps
/// a file that is still being read. `None` on any miss, including a file the
/// sweep or a user removed.
pub fn read_cache(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    if let Ok(file) = fs::File::open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
    Some(text)
}

/// Remove regular cache files older than `max_age`, never symlinks. Returns
/// how many were removed.
pub fn sweep_cache(dir: &Path, max_age: Duration) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let age = meta
            .modified()
            .ok()
            .and_then(|m| SystemTime::now().duration_since(m).ok())
            .unwrap_or_default();
        if age >= max_age && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}
