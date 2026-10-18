// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The rm guard against a corpus of real commands it judged in agent sessions,
//! anonymized, plus the deletion commands it should also see.
//!
//! Each case in `fixtures/rm_guard_corpus.json` names the folder the command
//! ran in, the command, the verdict it should get (`want`), the verdict the
//! guard gives today (`expect`) and the class of the case. The test runs every
//! case through the real binary in a fixture tree and fails when a verdict
//! moves away from `expect`, so a change to the guard has to update the corpus
//! on purpose. The cases where `expect` is not yet `want` are the known gaps.
//!
//! Paths in the corpus are written under `/Users/<user>`, which the test maps
//! to a fake home in Cargo's scratch folder, where it also builds the repos and
//! worktrees the cases name. A case with an `os` runs on that OS only. Set `RM_GUARD_CORPUS_PRINT=1` to print the
//! verdict of every case.

use serde_json::Value;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

const USER: &str = "<user>";
const USER_HOME: &str = "/Users/<user>";

struct Case {
    id: String,
    cwd: String,
    command: String,
    want: String,
    expect: String,
    class: String,
    /// The one OS the case applies to, as `std::env::consts::OS` names it,
    /// or empty for every OS.
    os: String,
}

fn corpus() -> Vec<Case> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rm_guard_corpus.json");
    let text = fs::read_to_string(&path).expect("read corpus");
    let cases: Vec<Value> = serde_json::from_str(&text).expect("corpus is JSON");
    let field = |c: &Value, k: &str| c[k].as_str().unwrap_or_default().to_string();
    cases
        .iter()
        .map(|c| Case {
            id: field(c, "id"),
            cwd: field(c, "cwd"),
            command: field(c, "command"),
            want: field(c, "want"),
            expect: field(c, "expect"),
            class: field(c, "class"),
            os: field(c, "os"),
        })
        .collect()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

fn new_repo(dir: &Path) {
    fs::create_dir_all(dir).expect("repo dir");
    git(dir, &["init", "-q"]);
    git(dir, &["commit", "-q", "--allow-empty", "-m", "init"]);
}

fn add_worktree(repo: &Path, path: &Path, branch: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("worktree parent");
    git(
        repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            branch,
            path.to_str().expect("utf8"),
        ],
    );
}

/// The fake home and the repos the corpus names.
struct Tree {
    base: PathBuf,
    home: PathBuf,
}

impl Tree {
    fn new() -> Tree {
        let run = playbook::testing::run_id();
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("rmcorpus-{run}"));
        fs::create_dir_all(&base).expect("base");
        let base = fs::canonicalize(base).expect("canonical base");
        let home = base.join("home");
        let w = home.join("code");
        let pb = w.join("pragmatic-engineer/playbook");
        let wt = w.join("pragmatic-engineer/.worktrees");
        let coda = w.join("acme/coda");
        new_repo(&pb);
        add_worktree(&pb, &wt.join("rm-guard"), "rm-guard");
        add_worktree(&pb, &pb.join(".claude/worktrees/agent-a81c"), "agent-a81c");
        for dir in [
            "playbook/usage-limits",
            "playbook/usage-tui",
            "playbook/fresh-briefs",
            "playbook/ci-quick-wins",
            "ward-s4",
            "cargo-auto-budget",
        ] {
            fs::create_dir_all(wt.join(dir)).expect("worktrees folder");
        }
        new_repo(&coda);
        for dir in ["crates/coda-index/src", "crates/coda-providers", "target"] {
            fs::create_dir_all(coda.join(dir)).expect("coda folder");
        }
        fs::create_dir_all(base.join("outside")).expect("outside");
        symlink(base.join("outside"), coda.join("linkout")).expect("link out of the repo");
        for repo in [
            "acme/factory",
            "solo/ward",
            "solo/dealwatch",
            "solo/dealwatch-backup",
            "solo/lockstep",
        ] {
            new_repo(&w.join(repo));
        }
        fs::create_dir_all(w.join("acme/factory/worker")).expect("worker");
        fs::create_dir_all(w.join("solo/lockstep/hosts/godot-capsule")).expect("godot");
        fs::create_dir_all(home.join(".config/playbook")).expect("config store");
        Tree { base, home }
    }

    fn map(&self, text: &str) -> String {
        text.replace(USER_HOME, &self.home.to_string_lossy())
    }

    /// `allow` or `deny`, from the real binary.
    fn verdict(&self, case: &Case) -> String {
        let cwd = PathBuf::from(self.map(&case.cwd));
        fs::create_dir_all(&cwd).expect("case cwd");
        let command = self.map(&case.command);
        let payload = serde_json::json!({
            "tool_name": "Bash",
            "tool_input": { "command": command },
            "cwd": cwd,
        })
        .to_string();
        let out = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(["hook", "rm-workspace-guard"])
            .env("HOOK_INPUT", payload)
            .env("HOME", &self.home)
            .env_remove("PLAYBOOK_SAFE_ROOTS")
            .env_remove("WORKTREE_BASE_DIR")
            .env_remove("PLAYBOOK_BIN_DIR")
            .env_remove("XDG_CACHE_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .current_dir(&cwd)
            .output()
            .expect("spawn");
        assert!(out.status.success(), "{}: a guard exits 0", case.id);
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains(r#""permissionDecision":"deny""#) {
            "deny".to_string()
        } else {
            "allow".to_string()
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

#[test]
fn the_corpus_is_well_formed() {
    let cases = corpus();
    let mut ids: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();

    assert_eq!(ids.len(), before, "case ids are unique");
    for case in &cases {
        assert!(
            matches!(case.want.as_str(), "allow" | "deny"),
            "{}: want",
            case.id
        );
        assert!(
            matches!(case.expect.as_str(), "allow" | "deny"),
            "{}: expect",
            case.id
        );
        assert!(!case.class.is_empty(), "{}: class", case.id);
        for text in [&case.command, &case.cwd] {
            let named = text.match_indices("/Users/").all(|(at, _)| {
                text[at + 7..].starts_with(USER) || text[at + 7..].starts_with("Shared")
            });
            assert!(named, "{}: anonymized", case.id);
        }
    }
}

#[test]
fn every_corpus_case_gets_its_recorded_verdict() {
    let tree = Tree::new();
    let print = std::env::var_os("RM_GUARD_CORPUS_PRINT").is_some();
    let cases: Vec<Case> = corpus()
        .into_iter()
        .filter(|c| c.os.is_empty() || c.os == std::env::consts::OS)
        .collect();
    // Each case spawns the binary, so they run eight at a time.
    let verdicts: Vec<String> = std::thread::scope(|scope| {
        let tree = &tree;
        let handles: Vec<_> = cases
            .chunks(cases.len().div_ceil(8).max(1))
            .map(|chunk| {
                scope.spawn(move || chunk.iter().map(|c| tree.verdict(c)).collect::<Vec<_>>())
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("case thread"))
            .collect()
    });
    let mut moved = Vec::new();
    let mut gaps = 0;
    for (case, got) in cases.iter().zip(verdicts) {
        if print {
            println!("{}\t{}\t{}", case.id, case.want, got);
        }
        if got != case.expect {
            moved.push(format!("{}: expect {}, got {}", case.id, case.expect, got));
        }
        gaps += usize::from(case.expect != case.want);
    }
    println!("known gaps (expect != want): {gaps}");

    assert!(moved.is_empty(), "verdicts moved:\n{}", moved.join("\n"));
}
