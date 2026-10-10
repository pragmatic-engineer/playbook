// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The marketplace plugin archive (`playbook-plugin-<version>.zip`) is built
//! from `.claude-plugin/archive-files.txt`. These tests build it exactly the
//! way `.github/workflows/release.yml` does (`git archive` of a tree, limited
//! to the allowlist pathspecs) and prove the trimmed copy still works.
//!
//! The archive is cut from the git index (`git write-tree`), so a new file
//! must be staged before it counts, the same way it must be committed before
//! a release tag does.

use regex::Regex;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

const SIZE_BUDGET_BYTES: u64 = 1024 * 1024;
const FORBIDDEN_PREFIXES: [&str; 4] = ["src/", "tests/", "docs/", ".github/"];

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "playbook-archive-{}-{tag}",
        playbook::testing::run_id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn git(args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo())
        .output()
        .expect("git should spawn");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8")
}

/// The tree of the current index, written from a private copy of the index
/// file. Run by several test processes at once (nextest starts one per test),
/// a plain `git write-tree` fights over the repository's own `index.lock`.
fn write_tree_from_index_copy(dir: &std::path::Path) -> String {
    let index = git(&["rev-parse", "--path-format=absolute", "--git-path", "index"]);
    let copy = dir.join("index-copy");
    fs::copy(index.trim(), &copy).expect("the index should be copyable");
    let out = Command::new("git")
        .args(["write-tree"])
        .env("GIT_INDEX_FILE", &copy)
        .current_dir(repo())
        .output()
        .expect("git should spawn");
    assert!(
        out.status.success(),
        "git write-tree failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf8")
        .trim()
        .to_string()
}

