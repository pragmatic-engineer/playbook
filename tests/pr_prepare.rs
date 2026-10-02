// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! `pr prepare` against real scratch git repos with a local bare `origin`,
//! and a fake `GhClient` so no test needs GitHub. `prepare` runs git in the
//! process cwd, so every test serialises on it like `tests/gate_record.rs`.

mod pr_support;

use playbook::pr::prepare::prepare;
use playbook::pr::shared::ExistingPr;
use pr_support::{git, in_dir, FakeGh, Fixture};
use std::fs;

trait RunPrepare {
    fn run(&self, gh: &FakeGh, base: Option<&str>, ticket: Option<&str>) -> Result<String, String>;
}

impl RunPrepare for Fixture {
    fn run(&self, gh: &FakeGh, base: Option<&str>, ticket: Option<&str>) -> Result<String, String> {
        let branch = self.branch();
        in_dir(&self.work, || {
            prepare(&self.state, &branch, gh, base, ticket)
        })
    }
}

fn has_line(out: &str, line: &str) -> bool {
    out.lines().any(|l| l == line)
}

const OVER_NOTE: &str = "needs explicit justification in the PR body";

#[test]
fn an_open_pr_returns_ok_with_the_redirect_and_runs_no_further_checks() {
    // Arrange
    let fx = Fixture::new("existing", "feat/x");
    fx.commit_lines("a.txt", 3);
    let gh = FakeGh {
        existing: Some(ExistingPr {
            url: "https://example.test/pr/9".into(),
            state: "OPEN".into(),
        }),
        ..FakeGh::default()
    };

    // Act
    let got = fx
        .run(&gh, None, None)
        .expect("an existing PR is a success path");

    // Assert
    assert_eq!(
        got,
        "A PR already exists: https://example.test/pr/9\n\
         Use /playbook:address-pr-comments or /playbook:quick-review instead."
    );
    assert!(
        gh.count("repo_default_branch") == 0,
        "no further checks may run"
    );
}

#[test]
fn a_closed_pr_does_not_trigger_the_redirect() {
    // Arrange
    let fx = Fixture::new("closed", "feat/x");
    fx.commit_lines("a.txt", 3);
    let gh = FakeGh {
        existing: Some(ExistingPr {
            url: "https://example.test/pr/9".into(),
            state: "MERGED".into(),
        }),
        ..FakeGh::default()
    };

    // Act
    let got = fx.run(&gh, None, None).expect("proceeds");

    // Assert
    assert!(has_line(&got, "branch=feat/x"), "got {got}");
}

#[test]
fn it_fetches_the_base_itself_so_a_stale_local_ref_cannot_hide_that_nothing_is_ahead() {
    // Arrange: the branch looks one commit ahead of the locally cached
    // `origin/main`, but origin's `main` already contains that commit.
    let fx = Fixture::new("stale", "feat/x");
    fx.commit_lines("a.txt", 3);
    git(&fx.work, &["push", "--quiet", "origin", "feat/x"]);
    git(
        &fx.bare,
        &["update-ref", "refs/heads/main", "refs/heads/feat/x"],
    );

    // Act
    let got = fx.run(&FakeGh::default(), None, None);

    // Assert
    let err = got.expect_err("after a fresh fetch nothing is ahead");
    assert!(err.contains("nothing ahead of main"), "got {err}");
}

#[test]
fn the_base_flag_wins_and_the_repo_default_is_never_asked() {
    // Arrange
    let fx = Fixture::new("flag", "feat/x");
    fx.publish_base_as("release");
    fx.commit_lines("a.txt", 3);
    let gh = FakeGh {
        default_branch: Some("develop".into()),
        ..FakeGh::default()
    };

    // Act
    let got = fx.run(&gh, Some("release"), None).expect("prepares");

    // Assert
    assert!(
        has_line(&got, "base=release (source: --base flag)"),
        "got {got}"
    );
    assert!(gh.count("repo_default_branch") == 0);
}

