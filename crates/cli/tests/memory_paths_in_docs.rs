// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Shipped text must name the real home of the memory graph,
//! `~/.config/playbook/data/memory.db`. A command that sends the model to
//! `memory.graph.json` or to `~/.config/playbook/memory.db` makes it look in
//! the wrong place and reach for `sqlite3`. ADRs are history and are not
//! scanned; the few docs that describe the old file as history are listed
//! by name below.

use std::fs;
use std::path::{Path, PathBuf};

/// Folders and files the plugin, the setup and the docs ship.
const SCANNED: [&str; 6] = [
    "plugins",
    "prompts",
    "seed",
    "docs",
    "README.md",
    "CONTRIBUTING.md",
];

/// Not scanned: decision records keep the paths of their day.
const SKIPPED_DIRS: [&str; 1] = ["docs/adr"];

/// Docs that name `memory.graph.json` only as the file older versions kept.
const OLD_FILE_HISTORY: [&str; 4] = [
    "docs/concepts/02-memory-system.md",
    "docs/guides/06-what-init-installs.md",
    "docs/internals/11-data-folder.md",
    "docs/reference/cli-and-config.md",
];

fn root() -> PathBuf {
    PathBuf::from(env!("PLAYBOOK_REPO_ROOT"))
}

fn text_files(path: &Path, rel: &str, out: &mut Vec<(String, PathBuf)>) {
    if SKIPPED_DIRS.contains(&rel) {
        return;
    }
    if path.is_file() {
        if [".md", ".json", ".sh", ".txt"]
            .iter()
            .any(|e| rel.ends_with(e))
        {
            out.push((rel.to_string(), path.to_path_buf()));
        }
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        text_files(&entry.path(), &format!("{rel}/{name}"), out);
    }
}

/// `(file:line, text)` of every line in shipped text that names a stale
/// memory graph path.
fn stale_lines() -> Vec<String> {
    let mut files = Vec::new();
    for rel in SCANNED {
        text_files(&root().join(rel), rel, &mut files);
    }
    let mut bad = Vec::new();
    for (rel, path) in files {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            let root_db = line.contains("config/playbook/memory.db");
            let old_file =
                line.contains("memory.graph.json") && !OLD_FILE_HISTORY.contains(&rel.as_str());
            if root_db || old_file {
                bad.push(format!("{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    bad
}

#[test]
fn shipped_text_names_the_real_memory_graph_path() {
    let bad = stale_lines();
    assert!(
        bad.is_empty(),
        "the memory graph lives in ~/.config/playbook/data/memory.db; inspect it with \
         `playbook memory stats`. Fix these lines (or add a history-only doc to \
         OLD_FILE_HISTORY):\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_scan_reaches_the_shipped_commands_and_the_prompt() {
    let mut files = Vec::new();
    for rel in SCANNED {
        text_files(&root().join(rel), rel, &mut files);
    }
    let names: Vec<&str> = files.iter().map(|(rel, _)| rel.as_str()).collect();
    assert!(names.contains(&"plugins/playbook/commands/learn-project.md"));
    assert!(names.iter().any(|n| n.ends_with("SYSTEM_PROMPT.md")));
    assert!(!names.iter().any(|n| n.starts_with("docs/adr/")));
}
