// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Claude Code replaces `$0`..`$9` in a command or skill file with the
//! invocation's argument words before the model reads it, so a shell
//! positional parameter in a fenced block silently becomes an argument word.

use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

/// `(line number, line)` for each fenced-code line that uses `$N` or `${N}`.
fn positional_uses(text: &str) -> Vec<(usize, String)> {
    let pattern = Regex::new(r"\$(\{[0-9]\}|[0-9](?:[^0-9.,]|$))").expect("valid regex");
    let mut in_fence = false;
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence && pattern.is_match(line) {
            found.push((index + 1, line.trim().to_string()));
        }
    }
    found
}

#[test]
fn the_detector_flags_positional_parameters_and_not_prices_or_prose() {
    let text = "Costs $1.50 in prose: $1\n```sh\nf() { cat \"$1\"; }\necho ${2}\necho \"$10.5\" $ARGUMENTS\n```\n";

    let found: Vec<usize> = positional_uses(text).iter().map(|(n, _)| *n).collect();

    assert_eq!(found, vec![3, 4]);
}

#[test]
fn no_command_or_skill_uses_a_shell_positional_parameter() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    markdown_files(&root.join("commands"), &mut files);
    markdown_files(&root.join("skills"), &mut files);
    assert!(!files.is_empty(), "no command or skill files found");

    let mut offenders = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("readable markdown");
        for (line, code) in positional_uses(&text) {
            let shown = file.strip_prefix(&root).unwrap_or(file).display();
            offenders.push(format!("{shown}:{line}: {code}"));
        }
    }

    assert!(
        offenders.is_empty(),
        "use a named variable instead of $N:\n{}",
        offenders.join("\n")
    );
}
