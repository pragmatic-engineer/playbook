// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The install and repair command is `playbook setup` (ADR-0040). Text the
//! binaries and the installer print must name it. Two kinds of line may
//! still say `playbook init`: the deprecation line itself, and a note for a
//! reader who may run an older binary, written `older versions: playbook init`.

use std::fs;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("PLAYBOOK_REPO_ROOT"))
}

/// Every `.rs` file under `dir`.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// A line that may name the old command.
fn allowed(line: &str) -> bool {
    line.contains("older versions: `playbook init")
        || line.contains("older versions: playbook init")
        || line.contains("INIT_DEPRECATION")
        || line.contains("\"playbook init is deprecated, use: playbook setup\"")
        || line.contains("/// The line `playbook init` prints")
}

/// `file:line: text` for each line that names the old command or uses the
/// old `init:` message prefix. Unit tests at the end of a file are skipped,
/// since they hold sample data, and so are lines that start with `comment`.
fn offenders(path: &Path, text: &str, comment: Option<&str>) -> Vec<String> {
    let body = text.split("#[cfg(test)]").next().unwrap_or("");
    body.lines()
        .enumerate()
        .filter(|(_, l)| comment.is_none_or(|c| !l.trim_start().starts_with(c)))
        .filter(|(_, l)| l.contains("playbook init") || l.contains("\"init: "))
        .filter(|(_, l)| !allowed(l))
        .map(|(i, l)| format!("{}:{}: {}", path.display(), i + 1, l.trim()))
        .collect()
}

#[test]
fn no_binary_or_installer_text_names_playbook_init() {
    let root = repo();
    let mut files = Vec::new();
    for krate in ["cli", "common", "data", "telemetry", "code"] {
        rust_files(&root.join("crates").join(krate).join("src"), &mut files);
    }
    let mut bad = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).unwrap_or_default();
        bad.extend(offenders(file.strip_prefix(&root).unwrap(), &text, None));
    }
    let install = fs::read_to_string(root.join("install.sh")).expect("install.sh");
    bad.extend(offenders(Path::new("install.sh"), &install, Some("#")));
    assert!(
        bad.is_empty(),
        "these lines still name `playbook init`; say `playbook setup`, or add \
         `(older versions: playbook init ...)` where an older binary may read it:\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_check_catches_the_old_name() {
    let sample = "fn f() {\n    println!(\"run playbook init --yes\");\n}\n";
    assert_eq!(offenders(Path::new("x.rs"), sample, None).len(), 1);
    let note = "    hint(\"run `playbook setup` (older versions: `playbook init`)\");\n";
    assert!(offenders(Path::new("x.rs"), note, None).is_empty());
    let prefix = "    eprintln!(\"init: {err}\");\n";
    assert_eq!(offenders(Path::new("x.rs"), prefix, None).len(), 1);
}
