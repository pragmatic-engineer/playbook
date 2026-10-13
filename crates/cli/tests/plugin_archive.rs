// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The marketplace ships the hooks plugin as `playbook-hooks-plugin-<version>.zip`
//! (hooks, shim, installer, seed files), built from
//! `.claude-plugin/archive-hooks-files.txt`. The skills plugin lives in
//! pragmatic-engineer/skills. These tests build the hooks archive with the same
//! script `.github/workflows/release.yml` runs and prove the trimmed copy works.
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
const FORBIDDEN_PREFIXES: [&str; 3] = ["crates/", "docs/", ".github/"];

fn repo() -> PathBuf {
    PathBuf::from(env!("PLAYBOOK_REPO_ROOT"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "playbook-archive-{}-{}-{tag}",
        playbook::testing::run_id(),
        // nextest runs each test in its own process, so a shared name races.
        std::process::id()
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

#[derive(Clone, Copy)]
enum Kind {
    Hooks,
}

impl Kind {
    fn list(self) -> &'static str {
        match self {
            Kind::Hooks => ".claude-plugin/archive-hooks-files.txt",
        }
    }
    fn tag(self) -> &'static str {
        match self {
            Kind::Hooks => "hooks",
        }
    }
}

/// Pathspecs from an allowlist file: comments and blank lines dropped.
fn pathspecs(kind: Kind) -> Vec<String> {
    let text =
        fs::read_to_string(repo().join(kind.list())).expect("the archive allowlist should exist");
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

/// A throwaway commit of the current index with a fixed date, so the build
/// script stamps entries from a commit time exactly as it does in the release.
fn commit_from_index_copy(dir: &Path) -> String {
    let tree = write_tree_from_index_copy(dir);
    let out = Command::new("git")
        .args([
            "-c",
            "commit.gpgsign=false",
            "commit-tree",
            &tree,
            "-m",
            "archive test",
        ])
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_AUTHOR_DATE", "2026-01-02T03:04:05Z")
        .env("GIT_COMMITTER_DATE", "2026-01-02T03:04:05Z")
        .current_dir(repo())
        .output()
        .expect("git should spawn");
    assert!(
        out.status.success(),
        "git commit-tree failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("utf8")
        .trim()
        .to_string()
}

struct Archive {
    zip: PathBuf,
    root: PathBuf,
    files: BTreeSet<String>,
}

fn hooks() -> &'static Archive {
    static A: OnceLock<Archive> = OnceLock::new();
    A.get_or_init(|| build(Kind::Hooks))
}

/// Built once per kind: the same script call the release job runs.
fn build(kind: Kind) -> Archive {
    let dir = scratch(&format!("build-{}", kind.tag()));
    let zip = dir.join(format!("playbook-{}-plugin-test.zip", kind.tag()));
    let tree = commit_from_index_copy(&dir);
    let out = Command::new("bash")
        .arg(repo().join(".github/scripts/build-plugin-archive.sh"))
        .arg(&tree)
        .arg(repo().join(kind.list()))
        .arg(&zip)
        .current_dir(repo())
        .output()
        .expect("the build script should spawn");
    assert!(
        out.status.success(),
        "build-plugin-archive.sh failed: {}",
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

fn both() -> [(&'static str, &'static Archive); 1] {
    [("hooks", hooks())]
}

fn manifest(a: &Archive) -> serde_json::Value {
    serde_json::from_str(
        &fs::read_to_string(a.root.join(".claude-plugin/plugin.json"))
            .expect("the archive should hold a manifest"),
    )
    .expect("the manifest should parse")
}

#[test]
fn archives_exclude_source_tests_docs_ci_and_cargo_files() {
    for (name, a) in both() {
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
        assert!(bad.is_empty(), "the {name} archive must not ship: {bad:?}");
    }
}

#[test]
fn the_same_commit_builds_the_same_bytes() {
    // Arrange
    let dir = scratch("determinism");
    let commit = commit_from_index_copy(&dir);
    let build_once = |name: &str| {
        let zip = dir.join(name);
        let out = Command::new("bash")
            .arg(repo().join(".github/scripts/build-plugin-archive.sh"))
            .arg(&commit)
            .arg(repo().join(Kind::Hooks.list()))
            .arg(&zip)
            .current_dir(repo())
            .output()
            .expect("the build script should spawn");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        fs::read(zip).expect("zip")
    };

    // Act
    let first = build_once("a.zip");
    let second = build_once("b.zip");

    // Assert
    assert_eq!(first, second, "the same commit must give the same bytes");
}

#[test]
fn the_hooks_manifest_is_named_playbook_hooks() {
    let h = manifest(hooks());
    assert_eq!(h["name"], "playbook-hooks");
    assert!(
        !hooks().files.contains(".claude-plugin/hooks.plugin.json"),
        "the hooks manifest is renamed to plugin.json inside the zip"
    );
}

#[test]
fn archive_contains_every_file_the_installed_plugin_reads_at_runtime() {
    // Arrange
    // Evidence: src/init/{run,system_prompt,shim}.rs, src/hooks/session_init.rs,
    // src/common/config_hash.rs and hooks/hooks.json.
    let hooks_needed = [
        ".claude-plugin/plugin.json",
        "settings.shared.json",
        "prompts/SYSTEM_PROMPT.md",
        "hooks/hooks.json",
        "bin/playbook",
        "install.sh",
        "Brewfile",
        "LICENSE",
        "README.md",
    ];
    let mut by_name: Vec<(&str, &Archive, BTreeSet<String>)> = vec![(
        "hooks",
        hooks(),
        hooks_needed.iter().map(|s| (*s).to_string()).collect(),
    )];
    // Every literal ${CLAUDE_PLUGIN_ROOT}/<file> a hook names is read from the hooks plugin.
    let re = Regex::new(r"CLAUDE_PLUGIN_ROOT(?::-)?\}?/([A-Za-z0-9_./-]+)").expect("regex");
    for f in tracked(&["hooks/hooks.json"]) {
        let text = fs::read_to_string(repo().join(&f)).unwrap_or_default();
        for cap in re.captures_iter(&text) {
            let p = cap[1].trim_end_matches(['.', '/']).to_string();
            let leaf = p.rsplit('/').next().unwrap_or("");
            if leaf.contains('.') && !p.contains('<') {
                by_name[0].2.insert(p);
            }
        }
    }

    for (name, a, needed) in by_name {
        // Act
        let missing: Vec<&String> = needed.iter().filter(|n| !a.files.contains(*n)).collect();

        // Assert
        assert!(
            missing.is_empty(),
            "read through the {name} plugin root but missing from its archive, add them to its allowlist: {missing:?}"
        );
    }
}

#[test]
fn archives_are_under_the_size_budget() {
    for (name, a) in both() {
        // Act
        let size = fs::metadata(&a.zip).expect("zip metadata").len();

        // Assert
        assert!(
            size < SIZE_BUDGET_BYTES,
            "the {name} archive is {size} bytes, budget {SIZE_BUDGET_BYTES}"
        );
    }
}

#[test]
fn allowlist_entries_all_exist_and_are_tracked() {
    // Act
    let dead: Vec<String> = pathspecs(Kind::Hooks)
        .into_iter()
        .filter(|s| tracked(&[s.as_str()]).is_empty())
        .collect();

    // Assert
    assert!(
        dead.is_empty(),
        "{} entries that match no tracked file: {dead:?}",
        Kind::Hooks.list()
    );
}

#[test]
fn every_tracked_plugin_content_file_is_in_an_archive() {
    // Arrange
    let plugin_dirs = ["prompts", "hooks", "bin"];

    // Act
    let dropped: Vec<String> = tracked(&plugin_dirs)
        .into_iter()
        .filter(|f| !f.ends_with(".test.sh") && !hooks().files.contains(f))
        .collect();

    // Assert
    assert!(
        dropped.is_empty(),
        "tracked plugin files that no marketplace install would get, add them to one allowlist in .claude-plugin/: {dropped:?}"
    );
}

#[test]
fn init_from_the_unpacked_hooks_archive_places_prompt_and_settings() {
    // Arrange
    let a = hooks();
    let home = scratch("home");

    // Act
    let out = run_init(&home, &a.root);

    // Assert
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cfg = home.join(".config/playbook");
    assert_eq!(
        fs::read(cfg.join("prompts/SYSTEM_PROMPT.md"))
            .unwrap_or_else(|e| panic!("system prompt missing: {e}\n{stdout}")),
        fs::read(a.root.join("prompts/SYSTEM_PROMPT.md")).expect("archive file"),
        "the system prompt was not placed from the archive"
    );
    assert_settings_seeded(&home, &a.root, &stdout);
}

fn run_init(home: &Path, plugin_root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_playbook"))
        .args(["init", "--system-prompt", "--aliases"])
        .current_dir(home)
        .env("HOME", home)
        .env("CLAUDE_PLUGIN_ROOT", plugin_root)
        .env("SHELL", "/bin/bash")
        .env_remove("CLAUDE_CONFIG_DIR")
        .output()
        .expect("playbook should spawn")
}

fn assert_settings_seeded(home: &Path, template_root: &Path, stdout: &str) {
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(home.join(".claude/settings.json")).expect("settings.json placed"),
    )
    .expect("settings.json parses");
    let template: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(template_root.join("settings.shared.json")).expect("template"),
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
fn claude_plugin_validate_accepts_the_unpacked_archives() {
    for (name, a) in both() {
        // Arrange
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
            "claude plugin validate failed for the {name} archive:\n{text}"
        );
    }
}

/// The part of a source file before its `#[cfg(test)]` module.
fn non_test_part(text: &str) -> &str {
    text.find("#[cfg(test)]").map_or(text, |i| &text[..i])
}

#[test]
fn every_plugin_root_read_in_src_is_in_an_archive() {
    // Arrange: `self_root.join("a").join("b")`.
    let part = Regex::new(r#"\.join\("([^"]+)"\)"#).expect("regex");
    let chain = Regex::new(r#"\bself_root\s*\.join\("([^"]+)"\)((?:\s*\.join\("[^"]+"\))*)"#)
        .expect("regex");
    // `self_root` reads the seed files, which only the hooks plugin ships (or
    // the binary embeds).
    let mut found: BTreeSet<String> = BTreeSet::new();
    for f in tracked(&["crates/cli/src"]) {
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
    let shipped = hooks();
    let missing: Vec<&String> = found
        .iter()
        .filter(|p| p.as_str() != ".git")
        .filter(|p| {
            !shipped.files.contains(*p)
                && !shipped
                    .files
                    .iter()
                    .any(|f| f.starts_with(&format!("{p}/")))
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
        "src/ reads these through the plugin root but its archive lacks them, add them to the allowlist in .claude-plugin/: {missing:?}"
    );
}
