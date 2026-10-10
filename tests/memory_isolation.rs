// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Playbook memory and Claude Code memory never mix. Playbook's memory code
//! writes only under `~/.config/playbook/memory`, and nothing playbook ships
//! sets or edits Claude Code's own memory settings or directories. Reading
//! Claude Code's auto memory is allowed, only from `memory_import/claude_read.rs`.

use std::fs;
use std::path::{Path, PathBuf};

fn files_under(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, exts, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e))
        {
            out.push(path);
        }
    }
}

fn shipped_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for dir in ["src", "commands", "skills", "agents", "prompts", "hooks"] {
        files_under(&root.join(dir), &["rs", "md", "json", "sh"], &mut out);
    }
    out.push(root.join("settings.shared.json"));
    out
}

/// The production code of a Rust file: everything before `#[cfg(test)]`,
/// without comment lines.
fn production(text: &str) -> String {
    text.split("#[cfg(test)]")
        .next()
        .unwrap_or(text)
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn nothing_shipped_sets_claude_codes_memory_settings() {
    for file in shipped_files() {
        let text = fs::read_to_string(&file).unwrap_or_default();
        for banned in ["autoMemoryEnabled", "autoMemoryDirectory"] {
            assert!(
                !text.contains(banned),
                "{} names {banned}: playbook must never set Claude Code's memory settings",
                file.display()
            );
        }
    }
}

#[test]
fn no_rust_code_builds_a_path_into_claude_codes_project_memory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    files_under(&root, &["rs"], &mut files);
    for file in files {
        // The one exception: `policy-guard` names those directories only to
        // deny writes into them, and never touches them.
        if file.ends_with("hooks/policy_guard.rs") {
            continue;
        }
        let text = fs::read_to_string(&file).unwrap_or_default();
        let code = production(&text);
        let builds_projects = code.contains("\"projects\"") || code.contains("/projects/");
        let names_memory_dir = code.contains("\"memory\"") || code.contains("/memory\"");
        assert!(
            !(builds_projects && names_memory_dir),
            "{} builds a path under ~/.claude/projects/<project>/memory, which is Claude Code's auto memory",
            file.display()
        );
    }
}

#[test]
fn memory_hooks_never_write_under_claude_home() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hooks");
    for name in [
        "rebuild_memory_graph.rs",
        "session_init.rs",
        "memory_context.rs",
    ] {
        let Ok(text) = fs::read_to_string(root.join(name)) else {
            continue;
        };
        let code = production(&text);
        assert!(
            !code.contains("claude_dir()") && !code.contains(".claude/memory"),
            "{name} must resolve memory through the playbook root, not Claude Code's directory"
        );
    }
}

#[test]
fn the_claude_memory_reader_only_reads() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/memory_import/claude_read.rs");
    let text = fs::read_to_string(path).unwrap();
    let code = production(&text);
    for banned in [
        "fs::write",
        "File::create",
        "OpenOptions",
        "remove_file",
        "remove_dir",
        "rename",
        "fs::copy",
        "create_dir",
        "set_permissions",
        "set_modified",
        "write_atomic",
    ] {
        assert!(
            !code.contains(banned),
            "claude_read.rs must only read Claude Code's memory, found {banned}"
        );
    }
}

#[test]
fn policy_guard_only_denies_and_never_writes() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/hooks/policy_guard.rs");
    let code = production(&fs::read_to_string(path).unwrap());
    for banned in [
        "fs::write",
        "File::create",
        "OpenOptions",
        "create_dir",
        "remove_file",
        "remove_dir",
        "fs::rename",
        "fs::copy",
    ] {
        assert!(
            !code.contains(banned),
            "policy_guard.rs uses {banned}: it must only read and deny"
        );
    }
}
