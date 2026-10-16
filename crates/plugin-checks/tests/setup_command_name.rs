// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The install and repair command is `playbook setup` (ADR-0040). The plugin
//! can run next to a binary from before the rename, so a line that names the
//! old `playbook init` must also say it is the old name: `older versions`,
//! `Older binaries` or `predates the rename`. The user docs name only
//! `playbook setup`, apart from one note that says the old name still works.

use std::fs;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).expect("dir").flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            out.push(p);
        }
    }
}

fn names_the_old_command(line: &str) -> bool {
    line.contains("playbook init")
}

fn says_it_is_the_old_name(line: &str) -> bool {
    ["older versions", "Older binaries", "predates the rename", "Older releases"]
        .iter()
        .any(|m| line.contains(m))
}

fn offenders(files: &[PathBuf]) -> Vec<String> {
    let mut bad = Vec::new();
    for f in files {
        let text = fs::read_to_string(f).expect("read");
        for (i, line) in text.lines().enumerate() {
            if names_the_old_command(line) && !says_it_is_the_old_name(line) {
                bad.push(format!("{}:{}: {}", f.display(), i + 1, line.trim()));
            }
        }
    }
    bad
}

#[test]
fn plugin_text_names_init_only_as_the_old_name() {
    let mut files = Vec::new();
    walk(&PathBuf::from(env!("PLAYBOOK_PLUGIN_ROOT")), &mut files);
    assert!(!files.is_empty(), "no plugin markdown found");
    let bad = offenders(&files);
    assert!(
        bad.is_empty(),
        "say `playbook setup (older versions: playbook init)`:\n{}",
        bad.join("\n")
    );
}

#[test]
fn user_docs_name_setup() {
    let root = PathBuf::from(env!("PLAYBOOK_REPO_ROOT"));
    let mut files = vec![root.join("README.md"), root.join("CONTRIBUTING.md")];
    for dir in ["docs/guides", "docs/internals", "docs/concepts", "docs/reference"] {
        walk(&root.join(dir), &mut files);
    }
    let bad = offenders(&files);
    assert!(
        bad.is_empty(),
        "these docs still name `playbook init`:\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_check_tells_a_note_from_a_stale_hint() {
    assert!(names_the_old_command("Fix: `playbook init`."));
    assert!(!says_it_is_the_old_name("Fix: `playbook init`."));
    assert!(says_it_is_the_old_name(
        "Fix: `playbook setup` (older versions: `playbook init`)."
    ));
}
