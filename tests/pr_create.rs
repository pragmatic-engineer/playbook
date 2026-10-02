// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `pr create` against real scratch git repos with a local bare `origin` and
//! a fake `GhClient` that records the calls it receives.

mod pr_support;

use playbook::pr::create::run;
use pr_support::{git, in_dir, FakeGh, Fixture};
use std::fs;

const URL: &str = "https://example.test/pr/42";

fn body(fx: &Fixture) -> String {
    let path = fx.state.join("pr-body.md");
    fs::create_dir_all(&fx.state).expect("state dir");
    fs::write(&path, "## Summary\n\nbody\n").expect("body");
    path.to_str().expect("utf8").to_string()
}

fn create(fx: &Fixture, gh: &FakeGh, title: &str, base: Option<&str>) -> Result<String, String> {
    let body_file = body(fx);
    in_dir(&fx.work, || run(gh, title, &body_file, base))
}

fn remote_has_branch(fx: &Fixture, branch: &str) -> bool {
    let reference = format!("refs/heads/{branch}");
    !git(
        &fx.bare,
        &["for-each-ref", "--format=%(refname)", &reference],
    )
    .is_empty()
}

#[test]
fn a_title_over_72_characters_is_rejected_before_anything_is_pushed_or_created() {
    // Arrange
    let fx = Fixture::new("t73", "feat/x");
    fx.commit_lines("a.txt", 2);
    let gh = FakeGh::creating(URL);

    // Act
    let got = create(&fx, &gh, &"a".repeat(73), None);

    // Assert
    assert!(got.is_err(), "73 characters is one past the limit");
    assert_eq!(gh.count("pr_create"), 0, "no PR may be created");
    assert!(!remote_has_branch(&fx, "feat/x"), "nothing may be pushed");
}

#[test]
fn a_title_of_exactly_72_characters_is_accepted() {
    // Arrange
    let fx = Fixture::new("t72", "feat/x");
    fx.commit_lines("a.txt", 2);
    let gh = FakeGh::creating(URL);

    // Act
    let got = create(&fx, &gh, &"a".repeat(72), None);

    // Assert
    assert!(got.is_ok(), "72 characters is within the limit: {got:?}");
}

#[test]
fn a_successful_run_pushes_creates_and_reports_the_url_the_fake_returned() {
    // Arrange
    let fx = Fixture::new("ok", "feat/x");
    fx.commit_lines("a.txt", 2);
    let gh = FakeGh::creating(URL);

    // Act
    let got = create(&fx, &gh, "feat(pr): add a thing", None).expect("creates");

    // Assert
    assert!(got.lines().any(|l| l == format!("PR: {URL}")), "got {got}");
    assert!(
        remote_has_branch(&fx, "feat/x"),
        "the branch must be on origin"
    );
    assert_eq!(gh.count("pr_create"), 1);
}

#[test]
fn a_failed_push_never_reaches_pr_create() {
    // Arrange: an origin that cannot be reached, with no network involved.
    let fx = Fixture::new("pushfail", "feat/x");
    fx.commit_lines("a.txt", 2);
    git(
        &fx.work,
        &["remote", "set-url", "origin", "/nonexistent/origin.git"],
    );
    let gh = FakeGh::creating(URL);

    // Act
    let got = create(&fx, &gh, "feat(pr): add a thing", None);

    // Assert
    let err = got.expect_err("the push fails");
    assert!(err.contains("push of feat/x failed"), "got {err}");
    assert_eq!(
        gh.count("pr_create"),
        0,
        "a failed push must never create a PR"
    );
}

#[test]
fn a_missing_body_file_is_rejected_before_pushing() {
    // Arrange
    let fx = Fixture::new("nobody", "feat/x");
    fx.commit_lines("a.txt", 2);
    let gh = FakeGh::creating(URL);

    // Act
    let got = in_dir(&fx.work, || {
        run(&gh, "feat(pr): add a thing", "/nonexistent/body.md", None)
    });

    // Assert
    assert!(got
        .expect_err("no body file")
        .contains("/nonexistent/body.md"));
    assert!(!remote_has_branch(&fx, "feat/x"), "nothing may be pushed");
    assert_eq!(gh.count("pr_create"), 0);
}