#[test]
fn the_repo_default_wins_over_symbolic_ref_when_no_flag_is_given() {
    // Arrange
    let fx = Fixture::new("default", "feat/x");
    fx.publish_base_as("develop");
    fx.publish_base_as("trunk");
    git(
        &fx.work,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    );
    fx.commit_lines("a.txt", 3);
    let gh = FakeGh {
        default_branch: Some("develop".into()),
        ..FakeGh::default()
    };

    // Act
    let got = fx.run(&gh, None, None).expect("prepares");

    // Assert
    assert!(
        has_line(&got, "base=develop (source: repo default)"),
        "got {got}"
    );
}

#[test]
fn symbolic_ref_wins_over_the_main_fallback_when_the_repo_default_is_unavailable() {
    // Arrange
    let fx = Fixture::new("symref", "feat/x");
    fx.publish_base_as("trunk");
    git(
        &fx.work,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    );
    fx.commit_lines("a.txt", 3);

    // Act
    let got = fx.run(&FakeGh::default(), None, None).expect("prepares");

    // Assert
    assert!(
        has_line(&got, "base=trunk (source: git symbolic-ref)"),
        "got {got}"
    );
}

#[test]
fn main_is_the_fallback_only_when_every_earlier_tier_is_unavailable() {
    // Arrange
    let fx = Fixture::new("fallback", "feat/x");
    fx.commit_lines("a.txt", 3);

    // Act
    let got = fx.run(&FakeGh::default(), None, None).expect("prepares");

    // Assert
    assert!(has_line(&got, "base=main (source: fallback)"), "got {got}");
}

#[test]
fn being_on_the_base_branch_is_a_hard_abort_naming_it() {
    // Arrange
    let fx = Fixture::new("onbase", "feat/x");
    git(&fx.work, &["checkout", "--quiet", "main"]);

    // Act
    let got = fx.run(&FakeGh::default(), None, None);

    // Assert
    let err = got.expect_err("on the base branch");
    assert!(err.contains("on the base branch (main)"), "got {err}");
}

#[test]
fn zero_commits_ahead_is_a_hard_abort() {
    // Arrange
    let fx = Fixture::new("zero", "feat/x");

    // Act
    let got = fx.run(&FakeGh::default(), None, None);

    // Assert
    let err = got.expect_err("nothing to PR");
    assert!(err.contains("nothing ahead of main"), "got {err}");
}

#[test]
fn size_verdicts_pin_every_threshold_boundary() {
    // Arrange
    let soft =
        |n: usize| format!("VERDICT size: SOFT - {n} lines is above the 500-line soft limit");
    let over = |n: usize| {
        format!(
            "VERDICT size: OVER - {n} lines is above the 1000-line enforced limit and {OVER_NOTE}"
        )
    };
    let cases: Vec<(usize, String)> = vec![
        (500, "VERDICT size: OK - 500 lines".to_string()),
        (501, soft(501)),
        (1000, soft(1000)),
        (1001, over(1001)),
        (1500, over(1500)),
    ];

    for (lines, expected) in cases {
        // Act
        let fx = Fixture::new(&format!("size{lines}"), "feat/x");
        fx.commit_lines("data.txt", lines);
        let got = fx
            .run(&FakeGh::default(), None, None)
            .expect("within the hard limit");

        // Assert
        assert!(
            has_line(&got, &expected),
            "{lines} lines: expected {expected:?} in\n{got}"
        );
    }
}

#[test]
fn more_than_1500_lines_is_a_hard_abort_naming_the_limit() {
    // Arrange
    let fx = Fixture::new("size1501", "feat/x");
    fx.commit_lines("data.txt", 1501);

    // Act
    let got = fx.run(&FakeGh::default(), None, None);

    // Assert
    let err = got.expect_err("over the hard limit");
    assert!(err.contains("1500-line hard size limit"), "got {err}");
}

