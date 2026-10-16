// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The install and repair command is `playbook setup` (ADR-0040). Text the
//! binaries and the installer print must name it. Two kinds of line may
//! still say `playbook init`: the deprecation line itself, and a note for a
//! reader who may run an older binary, written `older versions: playbook init`.
//! Only the telemetry crate keeps that note, since `playbook-telemetry` and
//! `pbtop` can run next to an older `playbook`; there every `playbook setup`
//! hint must carry it. `crates/plugin-checks/tests/setup_command_name.rs`
//! applies the same note rule to the plugin and the docs; keep the two in
//! step.

use std::fs;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("PLAYBOOK_REPO_ROOT"))
}

/// Every `.rs` file under `dir`.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The old-name note from ADR-0040 decision 4.
fn has_note(line: &str) -> bool {
    line.contains("older versions: `playbook init")
        || line.contains("older versions: playbook init")
}

/// The deprecation line itself.
fn is_deprecation(line: &str) -> bool {
    line.contains("\"playbook init is deprecated, use: playbook setup\"")
}

/// The text before a file's unit test module. A `#[cfg(test)]` on a single
/// item higher up does not end the scan, only one that leads, past any other
/// attributes, to `mod tests`.
fn production_part(text: &str) -> &str {
    let mut at = 0;
    let mut lines = text.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        if line.trim() == "#[cfg(test)]" {
            let module = lines
                .clone()
                .map(str::trim_start)
                .find(|l| !l.starts_with("#["))
                .is_some_and(|l| l.starts_with("mod tests"));
            if module {
                return &text[..at];
            }
        }
        at += line.len();
    }
    text
}

/// A shell comment, which the installer never prints.
fn shell_comment(line: &str) -> bool {
    line.trim_start().starts_with('#')
}

/// A Rust comment a user never sees. `///` lines stay, since clap shows them
/// as help.
fn private_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") && !t.starts_with("///")
}

/// Where a line is read, which decides the note rule.
#[derive(Clone, Copy)]
enum Rule {
    /// Text of a binary that has `setup`: the note is not allowed.
    NoNote,
    /// Text that may run next to an older binary: the note is required on
    /// every `playbook setup` hint.
    Note,
}

/// Whether one line breaks `rule`.
fn bad_line(line: &str, rule: Rule) -> bool {
    let old = line.contains("playbook init") || line.contains("\"init: ");
    match rule {
        Rule::NoNote => (old && !is_deprecation(line)) || has_note(line),
        Rule::Note => (old || line.contains("playbook setup")) && !has_note(line),
    }
}

/// `file:line: text` for each line that breaks `rule`. The unit test module
/// is skipped, since it holds sample data, and so are lines that `skip`
/// matches.
fn offenders(path: &Path, text: &str, skip: fn(&str) -> bool, rule: Rule) -> Vec<String> {
    production_part(text)
        .lines()
        .enumerate()
        .filter(|(_, l)| !skip(l) && bad_line(l, rule))
        .map(|(i, l)| format!("{}:{}: {}", path.display(), i + 1, l.trim()))
        .collect()
}

#[test]
fn no_binary_or_installer_text_names_playbook_init() {
    let root = repo();
    let mut files = Vec::new();
    for krate in ["cli", "common", "data", "telemetry", "code"] {
        let before = files.len();
        rust_files(&root.join("crates").join(krate).join("src"), &mut files);
        assert!(files.len() > before, "no Rust files in crates/{krate}/src");
    }
    let mut bad = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("read a source file");
        let path = file.strip_prefix(&root).unwrap();
        let rule = if path.starts_with("crates/telemetry") {
            Rule::Note
        } else {
            Rule::NoNote
        };
        bad.extend(offenders(path, &text, private_comment, rule));
    }
    // The installer picks the name the installed binary knows and prints it.
    let install = fs::read_to_string(root.join("install.sh")).expect("install.sh");
    bad.extend(offenders(
        Path::new("install.sh"),
        &install,
        shell_comment,
        Rule::NoNote,
    ));
    assert!(
        bad.is_empty(),
        "these lines still name `playbook init`; say `playbook setup`, or add \
         `(older versions: playbook init ...)` where an older binary may read it:\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_check_catches_the_old_name() {
    let x = Path::new("x.rs");
    let cli = |text: &str| offenders(x, text, private_comment, Rule::NoNote).len();
    assert_eq!(
        cli("fn f() {\n    println!(\"run playbook init --yes\");\n}\n"),
        1
    );
    assert_eq!(cli("    eprintln!(\"init: {err}\");\n"), 1);
    assert_eq!(
        cli("pub const L: &str = \"playbook init is deprecated, use: playbook setup\";\n"),
        0
    );
    // A binary that has `setup` gives no old-name note.
    assert_eq!(
        cli("    hint(\"run `playbook setup` (older versions: `playbook init`)\");\n"),
        1
    );
    assert_eq!(
        cli("#[cfg(test)]\nfn helper() {}\n\nfn f() { hint(\"playbook init\") }\n"),
        1
    );
    assert_eq!(
        cli("fn f() {}\n#[cfg(test)]\nmod tests {\n    const S: &str = \"playbook init\";\n}\n"),
        0
    );
    assert_eq!(cli("fn f() {}\r\n#[cfg(test)]\r\n#[rustfmt::skip]\r\nmod tests {\r\n    const S: &str = \"playbook init\";\r\n}\r\n"), 0);
    assert_eq!(cli("// playbook init, the old name\n"), 0);
    assert_eq!(cli("/// Run `playbook init`\n"), 1);
}

#[test]
fn telemetry_text_keeps_the_note() {
    let x = Path::new("x.rs");
    let telemetry = |text: &str| offenders(x, text, private_comment, Rule::Note).len();
    assert_eq!(
        telemetry("    hint(\"run `playbook setup --telemetry`\");\n"),
        1
    );
    assert_eq!(telemetry("    hint(\"run `playbook init`\");\n"), 1);
    assert_eq!(
        telemetry("    hint(\"run `playbook setup` (older versions: `playbook init`)\");\n"),
        0
    );
}

#[test]
fn installer_comments_are_skipped() {
    let sh = Path::new("install.sh");
    let installer = |text: &str| offenders(sh, text, shell_comment, Rule::NoNote).len();
    assert_eq!(
        installer("# Releases before the rename only know `playbook init`\n"),
        0
    );
    assert_eq!(installer("echo \"run playbook init\"\n"), 1);
}