#[test]
fn a_pr_opened_against_the_wrong_base_is_corrected_before_returning() {
    // Arrange
    let fx = Fixture::new("drift", "feat/x");
    fx.publish_base_as("release");
    fx.commit_lines("a.txt", 2);
    let gh = FakeGh {
        reported_base: Some("main".into()),
        ..FakeGh::creating(URL)
    };

    // Act
    let got = create(&fx, &gh, "feat(pr): add a thing", Some("release"));

    // Assert
    assert!(got.is_ok(), "{got:?}");
    assert_eq!(gh.count("pr_edit_base:feat/x:release"), 1);
}

#[test]
fn a_pr_already_on_the_right_base_is_not_edited() {
    // Arrange
    let fx = Fixture::new("nodrift", "feat/x");
    fx.commit_lines("a.txt", 2);
    let gh = FakeGh::creating(URL);

    // Act
    create(&fx, &gh, "feat(pr): add a thing", None).expect("creates");

    // Assert
    assert_eq!(gh.count("pr_edit_base"), 0);
}

#[test]
fn a_failed_base_check_after_creation_still_reports_the_url_with_a_warning() {
    // Arrange
    let fx = Fixture::new("viewfail", "feat/x");
    fx.commit_lines("a.txt", 2);
    let gh = FakeGh {
        base_view_error: Some("gh is rate limited".into()),
        ..FakeGh::creating(URL)
    };

    // Act
    let got = create(&fx, &gh, "feat(pr): add a thing", None)
        .expect("the PR exists, so this is not an error");

    // Assert
    assert!(
        got.lines().any(|l| l == format!("PR: {URL}")),
        "the URL must survive: {got}"
    );
    assert!(
        got.contains("WARN: could not verify the PR's base"),
        "got {got}"
    );
    assert_eq!(gh.count("pr_create"), 1);
}

#[test]
fn running_on_the_base_branch_never_pushes_to_it() {
    // Arrange: a local commit on `main` that origin does not have yet.
    let fx = Fixture::new("onbase", "feat/x");
    git(&fx.work, &["checkout", "--quiet", "main"]);
    fx.commit_lines("local-only.txt", 2);
    let before = git(&fx.bare, &["rev-parse", "refs/heads/main"]);
    let gh = FakeGh::creating(URL);

    // Act
    let got = create(&fx, &gh, "feat(pr): add a thing", None);

    // Assert
    let err = got.expect_err("main is the base");
    assert!(err.contains("on the base branch (main)"), "got {err}");
    assert_eq!(
        git(&fx.bare, &["rev-parse", "refs/heads/main"]),
        before,
        "origin/main must not move"
    );
    assert_eq!(gh.count("pr_create"), 0);
}

fn open_pr(url: &str) -> Option<playbook::pr::shared::ExistingPr> {
    Some(playbook::pr::shared::ExistingPr {
        url: url.to_string(),
        state: "OPEN".to_string(),
    })
}

#[test]
fn a_rerun_reuses_the_open_pr_and_still_corrects_its_base() {
    // Arrange: a first run created the PR, then its base fix failed.
    let fx = Fixture::new("rerun", "feat/x");
    fx.publish_base_as("release");
    fx.commit_lines("a.txt", 2);
    let gh = FakeGh {
        existing: open_pr("https://example.test/pr/77"),
        reported_base: Some("main".into()),
        ..FakeGh::creating(URL)
    };

    // Act
    let got = create(&fx, &gh, "feat(pr): add a thing", Some("release")).expect("exits 0");

    // Assert
    assert!(
        got.lines().any(|l| l == "PR: https://example.test/pr/77"),
        "got {got}"
    );
    assert_eq!(gh.count("pr_create"), 0, "the open PR must be reused");
    assert_eq!(gh.count("pr_edit_base:feat/x:release"), 1);
}

#[test]
fn a_merged_pr_on_the_branch_does_not_block_creating_a_new_one() {
    // Arrange
    let fx = Fixture::new("merged", "feat/x");
    fx.commit_lines("a.txt", 2);
    let gh = FakeGh {
        existing: Some(playbook::pr::shared::ExistingPr {
            url: "https://example.test/pr/3".to_string(),
            state: "MERGED".to_string(),
        }),
        ..FakeGh::creating(URL)
    };

    // Act
    let got = create(&fx, &gh, "feat(pr): add a thing", None).expect("creates");

    // Assert
    assert!(got.lines().any(|l| l == format!("PR: {URL}")), "got {got}");
    assert_eq!(gh.count("pr_create"), 1);
}

