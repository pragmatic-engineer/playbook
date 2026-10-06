// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The PostToolUse backstop of `commit-message-sanitizer` from the real
//! binary, against a scratch repository and a stub `gh`: an unpushed commit
//! that still carries attribution is amended clean, a pushed one is only
//! reported, and everything else prints nothing.

#[path = "support/auto_env.rs"]
mod auto_env;
#[path = "support/sanitizer_lab.rs"]
mod sanitizer_lab;

use sanitizer_lab::{assert_no_attribution, Lab, COAUTHOR};
use serde_json::Value;
use std::fs;

const DIRTY: &str =
    "feat: x\n\nbody line\n\nRefs: 1\nCo-Authored-By: Claude <noreply@anthropic.com>";

fn context(out: &str) -> String {
    let value: Value =
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("not JSON ({e}): {out:?}"));
    let inner = &value["hookSpecificOutput"];
    assert_eq!(inner["hookEventName"], "PostToolUse", "{out}");
    assert!(inner.get("permissionDecision").is_none(), "{out}");
    inner["additionalContext"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn commit_dirty(lab: &Lab) {
    lab.git(&["commit", "--allow-empty", "-q", "-m", DIRTY]);
}

fn sha(lab: &Lab) -> String {
    lab.git(&["rev-parse", "HEAD"]).trim().to_string()
}

#[test]
fn an_unpushed_head_with_attribution_is_amended_clean() {
    let lab = Lab::new("post-amend");
    commit_dirty(&lab);
    let before = sha(&lab);
    let commits_before = lab.git(&["rev-list", "--count", "HEAD"]);

    let out = lab.post("git commit --allow-empty -m x", "");

    let head = lab.head();
    assert_no_attribution(&head);
    assert!(
        head.contains("feat: x") && head.contains("body line") && head.contains("Refs: 1"),
        "{head:?}"
    );
    assert!(
        head.contains("Signed-off-by: Test <test@example.com>"),
        "sign-off kept: {head:?}"
    );
    assert_ne!(sha(&lab), before, "HEAD was amended");
    assert_eq!(
        lab.git(&["rev-list", "--count", "HEAD"]),
        commits_before,
        "amended, not added"
    );
    let note = context(&out);
    assert!(
        note.contains("amended") && note.contains("credit trailer"),
        "{note}"
    );
    assert!(!note.contains("anthropic"), "no value in the note: {note}");
}

#[test]
fn a_head_on_a_remote_branch_is_reported_and_left_alone() {
    let lab = Lab::new("post-pushed");
    commit_dirty(&lab);
    lab.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    let before = sha(&lab);

    let out = lab.post("git push origin main", "");

    assert_eq!(sha(&lab), before, "a pushed commit is never amended");
    assert!(lab.head().contains("Claude"), "message untouched");
    let note = context(&out);
    assert!(note.contains("remote branch"), "{note}");
    assert!(
        note.contains("--force-with-lease") && note.contains("never"),
        "{note}"
    );
}

#[test]
fn a_clean_head_is_left_alone_and_prints_nothing() {
    let lab = Lab::new("post-clean");
    lab.git(&["commit", "--allow-empty", "-q", "-m", "feat: x\n\nRefs: 1"]);
    let before = sha(&lab);

    assert_eq!(lab.post("git commit --allow-empty -m x", ""), "");
    assert_eq!(lab.post("git status", ""), "");
    assert_eq!(sha(&lab), before);
}

#[test]
fn an_ai_author_on_an_unpushed_head_is_replaced_by_the_configured_identity() {
    let lab = Lab::new("post-author");
    lab.git(&[
        "commit",
        "--allow-empty",
        "-q",
        "--author",
        "Claude <noreply@anthropic.com>",
        "-m",
        "feat: x",
    ]);

    let out = lab.post("git commit --allow-empty -m x", "");

    assert_eq!(
        lab.git(&["log", "-1", "--format=%an <%ae>"]).trim(),
        "Test <test@example.com>"
    );
    assert!(context(&out).contains("AI as the author"), "{out}");
}

#[test]
fn an_older_unpushed_commit_is_reported_once_per_session() {
    let lab = Lab::new("post-older");
    commit_dirty(&lab);
    let dirty = sha(&lab);
    lab.git(&["commit", "--allow-empty", "-q", "-m", "feat: y"]);
    let head = sha(&lab);

    let first = lab.post("git log --oneline", "");
    let second = lab.post("git log --oneline", "");

    let note = context(&first);
    assert!(
        note.contains(&dirty[..7]) && note.contains("rebase"),
        "{note}"
    );
    assert_eq!(sha(&lab), head, "only HEAD is ever amended");
    assert_eq!(second, "", "the same session is not told twice");
}

#[test]
fn an_annotated_tag_at_head_with_attribution_is_reported() {
    let lab = Lab::new("post-tag");
    lab.git(&["tag", "-a", "v1", "-m", &format!("release\n\n{COAUTHOR}")]);

    let note = context(&lab.post("git tag -a v1 -m release", ""));

    assert!(
        note.contains("tag v1") && note.contains("credit trailer"),
        "{note}"
    );
    assert!(
        lab.git(&["tag", "-l", "--format=%(contents)", "v1"])
            .contains("Claude"),
        "tags are not rewritten"
    );
}

#[test]
fn a_signed_head_is_amended_with_its_signature_kept() {
    if std::process::Command::new("ssh-keygen")
        .arg("-?")
        .output()
        .is_err()
    {
        return;
    }
    let lab = Lab::new("post-signed");
    let key = lab.scratch.home.join("key");
    let made = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", ""])
        .arg("-f")
        .arg(&key)
        .status()
        .expect("ssh-keygen runs");
    assert!(made.success());
    lab.git(&["config", "gpg.format", "ssh"]);
    lab.git(&[
        "config",
        "user.signingkey",
        &format!("{}.pub", key.display()),
    ]);
    lab.git(&["config", "commit.gpgsign", "true"]);
    commit_dirty(&lab);
    assert!(
        lab.git(&["cat-file", "commit", "HEAD"]).contains("gpgsig"),
        "setup signs the commit"
    );

    lab.post("git commit --allow-empty -m x", "");

    assert_no_attribution(&lab.head());
    assert!(
        lab.git(&["cat-file", "commit", "HEAD"]).contains("gpgsig"),
        "the amend is signed too"
    );
}

#[test]
fn an_unsigned_repo_is_amended_without_a_signature() {
    let lab = Lab::new("post-unsigned");
    commit_dirty(&lab);

    lab.post("git commit --allow-empty -m x", "");

    assert!(!lab.git(&["cat-file", "commit", "HEAD"]).contains("gpgsig"));
}

fn gh_stub(lab: &Lab, pr_json: &str) {
    fs::write(lab.bin.join("pr.json"), pr_json).expect("pr.json");
    lab.write_stub(
        "gh",
        "#!/bin/sh\nd=\"$(dirname \"$0\")\"\nif [ \"$1 $2\" = \"pr view\" ]; then cat \"$d/pr.json\"; exit 0; fi\nif [ \"$1 $2\" = \"pr edit\" ]; then printf '%s\\n' \"$@\" > \"$d/edit.args\"; cat > \"$d/edit.body\"; exit 0; fi\nexit 1\n",
    );
}

#[test]
fn a_pr_created_with_attribution_is_edited_clean() {
    let lab = Lab::new("post-pr");
    let dirty = serde_json::json!({
        "title": "Generated with Claude Code",
        "body": format!("## Summary\n\nbody\n\n{COAUTHOR}\n"),
    });
    gh_stub(&lab, &dirty.to_string());

    let out = lab.post(
        "gh pr create --fill",
        "Creating pull request\nhttps://github.com/o/r/pull/7\n",
    );

    let args = fs::read_to_string(lab.bin.join("edit.args")).expect("gh pr edit ran");
    assert!(args.contains("https://github.com/o/r/pull/7"), "{args}");
    assert!(
        args.contains("--title") && args.contains("--body-file"),
        "{args}"
    );
    assert_no_attribution(&args);
    assert_eq!(
        fs::read_to_string(lab.bin.join("edit.body")).unwrap(),
        "## Summary\n\nbody\n"
    );
    let note = context(&out);
    assert!(
        note.contains("edited the PR") && !note.contains("anthropic"),
        "{note}"
    );
}

#[test]
fn only_the_part_of_a_pr_that_carries_attribution_is_edited() {
    let lab = Lab::new("post-pr-body");
    let dirty = serde_json::json!({ "title": "feat: x", "body": format!("body\n\n{COAUTHOR}") });
    gh_stub(&lab, &dirty.to_string());

    lab.post("gh pr edit 7 --body x", "");

    let args = fs::read_to_string(lab.bin.join("edit.args")).unwrap();
    assert!(
        args.contains("edit\n7\n") && args.contains("--body-file") && !args.contains("--title"),
        "{args}"
    );
}

#[test]
fn a_clean_pr_is_not_edited() {
    let lab = Lab::new("post-pr-clean");
    gh_stub(
        &lab,
        &serde_json::json!({ "title": "feat: x", "body": "## Summary\n\nbody" }).to_string(),
    );

    let out = lab.post("gh pr create --fill", "https://github.com/o/r/pull/7\n");

    assert_eq!(out, "");
    assert!(!lab.bin.join("edit.args").exists());
}

#[test]
fn a_failed_pr_read_is_silent() {
    let lab = Lab::new("post-pr-fail");
    lab.write_stub("gh", "#!/bin/sh\nexit 1\n");

    assert_eq!(
        lab.post("gh pr create --fill", "https://github.com/o/r/pull/7\n"),
        ""
    );
}

#[test]
fn unrelated_commands_print_nothing_and_do_no_git_work() {
    let lab = Lab::new("post-silent");
    commit_dirty(&lab);

    for command in ["ls -la", "echo hello", "cargo test", ""] {
        assert_eq!(lab.post(command, ""), "", "{command:?}");
    }
    assert!(
        lab.head().contains("Claude"),
        "unrelated commands do no git work"
    );
}
