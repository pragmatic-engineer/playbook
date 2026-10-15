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

/// A line that may name the old command.
fn allowed(line: &str) -> bool {
    line.contains("older versions: `playbook init")
        || line.contains("older versions: playbook init")
        || line.contains("\"playbook init is deprecated, use: playbook setup\"")
}

/// The text before a file's unit test module. A `#[cfg(test)]` on a single
/// item higher up does not end the scan, only one followed by `mod tests`.
fn production_part(text: &str) -> &str {
    let mut at = 0;
    for (i, line) in text.lines().enumerate() {
        if line.trim() == "#[cfg(test)]"
            && text
                .lines()
                .nth(i + 1)
                .is_some_and(|next| next.trim_start().starts_with("mod tests"))
        {
            return &text[..at];
        }
        at += line.len() + 1;
    }
    text
}

/// A Rust comment a user never sees. `///` lines stay, since clap shows them
/// as help.
fn private_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") && !t.starts_with("///")
}

/// `file:line: text` for each line that names the old command or uses the
/// old `init:` message prefix. The unit test module is skipped, since it
/// holds sample data, and so are lines that `skip` matches.
fn offenders(path: &Path, text: &str, skip: fn(&str) -> bool) -> Vec<String> {
    production_part(text)
        .lines()
        .enumerate()
        .filter(|(_, l)| !skip(l))
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
        let before = files.len();
        rust_files(&root.join("crates").join(krate).join("src"), &mut files);
        assert!(files.len() > before, "no Rust files in crates/{krate}/src");
    }
    let mut bad = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("read a source file");
        bad.extend(offenders(
            file.strip_prefix(&root).unwrap(),
            &text,
            private_comment,
        ));
    }
    let install = fs::read_to_string(root.join("install.sh")).expect("install.sh");
    let shell_comment = |l: &str| l.trim_start().starts_with('#');
    bad.extend(offenders(Path::new("install.sh"), &install, shell_comment));
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
    let sample = "fn f() {\n    println!(\"run playbook init --yes\");\n}\n";
    assert_eq!(offenders(x, sample, private_comment).len(), 1);
    let note = "    hint(\"run `playbook setup` (older versions: `playbook init`)\");\n";
    assert!(offenders(x, note, private_comment).is_empty());
    let prefix = "    eprintln!(\"init: {err}\");\n";
    assert_eq!(offenders(x, prefix, private_comment).len(), 1);
    let after_test_item = "#[cfg(test)]\nfn helper() {}\n\nfn f() { hint(\"playbook init\") }\n";
    assert_eq!(offenders(x, after_test_item, private_comment).len(), 1);
    let in_test_module =
        "fn f() {}\n#[cfg(test)]\nmod tests {\n    const S: &str = \"playbook init\";\n}\n";
    assert!(offenders(x, in_test_module, private_comment).is_empty());
    assert!(offenders(x, "// playbook init, the old name\n", private_comment).is_empty());
    assert_eq!(
        offenders(x, "/// Run `playbook init`\n", private_comment).len(),
        1
    );
}
