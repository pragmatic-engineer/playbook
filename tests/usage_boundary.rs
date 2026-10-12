// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Keeps `src/usage` extractable: only `host.rs` and `util.rs` may name the
//! rest of the crate. Everything else reaches the host through them.

use std::fs;
use std::path::{Path, PathBuf};

const HOST_FILES: &[&str] = &["host.rs", "util.rs"];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

/// The `crate::` paths on a line whose first segment is not `usage`.
fn foreign(line: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("crate::") {
        let after = &rest[at + "crate::".len()..];
        let head: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if head != "usage" {
            found.push(format!("crate::{head}"));
        }
        rest = after;
    }
    found
}

#[test]
fn usage_names_the_host_crate_only_in_host_and_util() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/usage");
    let mut files = Vec::new();
    rs_files(&root, &mut files);
    let mut bad = Vec::new();
    for file in files {
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let in_root = file.parent() == Some(root.as_path());
        if in_root && HOST_FILES.contains(&name) {
            continue;
        }
        let text = fs::read_to_string(&file).expect("read file");
        for (i, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for path in foreign(line) {
                bad.push(format!("{}:{}: {path}", file.display(), i + 1));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "route these through usage::host or usage::util:\n{}",
        bad.join("\n")
    );
}
