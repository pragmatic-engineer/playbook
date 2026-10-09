// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! `playbook commit prepare` and `commit run` against real git repos with a
//! local bare `origin`. They replace the two bash blocks of
//! `commands/commit-and-push.md`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

struct Repo {
    root: PathBuf,
    work: PathBuf,
    origin: PathBuf,
}

fn git_in(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let o = git_in(dir, args);
    assert!(
        o.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

impl Repo {
    /// `main` pushed to a bare origin, and a local branch `feat` checked out.
    fn new(tag: &str) -> Repo {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("pb-commit-{tag}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let origin = root.join("origin.git");
        let work = root.join("work");
        fs::create_dir_all(&origin).unwrap();
        fs::create_dir_all(&work).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        git(&origin, &["init", "-q", "--bare", "-b", "main"]);
        git(&work, &["init", "-q", "-b", "main"]);
        git(
            &work,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        fs::write(work.join("a.txt"), "a\n").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "init"]);
        git(&work, &["push", "-q", "-u", "origin", "main"]);
        git(&work, &["checkout", "-q", "-b", "feat"]);
        Repo { root, work, origin }
    }

    fn pb(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_playbook"))
            .args(args)
            .env("HOME", self.root.join("home"))
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .current_dir(&self.work)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn stage(&self, file: &str, body: &str) {
        fs::write(self.work.join(file), body).unwrap();
        git(&self.work, &["add", file]);
    }

    fn remote_head(&self, branch: &str) -> String {
        git(
            &self.origin,
            &["rev-parse", &format!("refs/heads/{branch}")],
        )
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

// prepare

#[test]
fn prepare_refuses_the_default_branch() {
    let r = Repo::new("prot");
    git(&r.work, &["checkout", "-q", "main"]);
    r.stage("b.txt", "b\n");
    let o = r.pb(&["commit", "prepare"], "");
    assert!(!o.status.success());
    assert!(err(&o).contains("HEAD is on 'main'"), "{}", err(&o));
}

#[test]
fn prepare_with_nothing_staged_says_so_and_exits_zero() {
    let r = Repo::new("none");
    let o = r.pb(&["commit", "prepare"], "");
    assert!(o.status.success());
    assert_eq!(out(&o).trim(), "NO_STAGED_CHANGES");
}

#[test]
fn prepare_prints_the_branch_the_files_and_the_diff() {
    let r = Repo::new("ctx");
    r.stage("b.txt", "hello\n");
    let t = out(&r.pb(&["commit", "prepare"], ""));
    assert!(t.starts_with("BRANCH=feat"), "{t}");
    assert!(
        t.contains("A\tb.txt") && t.contains("---DIFF_START---") && t.contains("+hello"),
        "{t}"
    );
}

#[test]
fn prepare_stages_with_the_flags() {
    let r = Repo::new("flags");
    fs::write(r.work.join("a.txt"), "changed\n").unwrap();
    fs::write(r.work.join("new.txt"), "new\n").unwrap();
    let t = out(&r.pb(&["commit", "prepare", "-u"], ""));
    assert!(t.contains("M\ta.txt") && !t.contains("new.txt"), "{t}");
    let t = out(&r.pb(&["commit", "prepare", "-A"], ""));
    assert!(t.contains("new.txt"), "{t}");
}

#[test]
fn prepare_amend_shows_the_last_commit() {
    let r = Repo::new("amend");
    let t = out(&r.pb(&["commit", "prepare", "-a"], ""));
    assert!(t.contains("AMENDING: ") && t.contains("init"), "{t}");
}

// run

#[test]
fn run_commits_with_a_signoff_and_pushes() {
    let r = Repo::new("run");
    r.stage("b.txt", "b\n");
    let o = r.pb(&["commit", "run"], "feat: add b\n");
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert!(
        out(&o).contains("Pushed:") && out(&o).contains("-> origin/feat"),
        "{}",
        out(&o)
    );
    let body = git(&r.work, &["log", "-1", "--format=%B"]);
    assert!(body.contains("Signed-off-by: t <t@t>"), "{body}");
    assert_eq!(r.remote_head("feat"), git(&r.work, &["rev-parse", "HEAD"]));
}

#[test]
fn run_leaves_the_signoff_off_when_asked_or_configured() {
    let r = Repo::new("nosign");
    r.stage("b.txt", "b\n");
    assert!(r
        .pb(&["commit", "run", "--no-signoff"], "feat: b\n")
        .status
        .success());
    assert!(!git(&r.work, &["log", "-1", "--format=%B"]).contains("Signed-off-by"));

    r.stage("c.txt", "c\n");
    assert!(r
        .pb(
            &["config", "set", "--global", "commit.signOff", "false"],
            ""
        )
        .status
        .success());
    assert!(r.pb(&["commit", "run"], "feat: c\n").status.success());
    assert!(!git(&r.work, &["log", "-1", "--format=%B"]).contains("Signed-off-by"));
}

#[test]
fn run_keeps_an_existing_trailer_without_doubling_it() {
    let r = Repo::new("trailer");
    r.stage("b.txt", "b\n");
    assert!(r
        .pb(
            &["commit", "run"],
            "feat: b\n\nSigned-off-by: Someone <s@s>\n"
        )
        .status
        .success());
    let body = git(&r.work, &["log", "-1", "--format=%B"]);
    assert_eq!(body.matches("Signed-off-by").count(), 1, "{body}");
}

#[test]
fn run_stops_when_the_commit_fails_and_pushes_nothing() {
    let r = Repo::new("hookfail");
    let hook = r.work.join(".git/hooks/commit-msg");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    r.stage("b.txt", "b\n");
    let o = r.pb(&["commit", "run"], "feat: b\n");
    assert!(!o.status.success());
    assert!(err(&o).contains("git commit failed"), "{}", err(&o));
    assert!(
        git_in(&r.origin, &["rev-parse", "refs/heads/feat"])
            .status
            .code()
            != Some(0)
    );
}

#[test]
fn run_rebases_when_behind_and_pushes_with_a_lease() {
    let r = Repo::new("rebase");
    // First push of feat, then main moves on.
    r.stage("b.txt", "b\n");
    assert!(r.pb(&["commit", "run"], "feat: b\n").status.success());
    let other = r.root.join("other");
    git(
        &r.root,
        &["clone", "-q", r.origin.to_str().unwrap(), "other"],
    );
    fs::write(other.join("m.txt"), "m\n").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-q", "-m", "main moves"]);
    git(&other, &["push", "-q", "origin", "main"]);
    r.stage("c.txt", "c\n");
    let o = r.pb(&["commit", "run"], "feat: c\n");
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert!(
        out(&o).contains("commits behind origin/main. Rebasing"),
        "{}",
        out(&o)
    );
    assert_eq!(r.remote_head("feat"), git(&r.work, &["rev-parse", "HEAD"]));
    assert!(git(&r.work, &["log", "--format=%s"]).contains("main moves"));
}

#[test]
fn run_refuses_a_branch_with_a_merge_commit() {
    let r = Repo::new("merge");
    git(&r.work, &["checkout", "-q", "-b", "side"]);
    fs::write(r.work.join("s.txt"), "s\n").unwrap();
    git(&r.work, &["add", "."]);
    git(&r.work, &["commit", "-q", "-m", "side"]);
    git(&r.work, &["checkout", "-q", "feat"]);
    fs::write(r.work.join("f.txt"), "f\n").unwrap();
    git(&r.work, &["add", "."]);
    git(&r.work, &["commit", "-q", "-m", "feat work"]);
    git(
        &r.work,
        &["merge", "-q", "--no-ff", "-m", "merge side", "side"],
    );
    r.stage("c.txt", "c\n");
    let o = r.pb(&["commit", "run"], "feat: c\n");
    assert!(!o.status.success());
    assert!(
        err(&o).contains("merge commit(s) on this branch"),
        "{}",
        err(&o)
    );
}

#[test]
fn auto_mode_pushes_a_new_branch_plainly_and_parks_an_amend() {
    let r = Repo::new("auto");
    r.stage("b.txt", "b\n");
    let o = r.pb(&["commit", "run", "--auto"], "feat: b\n");
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    // The branch now exists on the remote, so an amend would need a force.
    r.stage("c.txt", "c\n");
    let o = r.pb(&["commit", "run", "--auto", "-a"], "feat: b again\n");
    assert!(!o.status.success());
    assert!(err(&o).contains("PARKED"), "{}", err(&o));
}

#[test]
fn a_rejected_plain_push_is_never_forced() {
    let r = Repo::new("reject");
    r.stage("b.txt", "b\n");
    assert!(r.pb(&["commit", "run"], "feat: b\n").status.success());
    // Someone else moves the remote branch.
    let other = r.root.join("other");
    git(
        &r.root,
        &[
            "clone",
            "-q",
            "-b",
            "feat",
            r.origin.to_str().unwrap(),
            "other",
        ],
    );
    fs::write(other.join("o.txt"), "o\n").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-q", "-m", "other work"]);
    git(&other, &["push", "-q", "origin", "feat"]);
    let theirs = r.remote_head("feat");
    r.stage("c.txt", "c\n");
    let o = r.pb(&["commit", "run"], "feat: c\n");
    assert!(!o.status.success());
    assert!(err(&o).contains("never auto-escalates"), "{}", err(&o));
    assert_eq!(
        r.remote_head("feat"),
        theirs,
        "the remote must be untouched"
    );
}
