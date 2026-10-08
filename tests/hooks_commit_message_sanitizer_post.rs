// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The PostToolUse backstop of `commit-message-sanitizer` from the real
//! binary, against a scratch repository and a stub `gh`: an unpushed commit
//! that still carries attribution is amended clean, a pushed one is only
//! reported, and everything else prints nothing.

#[path = "support/auto_env.rs"]
mod auto_env;
#[path = "support/sanitizer_lab.rs"]
mod sanitizer_lab;

use sanitizer_lab::{assert_no_attribution, sh, write_executable, Lab, COAUTHOR};
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
    let command = format!("git commit --allow-empty -m {}", sh(DIRTY));
    let (mut before, mut commits_before) = (String::new(), String::new());

    let out = lab.through_hooks(&command, |lab| {
        lab.run(&command);
        before = sha(lab);
        commits_before = lab.git(&["rev-list", "--count", "HEAD"]);
    });

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

    let out = lab.post("git commit --amend --no-edit", "");

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

    let out = lab.run_through_hooks(
        "git commit --allow-empty --author 'Claude <noreply@anthropic.com>' -m x",
    );

    assert_eq!(
        lab.git(&["log", "-1", "--format=%an <%ae>"]).trim(),
        "Test <test@example.com>"
    );
    assert!(context(&out).contains("AI as the author"), "{out}");
}

#[test]
fn an_ai_committer_alone_is_replaced_and_the_human_author_is_kept() {
    let lab = Lab::new("post-committer");
    let command = "git commit --allow-empty --author 'Sam Lee <sam@example.com>' -m x";

    let out = lab.through_hooks(command, |lab| {
        let made = lab
            .base("git")
            .args(["commit", "--allow-empty", "-q", "--author"])
            .args(["Sam Lee <sam@example.com>", "-m", "x"])
            .env("GIT_COMMITTER_NAME", "Claude")
            .env("GIT_COMMITTER_EMAIL", "noreply@anthropic.com")
            .status()
            .expect("git runs");
        assert!(made.success());
    });

    assert_eq!(
        lab.git(&["log", "-1", "--format=%an <%ae>"]).trim(),
        "Sam Lee <sam@example.com>",
        "the author is a person and stays"
    );
    assert_eq!(
        lab.git(&["log", "-1", "--format=%cn <%ce>"]).trim(),
        "Test <test@example.com>"
    );
    assert!(
        context(&out).contains("AI as the author or committer"),
        "{out}"
    );
}

#[test]
fn an_older_unpushed_commit_is_reported_once_per_session() {
    let lab = Lab::new("post-older");
    commit_dirty(&lab);
    let dirty = sha(&lab);
    lab.git(&["commit", "--allow-empty", "-q", "-m", "feat: y"]);
    let head = sha(&lab);

    let first = lab.post("git commit --allow-empty -m y", "");
    let second = lab.post("git commit --allow-empty -m y", "");

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
    let again = lab.post("git tag -a v1 -m release", "");

    assert!(
        note.contains("tag v1") && note.contains("credit trailer"),
        "{note}"
    );
    assert!(
        lab.git(&["tag", "-l", "--format=%(contents)", "v1"])
            .contains("Claude"),
        "tags are not rewritten"
    );
    assert_eq!(again, "", "a tag is reported once per session");
}

#[test]
fn a_lightweight_tag_is_not_judged_by_the_commit_message_it_points_at() {
    let lab = Lab::new("post-light-tag");
    lab.git(&[
        "commit",
        "--allow-empty",
        "-q",
        "-m",
        &format!("feat: x\n\nRefs: 1\n{COAUTHOR}"),
    ]);
    lab.git(&["tag", "light"]);
    lab.git(&["tag", "-a", "v2", "-m", "release"]);

    assert_eq!(lab.post("git tag light", ""), "");
}

/// A lab whose repository signs commits with a fresh ssh key, or `None` when
/// `ssh-keygen` is missing.
fn signing_lab(tag: &str) -> Option<Lab> {
    std::process::Command::new("ssh-keygen")
        .arg("-?")
        .output()
        .ok()?;
    let lab = Lab::new(tag);
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
    Some(lab)
}

