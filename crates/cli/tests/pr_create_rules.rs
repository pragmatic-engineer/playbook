// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The PR body, title and commit scope rules of `pr create`, run against real
//! scratch repos and a fake `GhClient`.

mod pr_support;

use playbook::pr::body_rules::{Level, Rules};
use playbook::pr::create::{run_with_rules, Args};
use pr_support::{git, in_dir, FakeGh, Fixture};
use std::fs;

const URL: &str = "https://example.test/pr/42";
const GOOD: &str =
    "Ticket: PROJ-7\n\n## Summary\n\nRetries the sync job when the queue is busy.\n\nCloses #3\n";

struct Case {
    name: &'static str,
    branch: &'static str,
    title: &'static str,
    body: &'static str,
    base_is_default: bool,
    ticket: Option<&'static str>,
    issue: Option<&'static str>,
    scope_level: Level,
    /// `None` means the PR is created; `Some(text)` means refused with it.
    refused_with: Option<&'static str>,
    warns: Option<&'static str>,
}

fn run_case(c: &Case) {
    // Arrange
    let fx = Fixture::new("rules", c.branch);
    fx.commit_lines("a.txt", 2);
    git(&fx.work, &["commit", "--quiet", "--amend", "-m", c.title]);
    let mut gh = FakeGh::creating(URL);
    gh.default_branch = Some(if c.base_is_default { "main" } else { "develop" }.to_string());
    let path = fx.state.join("body.md");
    fs::create_dir_all(&fx.state).expect("state dir");
    fs::write(&path, c.body).expect("body");
    let body_file = path.to_str().expect("utf8").to_string();
    let rules = Rules {
        scope: c.scope_level,
        ..Rules::default()
    };
    let args = Args {
        ticket: c.ticket,
        issue: c.issue,
    };

    // Act
    let got = in_dir(&fx.work, || {
        run_with_rules(&gh, c.title, &body_file, Some("main"), true, &rules, &args)
    });

    // Assert
    match (c.refused_with, got) {
        (Some(text), Err(err)) => {
            assert!(err.contains(text), "{}: got {err}", c.name);
            assert_eq!(gh.count("pr_create"), 0, "{}: nothing is created", c.name);
        }
        (None, Ok(out)) => {
            if let Some(text) = c.warns {
                assert!(out.contains(text), "{}: got {out}", c.name);
            } else {
                assert!(!out.contains("WARN"), "{}: got {out}", c.name);
            }
        }
        (want, got) => panic!("{}: wanted refusal {want:?}, got {got:?}", c.name),
    }
}

fn base() -> Case {
    Case {
        name: "",
        branch: "feat/PROJ-7-retry",
        title: "feat(PROJ-7): add retry to the sync job",
        body: GOOD,
        base_is_default: true,
        ticket: None,
        issue: Some("3"),
        scope_level: Level::Warn,
        refused_with: None,
        warns: None,
    }
}

#[test]
fn a_correct_body_and_title_create_the_pr() {
    run_case(&Case {
        name: "correct",
        ..base()
    });
}

#[test]
fn a_body_that_breaks_the_template_is_refused() {
    run_case(&Case {
        name: "template",
        body: "Ticket: PROJ-7\n\n## Test plan\n\n- run it\n",
        refused_with: Some("## Summary"),
        ..base()
    });
}

#[test]
fn a_missing_ticket_on_the_first_line_is_refused() {
    run_case(&Case {
        name: "first line",
        body: "## Summary\n\nRetries the job.\n\nCloses #3\n",
        refused_with: Some("first line of the PR body must carry the ticket PROJ-7"),
        ..base()
    });
}

#[test]
fn a_missing_issue_clause_only_warns() {
    run_case(&Case {
        name: "issue",
        body: "Ticket: PROJ-7\n\n## Summary\n\nRetries the job.\n",
        warns: Some("must name issue #3"),
        ..base()
    });
}

#[test]
fn a_title_without_the_ticket_warns_when_the_pr_is_not_stacked() {
    run_case(&Case {
        name: "scope warn",
        title: "feat(sync): add retry to the sync job",
        warns: Some("PR title must carry the ticket"),
        ..base()
    });
}

#[test]
fn a_title_without_the_ticket_is_refused_when_the_pr_is_stacked() {
    run_case(&Case {
        name: "scope stacked",
        title: "feat(sync): add retry to the sync job",
        base_is_default: false,
        refused_with: Some("PR title must carry the ticket"),
        ..base()
    });
}

#[test]
fn the_scope_rule_can_be_set_to_block_without_a_stack() {
    run_case(&Case {
        name: "scope block",
        title: "feat(sync): add retry to the sync job",
        scope_level: Level::Block,
        refused_with: Some("PR title must carry the ticket"),
        ..base()
    });
}

#[test]
fn an_untitled_ticket_rule_is_skipped_when_no_ticket_is_known() {
    run_case(&Case {
        name: "no ticket",
        branch: "feat/retry",
        title: "feat(sync): add retry to the sync job",
        body: "## Summary\n\nRetries the job.\n\nCloses #3\n",
        base_is_default: false,
        ..base()
    });
}

#[test]
fn an_explicit_none_turns_the_ticket_rules_off() {
    run_case(&Case {
        name: "none",
        title: "feat(sync): add retry to the sync job",
        body: "## Summary\n\nRetries the job.\n\nCloses #3\n",
        ticket: Some("none"),
        base_is_default: false,
        ..base()
    });
}
