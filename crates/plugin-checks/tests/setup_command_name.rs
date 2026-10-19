// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The install and repair command is `playbook setup` (ADR-0040). The plugin
//! can run next to a binary from before the rename, so a line that names the
//! old `playbook init` must carry the note from ADR-0040 decision 4:
//! `older versions: playbook init`, with or without backticks. That is the
//! same rule `crates/cli/tests/setup_naming.rs` applies to the binary and the
//! installer; keep the two in step. The user docs also may not use a bare
//! `init` as the command name, outside that note and a few named forms.

#[path = "support/markdown.rs"]
mod markdown;

use markdown::markdown_files;
use std::fs;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("PLAYBOOK_REPO_ROOT"))
}

/// The ADR-0040 note. A line that names the old command must hold it.
fn has_the_note(line: &str) -> bool {
    line.contains("older versions: playbook init")
        || line.contains("older versions: `playbook init")
}

/// A table row that links an ADR. It quotes the title of an accepted record,
/// and those stay as written.
fn is_adr_row(line: &str) -> bool {
    line.starts_with('|') && line.contains("(adr/")
}

/// Lines that name `playbook init` without the note.
fn stale_full_name(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !is_adr_row(l))
        .filter(|(_, l)| l.contains("playbook init") && !has_the_note(l))
        .map(|(i, l)| (i + 1, l.trim().to_string()))
        .collect()
}

/// Words around `init` that make it something other than the command:
/// `shell-init`, `session-init`, `--no-init`, `src/init/`, `system/init`,
/// `init_docs.rs`, `init-to-setup`, `git init`, and the history notes.
fn bare_init_is_allowed(line: &str, start: usize) -> bool {
    let before = line[..start].chars().next_back();
    let after = line[start + "init".len()..].chars().next();
    if before.is_some_and(|c| "-/_".contains(c)) || after.is_some_and(|c| "-/_".contains(c)) {
        return true;
    }
    line[..start].ends_with("git ")
        || line[start..].starts_with("init.rs")
        || has_the_note(line)
        || line.contains("then called `init`")
        || line.contains("old name `init`")
}

/// Lines that use a bare `init` as the command name.
fn stale_bare_name(text: &str) -> Vec<(usize, String)> {
    let word = regex::Regex::new(r"\binit\b").expect("valid regex");
    text.lines()
        .enumerate()
        .filter(|(_, l)| !is_adr_row(l))
        .filter(|(_, l)| {
            word.find_iter(l)
                .any(|m| !bare_init_is_allowed(l, m.start()))
        })
        .map(|(i, l)| (i + 1, l.trim().to_string()))
        .collect()
}

fn report(files: &[PathBuf], check: fn(&str) -> Vec<(usize, String)>) -> Vec<String> {
    let mut bad = Vec::new();
    for f in files {
        let text = fs::read_to_string(f).expect("read");
        for (n, line) in check(&text) {
            bad.push(format!("{}:{n}: {line}", f.display()));
        }
    }
    bad
}

fn user_docs() -> Vec<PathBuf> {
    let root = repo();
    let mut files = vec![
        root.join("README.md"),
        root.join("CONTRIBUTING.md"),
        root.join("docs/README.md"),
    ];
    for dir in [
        "docs/guides",
        "docs/internals",
        "docs/reference",
        "docs/authoring",
    ] {
        markdown_files(&root.join(dir), &mut files);
    }
    files
}

#[test]
fn plugin_text_names_init_only_with_the_note() {
    let mut files = Vec::new();
    markdown_files(Path::new(env!("PLAYBOOK_PLUGIN_ROOT")), &mut files);
    assert!(!files.is_empty(), "no plugin markdown found");
    let bad = report(&files, stale_full_name);
    assert!(
        bad.is_empty(),
        "say `playbook setup (older versions: playbook init)`:\n{}",
        bad.join("\n")
    );
}

#[test]
fn user_docs_name_setup() {
    let files = user_docs();
    let mut bad = report(&files, stale_full_name);
    bad.extend(report(&files, stale_bare_name));
    assert!(
        bad.is_empty(),
        "these docs still name `init` as the command; say `setup`:\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_full_name_check_wants_the_adr_note() {
    assert_eq!(stale_full_name("Fix: `playbook init`.").len(), 1);
    assert_eq!(
        stale_full_name("`playbook init` also works on older versions of Claude Code").len(),
        1
    );
    assert!(stale_full_name("Fix: `playbook setup` (older versions: `playbook init`).").is_empty());
    assert!(stale_full_name("run playbook setup (older versions: playbook init)").is_empty());
    assert!(stale_full_name("| [0016](adr/0016-x.md) | Run from `playbook init` |").is_empty());
}

#[test]
fn the_bare_name_check_skips_other_inits() {
    assert_eq!(stale_bare_name("Restart Claude Code after init.").len(), 1);
    assert_eq!(stale_bare_name("Run `init --yes` first.").len(), 1);
    for fine in [
        "the `session-init` hook and `playbook shell-init`",
        "install.sh --no-init",
        "lives in `crates/cli/src/init/migrate.rs`",
        "see `system/init` in stream-json",
        "run `git init` in a scratch folder",
        "the command (then called `init`) printed a plan",
        "`setup` or its old name `init`",
        "| [0016](adr/0016-migration-framework.md) | Migrations run from `playbook init` |",
        "[What setup installs](05-what-setup-installs.md)",
    ] {
        assert!(stale_bare_name(fine).is_empty(), "{fine}");
    }
}