#[test]
fn a_clean_tree_reports_ok_and_a_dirty_one_counts_the_files() {
    // Arrange
    let clean = Fixture::new("clean", "feat/x");
    clean.commit_lines("a.txt", 2);
    let dirty = Fixture::new("dirty", "feat/x");
    dirty.commit_lines("a.txt", 2);
    fs::write(dirty.work.join("u1.txt"), "x").expect("u1");
    fs::write(dirty.work.join("u2.txt"), "x").expect("u2");

    // Act
    let clean_out = clean.run(&FakeGh::default(), None, None).expect("clean");
    let dirty_out = dirty.run(&FakeGh::default(), None, None).expect("dirty");

    // Assert
    assert!(has_line(
        &clean_out,
        "VERDICT dirty: OK - nothing uncommitted"
    ));
    assert!(has_line(
        &dirty_out,
        "VERDICT dirty: WARN - 2 uncommitted file(s) will NOT be in the PR"
    ));
}

#[test]
fn test_verdicts_cover_a_test_file_inline_tests_and_none() {
    // Arrange
    let none = Fixture::new("tnone", "feat/x");
    none.commit_lines("data.txt", 2);
    let file = Fixture::new("tfile", "feat/x");
    file.commit_file("tests/foo.rs", "fn main() {}\n");
    let inline = Fixture::new("tinline", "feat/x");
    inline.commit_file("src/lib.rs", "#[test]\nfn it_works() {}\n");

    // Act
    let none_out = none.run(&FakeGh::default(), None, None).expect("none");
    let file_out = file.run(&FakeGh::default(), None, None).expect("file");
    let inline_out = inline.run(&FakeGh::default(), None, None).expect("inline");

    // Assert
    assert!(has_line(
        &none_out,
        "VERDICT tests: NONE - the diff adds no test files or inline test blocks; the readiness criteria expect tests for behaviour changes"
    ));
    assert!(has_line(
        &file_out,
        "VERDICT tests: OK - 1 test file(s) or inline test block(s) touched"
    ));
    assert!(has_line(
        &inline_out,
        "VERDICT tests: OK - 1 test file(s) or inline test block(s) touched"
    ));
}

#[test]
fn the_ticket_comes_from_the_flag_then_the_branch_and_the_line_is_never_omitted() {
    // Arrange
    let from_branch = Fixture::new("tk1", "feat/PROJ-1234-thing");
    from_branch.commit_lines("a.txt", 2);
    let from_flag = Fixture::new("tk2", "feat/PROJ-1234-thing");
    from_flag.commit_lines("a.txt", 2);
    let none = Fixture::new("tk3", "feat/plain");
    none.commit_lines("a.txt", 2);

    // Act
    let branch_out = from_branch
        .run(&FakeGh::default(), None, None)
        .expect("branch");
    let flag_out = from_flag
        .run(&FakeGh::default(), None, Some("ABC-9"))
        .expect("flag");
    let none_out = none.run(&FakeGh::default(), None, None).expect("none");

    // Assert
    assert_eq!(branch_out.lines().last(), Some("ticket=PROJ-1234"));
    assert_eq!(flag_out.lines().last(), Some("ticket=ABC-9"));
    assert_eq!(none_out.lines().last(), Some("ticket="));
}

#[test]
fn the_diff_is_written_to_the_state_dir_and_its_path_is_printed() {
    // Arrange
    let fx = Fixture::new("difffile", "feat/x");
    fx.commit_file("a.txt", "hello\n");

    // Act
    let got = fx.run(&FakeGh::default(), None, None).expect("prepares");

    // Assert
    let expected_path = fx.state.join("pr-diff.txt");
    assert!(
        has_line(&got, &format!("diff_file={}", expected_path.display())),
        "got {got}"
    );
    let diff = fs::read_to_string(&expected_path).expect("diff file exists");
    assert!(diff.contains("+hello"), "diff was {diff}");
}

#[test]
fn an_unknown_base_names_the_missing_remote_ref_instead_of_claiming_nothing_is_ahead() {
    // Arrange
    let fx = Fixture::new("nobase", "feat/x");
    fx.commit_lines("a.txt", 2);

    // Act
    let got = fx.run(&FakeGh::default(), Some("no-such-branch"), None);

    // Assert
    let err = got.expect_err("the base does not exist on origin");
    assert!(err.contains("origin/no-such-branch"), "got {err}");
    assert!(!err.contains("nothing ahead"), "got {err}");
}
