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
    !git(&fx.bare, &["branch", "--list", branch]).is_empty()
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
