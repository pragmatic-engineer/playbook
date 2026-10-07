// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Guard: comments in `src` and `tests` must not cite a retired script path.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::Command;

fn tracked_files() -> Vec<String> {
    let out = Command::new("git")
        .args(["ls-files"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run git ls-files");
    assert!(out.status.success(), "git ls-files failed");
    String::from_utf8(out.stdout)
        .expect("utf8 listing")
        .lines()
        .map(str::to_string)
        .collect()
}

/// Path-like tokens a comment cites: `hooks/<name>.py`, `shell/<...>.sh|py`,
/// `hooks/<...>.sh` and `SEGMENT-B-RULES.md`.
fn cited_paths(line: &str) -> Vec<String> {
    let is_path_char = |c: char| c.is_ascii_alphanumeric() || "_./-".contains(c);
    let mut found = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        let start = rest.find(|c: char| is_path_char(c)).unwrap_or(rest.len());
        let tail = &rest[start..];
        let len = tail.find(|c: char| !is_path_char(c)).unwrap_or(tail.len());
        let token = tail[..len].trim_end_matches('.');
        let scripted = token.ends_with(".sh") || token.ends_with(".py");
        let dir_ok = token.starts_with("hooks/") || token.starts_with("shell/");
        if (dir_ok && scripted) || token == "SEGMENT-B-RULES.md" {
            found.push(token.to_string());
        }
        rest = &tail[len..];
    }
    found
}

#[test]
fn comments_do_not_cite_paths_that_are_not_tracked() {
    // Arrange
    let tracked = tracked_files();
    let known: HashSet<&str> = tracked.iter().map(String::as_str).collect();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    // Act
    let mut stale = Vec::new();
    for file in tracked
        .iter()
        .filter(|f| f.ends_with(".rs") && (f.starts_with("src/") || f.starts_with("tests/")))
        .filter(|f| f.as_str() != "tests/comment_refs.rs")
    {
        let text = fs::read_to_string(root.join(file)).expect("read tracked source");
        for (i, line) in text.lines().enumerate() {
            if !line.trim_start().starts_with("//") {
                continue;
            }
            for path in cited_paths(line) {
                if !known.contains(path.as_str()) {
                    stale.push(format!("{file}:{} cites {path}", i + 1));
                }
            }
        }
    }

    // Assert
    assert!(
        stale.is_empty(),
        "comments cite untracked paths:\n{}",
        stale.join("\n")
    );
}

#[test]
fn cited_paths_finds_retired_script_and_rules_names() {
    let line = "// ports hooks/session-init.py and SEGMENT-B-RULES.md, see shell/cc.sh.";
    assert_eq!(
        cited_paths(line),
        ["hooks/session-init.py", "SEGMENT-B-RULES.md", "shell/cc.sh"]
    );
}