#[test]
fn a_branch_name_starting_with_a_dash_is_refused_before_anything_is_pushed() {
    // Arrange
    let fx = Fixture::new("dashbranch", "feat/x");
    fx.commit_lines("a.txt", 2);
    fx.checkout_dash_branch("-weird");
    let gh = FakeGh::creating(URL);

    // Act
    let err = create(&fx, &gh, "feat(pr): add a thing", None).expect_err("refused");

    // Assert
    assert!(err.contains("starts with '-'"), "got {err}");
    assert_eq!(gh.count("pr_create"), 0);
    assert!(!remote_has_branch(&fx, "-weird"));
}

#[test]
fn a_base_starting_with_a_dash_is_refused_before_anything_is_pushed() {
    // Arrange
    let fx = Fixture::new("dashbase", "feat/x");
    fx.commit_lines("a.txt", 2);
    let gh = FakeGh::creating(URL);

    // Act
    let err =
        create(&fx, &gh, "feat(pr): add a thing", Some("--upload-pack=x")).expect_err("refused");

    // Assert
    assert!(err.contains("starts with '-'"), "got {err}");
    assert_eq!(gh.count("pr_create"), 0);
    assert!(!remote_has_branch(&fx, "feat/x"));
}

fn create_with_body(fx: &Fixture, gh: &FakeGh, title: &str, text: &str) -> Result<String, String> {
    let path = fx.state.join("guard-body.md");
    fs::create_dir_all(&fx.state).expect("state dir");
    fs::write(&path, text).expect("body");
    let body_file = path.to_str().expect("utf8").to_string();
    in_dir(&fx.work, || run(gh, title, &body_file, None))
}

fn assert_nothing_published(fx: &Fixture, gh: &FakeGh) {
    assert_eq!(gh.count("pr_create"), 0, "no PR may be created");
    assert!(!remote_has_branch(fx, "feat/x"), "nothing may be pushed");
}

#[test]
fn attribution_in_the_body_blocks_the_push_and_names_the_line() {
    // Arrange
    let fx = Fixture::new("attr-body", "feat/x");
    fx.commit_lines("a.txt", 2);
    let gh = FakeGh::creating(URL);

    // Act
    let err = create_with_body(&fx, &gh, "feat: x", "ok\nClaude-Session: https://x\n")
        .expect_err("refused");

    // Assert
    assert_eq!(
        err,
        "refusing to push or create:\nbody line 2 carries AI attribution\n\
         fix: rewrite the title or body, or amend the named commit, before pushing"
    );
    assert_nothing_published(&fx, &gh);
}

#[test]
fn attribution_in_a_commit_message_blocks_the_push_and_names_the_sha() {
    // Arrange
    let fx = Fixture::new("attr-commit", "feat/x");
    fx.commit_lines("a.txt", 2);
    fs::write(fx.work.join("b.txt"), "b\n").expect("write");
    git(&fx.work, &["add", "-A"]);
    git(
        &fx.work,
        &[
            "commit",
            "--quiet",
            "-m",
            "feat: b",
            "-m",
            "Co-Authored-By: Claude <noreply@anthropic.com>",
        ],
    );
    let sha = git(&fx.work, &["rev-parse", "--short", "HEAD"]);
    let gh = FakeGh::creating(URL);

    // Act
    let err = create_with_body(&fx, &gh, "feat: x", "body\n").expect_err("refused");

    // Assert
    assert!(
        err.contains(&format!("commit {sha} carries AI attribution")),
        "got {err}"
    );
    assert_nothing_published(&fx, &gh);
}

#[test]
fn a_dash_in_the_title_blocks_the_push() {
    // Arrange
    let fx = Fixture::new("dash-title", "feat/x");
    fx.commit_lines("a.txt", 2);
    let gh = FakeGh::creating(URL);

    // Act
    let err = create_with_body(&fx, &gh, "feat \u{2014} x", "body\n").expect_err("refused");

    // Assert
    assert_eq!(
        err,
        "refusing to push or create:\nline 0 (title) has a em (U+2014) dash\n\
         fix: replace each dash with a comma, colon, or parentheses"
    );
    assert_nothing_published(&fx, &gh);
}

#[test]
fn dashes_inside_code_do_not_block_a_clean_pr() {
    // Arrange
    let fx = Fixture::new("dash-code", "feat/x");
    fx.commit_lines("a.txt", 2);
    let gh = FakeGh::creating(URL);
    let text = "use `a \u{2013} b`\n```\nx \u{2014} y\n```\n";

    // Act
    let got = create_with_body(&fx, &gh, "feat: x", text);

    // Assert
    assert!(got.is_ok(), "{got:?}");
    assert_eq!(gh.count("pr_create"), 1);
}
