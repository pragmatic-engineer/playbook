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
        let root = std::env::temp_dir().join(format!(
            "pb-review-{tag}-{}-{n}",
            playbook::testing::run_id()
        ));
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
  "repo view"*) echo '{{"nameWithOwner":"o/r","url":"https://github.com/o/r"}}';;
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

impl Sandbox {
    fn replace_gh(&self, script: &str) {
        let f = self.root.join("bin/gh");
        fs::write(&f, script).unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn the_four_gh_calls_run_at_the_same_time() {
    let s = Sandbox::new("concurrent", "alice", "bob");
    let rendezvous = s.root.join("rv");
    fs::create_dir_all(&rendezvous).unwrap();
    // Each call checks in, then waits until all four have. A serial run never
    // sees four check-ins, so `ok` is written only when the calls overlap.
    let script = format!(
        r#"#!/bin/sh
D={rv}
case "$*" in
  "repo view"*|"pr view 7 --json headRefOid"*|"pr view 7 --json author"*|"api /user"*)
    touch "$D/$$"
    i=0
    while [ "$(ls "$D" | grep -vc '^ok$')" -lt 4 ] && [ "$i" -lt 100 ]; do sleep 0.05; i=$((i+1)); done
    [ "$(ls "$D" | grep -vc '^ok$')" -ge 4 ] && touch "$D/ok"
    ;;
esac
case "$*" in
  "repo view"*) echo '{{"nameWithOwner":"o/r","url":"https://github.com/o/r"}}';;
  "pr view 7 --json headRefOid"*) echo {sha};;
  "pr view 7 --json author"*) echo alice;;
  "api /user"*) echo bob;;
esac
"#,
        rv = rendezvous.display(),
        sha = s.sha
    );
    s.replace_gh(&script);
    let o = s.run(&["review", "prepare", "quick", "7"]);
    let t = text(&o);
    assert!(
        o.status.success(),
        "{t}{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(
        rendezvous.join("ok").exists(),
        "the gh calls ran one by one"
    );
    // Same output as the serial version.
    assert_eq!(field(&t, "REPO"), "o/r");
    assert_eq!(field(&t, "HEAD_SHA"), s.sha);
    assert_eq!(field(&t, "AUTHOR"), "alice");
    assert_eq!(field(&t, "SELF_REVIEW"), "false");
}

#[test]
fn when_two_calls_fail_the_first_in_the_old_order_is_reported() {
    let s = Sandbox::new("precedence", "alice", "bob");
    s.replace_gh(
        r#"#!/bin/sh
case "$*" in
  "repo view"*) echo '{"nameWithOwner":"o/r","url":"https://github.com/o/r"}';;
  "pr view 7 --json headRefOid"*) sleep 0.3; echo "head failed" >&2; exit 1;;
  "pr view 7 --json author"*) echo "author failed" >&2; exit 1;;
  "api /user"*) echo "user failed" >&2; exit 1;;
esac
"#,
    );
    let o = s.run(&["review", "prepare", "quick", "7"]);
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("head failed"), "{err}");
    assert!(!err.contains("author failed"), "{err}");
}

#[test]
fn a_failing_repo_call_is_reported_first() {
    let s = Sandbox::new("repo-fails", "alice", "bob");
    s.replace_gh(
        r#"#!/bin/sh
case "$*" in
  "repo view"*) echo "repo failed" >&2; exit 1;;
  *) echo "other failed" >&2; exit 1;;
esac
"#,
    );
    let o = s.run(&["review", "prepare", "quick", "7"]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("repo failed"), "{err}");
}