/// Pathspecs from the allowlist file: comments and blank lines dropped.
fn pathspecs() -> Vec<String> {
    let text = fs::read_to_string(repo().join(".claude-plugin/archive-files.txt"))
        .expect("the archive allowlist should exist");
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

fn include_specs() -> Vec<String> {
    pathspecs()
        .into_iter()
        .filter(|s| !s.starts_with(":("))
        .collect()
}

struct Archive {
    zip: PathBuf,
    root: PathBuf,
    files: BTreeSet<String>,
}

/// Built once: the same `git archive` call the release job runs.
fn archive() -> &'static Archive {
    static ARCHIVE: OnceLock<Archive> = OnceLock::new();
    ARCHIVE.get_or_init(|| {
        let dir = scratch("build");
        let zip = dir.join("playbook-plugin-test.zip");
        let tree = write_tree_from_index_copy(&dir);
        let mut args = vec![
            "archive".to_string(),
            "--format=zip".to_string(),
            format!("--output={}", zip.display()),
            tree,
            "--".to_string(),
        ];
        args.extend(pathspecs());
        let out = Command::new("git")
            .args(&args)
            .current_dir(repo())
            .output()
            .expect("git archive should spawn");
        assert!(
            out.status.success(),
            "git archive failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        let root = dir.join("unpacked");
        fs::create_dir_all(&root).expect("unpack dir");
        let out = Command::new("unzip")
            .arg("-q")
            .arg(&zip)
            .arg("-d")
            .arg(&root)
            .output()
            .expect("unzip should spawn");
        assert!(
            out.status.success(),
            "unzip failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut files = BTreeSet::new();
        collect(&root, &root, &mut files);
        Archive { zip, root, files }
    })
}

fn collect(base: &Path, dir: &Path, out: &mut BTreeSet<String>) {
    for entry in fs::read_dir(dir).expect("read_dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(base, &path, out);
        } else {
            let rel = path.strip_prefix(base).expect("under base");
            out.insert(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn tracked(prefixes: &[&str]) -> Vec<String> {
    let mut args = vec!["ls-files", "--"];
    args.extend(prefixes);
    git(&args).lines().map(String::from).collect()
}

#[test]
fn archive_excludes_source_tests_docs_ci_and_cargo_files() {
    // Arrange
    let a = archive();

    // Act
    let bad: Vec<&String> = a
        .files
        .iter()
        .filter(|f| {
            FORBIDDEN_PREFIXES.iter().any(|p| f.starts_with(p))
                || f.starts_with("Cargo.")
                || f.ends_with(".test.sh")
        })
        .collect();

    // Assert
    assert!(bad.is_empty(), "the archive must not ship: {bad:?}");
}

#[test]
fn archive_contains_every_file_the_installed_plugin_reads_at_runtime() {
    // Arrange
    let a = archive();
    // Evidence: src/init/{run,system_prompt,shim}.rs, src/hooks/session_init.rs,
    // src/common/config_hash.rs, hooks/hooks.json, and the commands that run scripts.
    let mut needed: BTreeSet<String> = [
        ".claude-plugin/plugin.json",
        "settings.shared.json",
        "prompts/SYSTEM_PROMPT.md",
        "output-styles/concise-direct.md",
        "hooks/hooks.json",
        "Brewfile",
        "LICENSE",
        "README.md",
    ]
    .iter()
    .map(std::string::ToString::to_string)
    .collect();
    // Every literal ${CLAUDE_PLUGIN_ROOT}/<file> a command, skill, agent or hook names.
    let re = Regex::new(r"CLAUDE_PLUGIN_ROOT(?::-)?\}?/([A-Za-z0-9_./-]+)").expect("regex");
    for f in tracked(&["commands", "skills", "agents", "hooks/hooks.json"]) {
        let text = fs::read_to_string(repo().join(&f)).unwrap_or_default();
        for cap in re.captures_iter(&text) {
            let p = cap[1].trim_end_matches(['.', '/']).to_string();
            let leaf = p.rsplit('/').next().unwrap_or("");
            if leaf.contains('.') && !p.contains('<') {
                needed.insert(p);
            }
        }
    }

    // Act
    let missing: Vec<&String> = needed.iter().filter(|n| !a.files.contains(*n)).collect();

    // Assert
    assert!(
        missing.is_empty(),
        "read through the plugin root but missing from the archive, add them to .claude-plugin/archive-files.txt: {missing:?}"
    );
}

#[test]
fn archive_is_under_the_size_budget() {
    // Arrange
    let a = archive();

    // Act
    let size = fs::metadata(&a.zip).expect("zip metadata").len();

    // Assert
    assert!(
        size < SIZE_BUDGET_BYTES,
        "archive is {size} bytes, budget {SIZE_BUDGET_BYTES}"
    );
}

#[test]
fn allowlist_entries_all_exist_and_are_tracked() {
    // Arrange
    let specs = include_specs();

    // Act
    let dead: Vec<&String> = specs
        .iter()
        .filter(|s| tracked(&[s.as_str()]).is_empty())
        .collect();

    // Assert
    assert!(
        dead.is_empty(),
        "allowlist entries that match no tracked file: {dead:?}"
    );
}

#[test]
fn every_tracked_plugin_content_file_is_covered_by_the_allowlist() {
    // Arrange
    let a = archive();
    let plugin_dirs = [
        "commands",
        "skills",
        "agents",
        "output-styles",
        "prompts",
        "hooks",
    ];

    // Act
    let dropped: Vec<String> = tracked(&plugin_dirs)
        .into_iter()
        .filter(|f| !f.ends_with(".test.sh") && !a.files.contains(f))
        .collect();

    // Assert
    assert!(
        dropped.is_empty(),
        "tracked plugin files that would be silently dropped from the marketplace install, add them to .claude-plugin/archive-files.txt: {dropped:?}"
    );
}

#[test]
fn init_from_the_unpacked_archive_places_prompt_and_settings() {
    // Arrange
    let a = archive();
    let home = scratch("home");

    // Act
    let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["init", "--system-prompt", "--aliases"])
        .current_dir(&home)
        .env("HOME", &home)
        .env("CLAUDE_PLUGIN_ROOT", &a.root)
        .env("SHELL", "/bin/bash")
        .env_remove("CLAUDE_CONFIG_DIR")
        .output()
        .expect("playbook should spawn");

    // Assert
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cfg = home.join(".config/playbook");
    let same = |dst: PathBuf, rel: &str| {
        assert_eq!(
            fs::read(&dst).unwrap_or_else(|e| panic!("{} missing: {e}\n{stdout}", dst.display())),
            fs::read(a.root.join(rel)).expect("archive file"),
            "{rel} was not placed from the archive"
        );
    };
    same(
        cfg.join("prompts/SYSTEM_PROMPT.md"),
        "prompts/SYSTEM_PROMPT.md",
    );
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(home.join(".claude/settings.json")).expect("settings.json placed"),
    )
    .expect("settings.json parses");
    let template: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(a.root.join("settings.shared.json")).expect("template"),
    )
    .expect("template parses");
    for key in template.as_object().expect("template object").keys() {
        if key == "permissions" {
            // The security defaults are opt-in, a plain init leaves them out.
            continue;
        }
        assert!(
            settings.get(key).is_some(),
            "settings.json lacks the template key {key}\n{stdout}"
        );
    }
}

#[test]
fn claude_plugin_validate_accepts_the_unpacked_archive() {
    // Arrange
    let a = archive();
    let Ok(out) = Command::new("claude")
        .args(["plugin", "validate"])
        .arg(&a.root)
        .output()
    else {
        // CI installs the CLI, so a missing one there means the check would
        // pass without running. Only a local run may skip.
        assert!(
            std::env::var_os("CI").is_none(),
            "claude CLI is missing in CI, so `claude plugin validate` did not run"
        );
        eprintln!("claude CLI not available, skipping plugin validate");
        return;
    };

    // Act
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // Assert
    assert!(
        out.status.success(),
        "claude plugin validate failed:\n{text}"
    );
}

/// The part of a source file before its `#[cfg(test)]` module.
fn non_test_part(text: &str) -> &str {
    text.find("#[cfg(test)]").map_or(text, |i| &text[..i])
}

#[test]
fn every_plugin_root_read_in_src_is_in_the_archive() {
    // Arrange: `self_root.join("a").join("b")` and `plugin_root.join("a/b")`.
    let a = archive();
    let chain = Regex::new(
        r#"\b(?:self_root|plugin_root)\s*\.join\("([^"]+)"\)((?:\s*\.join\("[^"]+"\))*)"#,
    )
    .expect("regex");
    let part = Regex::new(r#"\.join\("([^"]+)"\)"#).expect("regex");
    let mut found: BTreeSet<String> = BTreeSet::new();
    for f in tracked(&["src"]) {
        if !f.ends_with(".rs") {
            continue;
        }
        let text = fs::read_to_string(repo().join(&f)).unwrap_or_default();
        for cap in chain.captures_iter(non_test_part(&text)) {
            let mut path = cap[1].to_string();
            for p in part.captures_iter(&cap[2]) {
                path = format!("{path}/{}", &p[1]);
            }
            found.insert(path);
        }
    }

    // Act: a path counts when it is a shipped file or a directory holding one.
    // `.git` is only probed, never read.
    let missing: Vec<&String> = found
        .iter()
        .filter(|p| p.as_str() != ".git")
        .filter(|p| {
            !a.files.contains(*p) && !a.files.iter().any(|f| f.starts_with(&format!("{p}/")))
        })
        .collect();

    // Assert: the scan itself must still see the reads it exists to guard.
    for known in ["settings.shared.json", "prompts/SYSTEM_PROMPT.md"] {
        assert!(
            found.contains(known),
            "the scan no longer finds the read of {known}: {found:?}"
        );
    }
    assert!(
        missing.is_empty(),
        "src/ reads these through the plugin root but the archive lacks them, add them to .claude-plugin/archive-files.txt: {missing:?}"
    );
}
