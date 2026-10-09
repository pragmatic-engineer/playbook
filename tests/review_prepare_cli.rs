// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook review prepare` and `review checks` against a fake `gh` and a real
//! git repo. They replace the bash setup blocks of the two review commands.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

struct Sandbox {
    root: PathBuf,
    repo: PathBuf,
    sha: String,
}

impl Sandbox {
    /// A clean repo whose HEAD is `sha`, and a fake `gh` for PR 7 authored by `author`.
    fn new(tag: &str, author: &str, me: &str) -> Sandbox {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("pb-review-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        let git = |args: &[&str]| {
            let o = Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .output()
                .unwrap();
            assert!(o.status.success(), "{args:?}");
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        };
        git(&["init", "-q"]);
        git(&["commit", "-q", "--allow-empty", "-m", "x"]);
        let sha = git(&["rev-parse", "HEAD"]);
        let gh = format!(
            r#"#!/bin/sh
case "$*" in
  "repo view"*) echo o/r;;
  "pr view 7 --json headRefOid"*) echo {sha};;
  "pr view 7 --json author"*) echo {author};;
  "pr view --json number"*) echo 7;;
  "pr list --head feat/x"*) echo 7;;
  "pr list --head nope"*) ;;
  "api /user"*) echo {me};;
esac
"#
        );
        let f = root.join("bin/gh");
        fs::write(&f, gh).unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).unwrap();
        Sandbox { root, repo, sha }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(args)
            .env("HOME", self.root.join("home"))
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
            )
            .current_dir(&self.repo)
            .output()
            .unwrap()
    }
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn field(t: &str, key: &str) -> String {
    t.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")))
        .unwrap_or("<missing>")
        .to_string()
}

#[test]
fn a_clean_matching_checkout_reviews_in_place_and_posts() {
    let s = Sandbox::new("inplace", "alice", "bob");
    let o = s.run(&["review", "prepare", "quick", "7"]);
    let t = text(&o);
    assert!(
        o.status.success(),
        "{t}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(field(&t, "PR_NUMBER"), "7");
    assert_eq!(field(&t, "HEAD_SHA"), s.sha);
    assert_eq!(field(&t, "SELF_REVIEW"), "false");
    assert_eq!(field(&t, "SELF_MODE"), "false");
    assert!(field(&t, "MODE").starts_with("in-place"), "{t}");
    assert_eq!(field(&t, "REVIEW_JSON"), "/tmp/o/r/quick-review-7.json");
    assert!(PathBuf::from("/tmp/o/r").is_dir());
}

#[test]
fn deep_names_its_own_review_json() {
    let s = Sandbox::new("deep", "alice", "bob");
    let t = text(&s.run(&["review", "prepare", "deep", "#7"]));
    assert_eq!(field(&t, "REVIEW_JSON"), "/tmp/o/r/deep-review-7.json");
}

#[test]
fn your_own_pr_is_report_only() {
    let s = Sandbox::new("own", "bob", "bob");
    let t = text(&s.run(&["review", "prepare", "deep", "7"]));
    assert_eq!(field(&t, "SELF_REVIEW"), "true");
    assert_eq!(field(&t, "SELF_MODE"), "true");
}

#[test]
fn self_flag_auto_and_no_target_are_each_report_only() {
    let s = Sandbox::new("modes", "alice", "bob");
    assert_eq!(
        field(
            &text(&s.run(&["review", "prepare", "quick", "7 --self"])),
            "SELF_MODE"
        ),
        "true"
    );
    assert_eq!(
        field(
            &text(&s.run(&["review", "prepare", "quick", "7", "--auto"])),
            "SELF_MODE"
        ),
        "true"
    );
    let t = text(&s.run(&["review", "prepare", "quick"]));
    assert_eq!(field(&t, "PR_NUMBER"), "7");
    assert_eq!(field(&t, "SELF_MODE"), "true");
}

#[test]
fn a_branch_name_resolves_to_its_pr_and_an_unknown_one_errors() {
    let s = Sandbox::new("branch", "alice", "bob");
    let t = text(&s.run(&["review", "prepare", "deep", "feat/x"]));
    assert_eq!(field(&t, "PR_NUMBER"), "7");
    let o = s.run(&["review", "prepare", "deep", "nope"]);
    assert!(!o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("no open PR for branch nope"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn a_dirty_tree_goes_to_a_worktree_and_a_failure_is_reported() {
    let s = Sandbox::new("dirty", "alice", "bob");
    fs::write(s.repo.join("f"), "a").unwrap();
    assert!(Command::new("git")
        .args(["add", "f"])
        .current_dir(&s.repo)
        .status()
        .unwrap()
        .success());
    let o = s.run(&["review", "prepare", "quick", "7"]);
    assert!(!o.status.success());
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("error: worktree setup failed"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn checks_skip_an_unknown_toolchain() {
    let s = Sandbox::new("checks", "a", "b");
    let o = s.run(&["review", "checks", s.root.join("home").to_str().unwrap()]);
    assert_eq!(text(&o).trim(), "[no recognised toolchain; checks skipped]");
}