fn signed(lab: &Lab) -> bool {
    lab.git(&["cat-file", "commit", "HEAD"]).contains("gpgsig")
}

#[test]
fn signing_follows_the_repository_config_and_the_amend_never_forces_it() {
    for still_signs in [true, false] {
        let Some(lab) = signing_lab("post-sign-config") else {
            return;
        };
        let command = format!("git commit --allow-empty -m {}", sh(DIRTY));

        lab.through_hooks(&command, |lab| {
            lab.run(&command);
            assert!(signed(lab), "setup signs the commit");
            lab.git(&["config", "commit.gpgsign", &still_signs.to_string()]);
        });

        assert_no_attribution(&lab.head());
        assert_eq!(signed(&lab), still_signs, "the amend follows the config");
    }
}

#[test]
fn an_unsigned_repo_is_amended_without_a_signature() {
    let lab = Lab::new("post-unsigned");

    lab.run_through_hooks(&format!("git commit --allow-empty -m {}", sh(DIRTY)));

    assert_no_attribution(&lab.head());
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
fn a_pr_title_that_is_only_attribution_is_left_with_a_note_and_the_body_is_still_cleaned() {
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
        args.contains("--body-file") && !args.contains("--title"),
        "an empty title is never sent: {args}"
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
    assert!(
        note.contains("title") && note.contains("left as it is") && note.contains("rename"),
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
fn a_pr_title_is_edited_only_when_text_is_left_of_it() {
    let lab = Lab::new("post-pr-title");
    let created = "https://github.com/o/r/pull/7\n";
    let only = serde_json::json!({ "title": "Generated with Claude Code", "body": "body" });
    gh_stub(&lab, &only.to_string());

    let out = lab.post("gh pr create --fill", created);

    assert!(!lab.bin.join("edit.args").exists(), "nothing to edit");
    assert!(context(&out).contains("rename"), "{out}");
    let kept =
        serde_json::json!({ "title": "feat: x\nGenerated with Claude Code", "body": "body" });
    gh_stub(&lab, &kept.to_string());

    lab.post("gh pr create --fill", created);

    let args = fs::read_to_string(lab.bin.join("edit.args")).unwrap();
    assert!(args.contains("--title\nfeat: x\n"), "{args}");
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

/// The real `git`, found on the path the tests started with.
fn real_git() -> String {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(|dir| std::path::Path::new(dir).join("git"))
        .find(|candidate| candidate.is_file())
        .expect("git is installed")
        .display()
        .to_string()
}

/// A `git` ahead of the real one that logs each call, then runs it, with
/// `before` run first for a call whose arguments contain `when`.
fn git_spy(lab: &Lab, when: &str, before: &str) {
    let log = lab.bin.join("git.calls");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> {log}\ncase \"$*\" in *'{when}'*) {before};; esac\nexec {real} \"$@\"\n",
        log = log.display(),
        real = real_git(),
    );
    lab.write_stub("git", &script);
}

fn git_calls(lab: &Lab) -> String {
    fs::read_to_string(lab.bin.join("git.calls")).unwrap_or_default()
}

#[test]
fn a_call_that_runs_no_git_commit_or_tag_spawns_no_git() {
    let lab = Lab::new("post-cheap");
    commit_dirty(&lab);
    git_spy(&lab, "never matches", "true");
    let commands = [
        "git status",
        "git log --oneline -3",
        "git diff HEAD",
        "git push origin main",
        "git commit-tree HEAD^{tree}",
        "echo git commit -m x",
        "grep -r 'git tag' .",
        "gh pr view 3",
        "gh pr list",
        "gh pr comment 3 --body git",
    ];
    for command in commands {
        assert_eq!(lab.post(command, ""), "", "{command}");
    }
    assert_eq!(git_calls(&lab), "", "no git process was started");
    assert!(!lab.post("git commit --allow-empty -m x", "").is_empty());
    assert!(!git_calls(&lab).is_empty(), "a git commit does start git");
}

#[test]
fn a_commit_made_without_a_message_option_is_only_reported() {
    let lab = Lab::new("post-editor");
    let command = "git commit --allow-empty";
    let mut before = String::new();

    let out = lab.through_hooks(command, |lab| {
        commit_dirty(lab);
        before = sha(lab);
    });
    let again = lab.post(command, "");

    assert_eq!(sha(&lab), before, "an editor-written commit is not amended");
    let note = context(&out);
    assert!(note.contains("written in an editor"), "{note}");
    assert!(note.contains("Amend it with a clean message"), "{note}");
    assert_eq!(again, "", "reported once per session");
}

#[test]
fn a_commit_that_this_call_did_not_make_is_never_amended() {
    let commands = [
        "git commit -m x",
        "git commit -m x || true",
        "git commit --dry-run --allow-empty -m x",
        "git status",
    ];
    for command in commands {
        let lab = Lab::new("post-not-this-call");
        commit_dirty(&lab);
        let before = sha(&lab);

        let out = lab.run_through_hooks(command);

        let ran = lab.git(&["rev-list", "--count", "HEAD"]);
        assert_eq!(
            sha(&lab),
            before,
            "{command}: HEAD is the old, dirty commit"
        );
        assert!(
            lab.head().contains("Claude"),
            "{command}: message untouched"
        );
        assert!(ran.trim() == "2", "{command}: {ran}");
        if command.starts_with("git commit") {
            assert!(
                context(&out).contains("not made by this call"),
                "{command}: {out}"
            );
        }
    }
}

#[test]
fn a_commit_in_another_repository_is_amended_there_and_the_current_one_is_left_alone() {
    let lab = Lab::new("post-other-repo");
    commit_dirty(&lab);
    let here = sha(&lab);
    let other = Lab::new("post-other-repo-target");
    let command = format!(
        "git -C {} commit --allow-empty -m {}",
        other.repo.display(),
        sh(DIRTY)
    );

    let out = lab.run_through_hooks(&command);

    assert_eq!(sha(&lab), here, "the current repository is not touched");
    assert!(lab.head().contains("Claude"));
    assert_no_attribution(&other.head());
    assert!(context(&out).contains("amended"), "{out}");
}

#[test]
fn a_head_that_moved_without_a_commit_is_never_amended() {
    let lab = Lab::new("post-moved");
    let before = sha(&lab);
    let command = "git commit --allow-empty -m x";

    let out = lab.through_hooks(command, |lab| {
        lab.git(&["checkout", "-q", "-b", "side"]);
        commit_dirty(lab);
        lab.git(&["checkout", "-q", "main"]);
        lab.git(&["merge", "-q", "--ff-only", "side"]);
    });

    assert_ne!(sha(&lab), before);
    assert!(
        lab.head().contains("Claude"),
        "a fast-forward is not a commit this call made"
    );
    assert!(context(&out).contains("not made by this call"), "{out}");
}

#[test]
fn a_commit_pushed_by_the_same_call_is_never_amended() {
    let lab = Lab::new("post-push-call");
    let command = format!(
        "git commit --allow-empty -m {} && git push origin main",
        sh(DIRTY)
    );
    let mut before = String::new();

    let out = lab.through_hooks(&command, |lab| {
        lab.run(&format!("git commit --allow-empty -m {}", sh(DIRTY)));
        before = sha(lab);
    });

    assert_eq!(sha(&lab), before);
    assert!(context(&out).contains("pushed by this call"), "{out}");
}

#[test]
fn no_amend_happens_while_a_rebase_merge_cherry_pick_or_bisect_is_under_way() {
    for state in [
        "rebase-merge",
        "rebase-apply",
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "BISECT_LOG",
    ] {
        let lab = Lab::new("post-in-progress");
        let mut before = String::new();

        let out = lab.through_hooks("git commit --allow-empty -m x", |lab| {
            commit_dirty(lab);
            before = sha(lab);
            let path = lab.repo.join(".git").join(state);
            if state.starts_with("rebase") {
                fs::create_dir_all(&path).unwrap();
            } else {
                fs::write(&path, "x\n").unwrap();
            }
        });

        assert_eq!(sha(&lab), before, "{state}");
        assert!(context(&out).contains("under way"), "{state}: {out}");
    }
}

/// The commit objects the amend must keep: tree, parents, author and date.
fn shape(lab: &Lab) -> Vec<String> {
    ["%T", "%P", "%an <%ae> %aI", "%aD"]
        .iter()
        .map(|format| lab.git(&["log", "-1", &format!("--format={format}")]))
        .collect()
}

#[test]
fn the_amend_changes_the_message_alone_and_leaves_the_index_and_author_as_they_are() {
    let lab = Lab::new("post-message-only");
    lab.git(&["checkout", "-q", "-b", "side"]);
    lab.git(&["commit", "--allow-empty", "-q", "-m", "side"]);
    lab.git(&["checkout", "-q", "main"]);
    lab.git(&["commit", "--allow-empty", "-q", "-m", "main work"]);
    let hooks = lab.repo.join("custom-hooks");
    fs::create_dir_all(&hooks).unwrap();
    let marker = lab.repo.join("hook-ran");
    for name in ["pre-commit", "commit-msg", "post-commit", "post-rewrite"] {
        write_executable(
            &hooks,
            name,
            &format!("#!/bin/sh\necho {name} >> {}\n", marker.display()),
        );
    }
    lab.git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let command = format!(
        "git commit --allow-empty --author 'Sam Lee <sam@example.com>' --date 2020-01-02T03:04:05+00:00 -m {}",
        sh(DIRTY)
    );
    fs::write(lab.repo.join("staged.txt"), "staged\n").unwrap();
    fs::write(lab.repo.join("tracked.txt"), "one\n").unwrap();
    let (mut kept, mut staged, mut index) = (Vec::new(), String::new(), Vec::new());

    lab.through_hooks(&command, |lab| {
        lab.git(&["merge", "-q", "--no-commit", "--no-ff", "side"]);
        lab.run(&command);
        lab.git(&["add", "staged.txt", "tracked.txt"]);
        fs::write(lab.repo.join("unstaged.txt"), "x\n").unwrap();
        kept = shape(lab);
        staged = lab.git(&["diff", "--cached", "--name-status"]);
        index = fs::read(lab.repo.join(".git/index")).unwrap();
        fs::remove_file(&marker).unwrap_or_default();
    });

    assert_no_attribution(&lab.head());
    assert_eq!(
        kept[1].split_whitespace().count(),
        2,
        "a merge has two parents"
    );
    assert_eq!(shape(&lab), kept, "tree, parents, author and date");
    assert_eq!(
        lab.git(&["diff", "--cached", "--name-status"]),
        staged,
        "still staged"
    );
    assert_eq!(
        fs::read(lab.repo.join(".git/index")).unwrap(),
        index,
        "the index file is untouched"
    );
    assert!(lab.repo.join("unstaged.txt").exists());
    assert!(
        !marker.exists(),
        "no hook ran: {:?}",
        fs::read_to_string(&marker)
    );
    assert_eq!(
        lab.git(&["reflog", "-1", "--format=%gs"]).trim(),
        "playbook: clean commit message"
    );
}

#[test]
fn a_head_that_moves_while_the_amend_runs_is_left_as_it_is() {
    let lab = Lab::new("post-race");
    let command = format!("git commit --allow-empty -m {}", sh(DIRTY));
    git_spy(&lab, "update-ref", "git commit --allow-empty -q -m racer");

    let out = lab.run_through_hooks(&command);

    assert_eq!(
        lab.head().trim(),
        "racer",
        "the other writer's commit stays"
    );
    assert!(context(&out).contains("could not amend"), "{out}");
}

#[test]
fn the_sign_off_is_added_when_missing_and_never_duplicated() {
    let lab = Lab::new("post-sign-off");
    let signed = format!("{DIRTY}\nSigned-off-by: Test <test@example.com>");

    lab.run_through_hooks(&format!("git commit --allow-empty -m {}", sh(&signed)));

    let head = lab.head();
    assert_no_attribution(&head);
    assert_eq!(
        head.matches("Signed-off-by: Test <test@example.com>")
            .count(),
        1,
        "{head}"
    );

    let unsigned = Lab::new("post-sign-off-missing");
    unsigned.run_through_hooks(&format!("git commit --allow-empty -m {}", sh(DIRTY)));
    assert_eq!(
        unsigned
            .head()
            .matches("Signed-off-by: Test <test@example.com>")
            .count(),
        1
    );
}

#[test]
fn a_commit_made_with_no_signoff_gets_no_sign_off_from_the_backstop() {
    let lab = Lab::new("post-no-signoff");

    lab.run_through_hooks(&format!(
        "git commit --allow-empty --no-signoff -m {}",
        sh(DIRTY)
    ));

    let head = lab.head();
    assert_no_attribution(&head);
    assert!(!head.contains("Signed-off-by"), "{head}");
}

#[test]
fn commit_sign_off_false_keeps_the_backstop_from_adding_a_sign_off() {
    let lab = Lab::new("post-sign-off-config-off");
    lab.write_global_config(r#"{"commit":{"signOff":false}}"#);

    lab.run_through_hooks(&format!("git commit --allow-empty -m {}", sh(DIRTY)));

    assert!(!lab.head().contains("Signed-off-by"), "{}", lab.head());
}

#[test]
fn the_amend_never_touches_the_index_so_a_lock_held_by_another_git_does_not_stop_it() {
    let lab = Lab::new("post-lock");
    let command = format!("git commit --allow-empty -m {}", sh(DIRTY));

    let out = lab.through_hooks(&command, |lab| {
        lab.run(&command);
        fs::write(lab.repo.join(".git/index.lock"), "").unwrap();
    });

    assert_no_attribution(&lab.head());
    assert!(context(&out).contains("amended"), "{out}");
    assert!(
        lab.repo.join(".git/index.lock").exists(),
        "another git's lock is left alone"
    );
}

#[test]
fn an_edit_with_a_repo_flag_reads_and_edits_that_repo_and_the_push_hint_is_not_a_pr() {
    let lab = Lab::new("post-pr-repo");
    let dirty = serde_json::json!({ "title": "feat: x", "body": format!("body\n\n{COAUTHOR}") });
    gh_stub(&lab, &dirty.to_string());
    lab.write_stub(
        "gh",
        "#!/bin/sh\nd=\"$(dirname \"$0\")\"\nprintf '%s\\n' \"$@\" >> \"$d/gh.calls\"\nif [ \"$1 $2\" = \"pr view\" ]; then cat \"$d/pr.json\"; exit 0; fi\nif [ \"$1 $2\" = \"pr edit\" ]; then cat > \"$d/edit.body\"; exit 0; fi\nexit 1\n",
    );

    for command in [
        "gh pr edit 7 --repo o/r --body x",
        "gh pr edit 7 -R o/r --body x",
        "gh pr edit 7 -Ro/r --body x",
    ] {
        let _ = fs::remove_file(lab.bin.join("gh.calls"));

        lab.post(command, "");

        let calls = fs::read_to_string(lab.bin.join("gh.calls")).expect("gh ran");
        assert_eq!(
            calls.matches("--repo\no/r\n").count(),
            2,
            "view and edit: {command}: {calls}"
        );
    }
    let hint = "remote: Create a pull request for 'feat' on GitHub by visiting:\nremote:      https://github.com/o/r/pull/new/feat\n";
    let _ = fs::remove_file(lab.bin.join("gh.calls"));
    assert_eq!(lab.post("gh pr create --fill", hint), "");
    assert!(!lab.bin.join("gh.calls").exists(), "the hint is not a PR");
    lab.post(
        "gh pr create --fill",
        &format!("{hint}https://github.com/o/r/pull/12\n"),
    );
    assert!(fs::read_to_string(lab.bin.join("gh.calls"))
        .unwrap()
        .contains("pull/12"));
}
