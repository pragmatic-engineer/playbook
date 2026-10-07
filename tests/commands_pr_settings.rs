// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Structural checks that the PR and commit commands read the
//! `autoReview.fix`, `autoMerge.enabled` and `commit.signOff` settings and
//! keep the merge guard rails. They cover prose structure only: model
//! behaviour is not testable here.

use std::fs;
use std::path::PathBuf;

fn command_text(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("commands")
        .join(format!("{name}.md"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The text of the `## Step <n>` section, up to the next `## ` heading.
fn step_section(text: &str, step: &str) -> String {
    let heading = format!("## Step {step}:");
    let mut in_section = false;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with("## ") {
            in_section = line.starts_with(&heading);
        }
        if in_section {
            out.push(line);
        }
    }
    assert!(!out.is_empty(), "no `{heading}` section found");
    out.join("\n")
}

/// The self-review step, which reads the auto-review settings.
fn self_review_step() -> String {
    step_section(&command_text("create-pull-request"), "5")
}

#[test]
fn the_self_review_step_reads_the_fix_and_merge_settings_through_config_get() {
    // Arrange
    let step = self_review_step();

    // Act
    let reads_fix = step.contains("playbook config get autoReview.fix");
    let reads_merge = step.contains("playbook config get autoMerge.enabled");

    // Assert
    assert!(
        reads_fix,
        "Step 5 must run `playbook config get autoReview.fix`"
    );
    assert!(
        reads_merge,
        "Step 5 must run `playbook config get autoMerge.enabled`"
    );
}

#[test]
fn the_merge_flow_marks_ready_waits_for_green_and_merges_without_a_strategy() {
    // Arrange
    let step = self_review_step();

    // Act
    let merge_lines: Vec<&str> = step.lines().filter(|l| l.contains("gh pr merge")).collect();

    // Assert
    assert!(
        step.contains("gh pr ready"),
        "Step 5 must mark the PR ready"
    );
    assert!(
        step.contains("gh pr checks"),
        "Step 5 must wait on the PR checks before merging"
    );
    assert!(!merge_lines.is_empty(), "Step 5 must run `gh pr merge`");
    assert!(
        merge_lines.iter().any(|l| l.contains("--auto")),
        "`gh pr merge` must use --auto"
    );
    assert!(
        merge_lines
            .iter()
            .all(|l| !l.contains("--squash") && !l.contains("--merge") && !l.contains("--rebase")),
        "`gh pr merge` must not pick a strategy"
    );
    assert!(
        step.contains("MERGED"),
        "Step 5 must re-read the PR state until it reads MERGED"
    );
}

#[test]
fn the_merge_flow_never_bypasses_checks_or_deletes_the_branch() {
    // Arrange
    let text = command_text("create-pull-request");
    let flags = ["--admin", "--delete-branch"];

    for flag in flags {
        // Act: every line naming the flag must be one that forbids it.
        let mentions: Vec<&str> = text.lines().filter(|l| l.contains(flag)).collect();
        let permitted: Vec<&&str> = mentions
            .iter()
            .filter(|l| !l.to_lowercase().contains("never"))
            .collect();

        // Assert
        assert!(!mentions.is_empty(), "the file must forbid {flag} by name");
        assert!(
            permitted.is_empty(),
            "{flag} used without a ban: {permitted:?}"
        );
    }
}

#[test]
fn the_merge_flow_holds_a_stacked_pr_until_its_base_merges() {
    // Arrange
    let step = self_review_step().to_lowercase();

    // Act
    let handles_stack = step.contains("stacked") && step.contains("base");

    // Assert
    assert!(
        handles_stack,
        "Step 5 must stop before merging a PR whose base is another open PR"
    );
}

#[test]
fn commit_and_push_reads_the_sign_off_setting_and_names_the_stand_down_cases() {
    // Arrange
    let text = command_text("commit-and-push");

    // Act
    let step4 = step_section(&text, "4");

    // Assert
    assert!(
        text.contains("playbook config get commit.signOff"),
        "commit-and-push must read `commit.signOff`"
    );
    for case in ["--no-signoff", "prepare-commit-msg", "commit-msg"] {
        assert!(
            step4.contains(case),
            "Step 4 must name the {case} stand-down"
        );
    }
    assert!(
        step4.contains("--gpg-sign"),
        "cryptographic signing must stay on regardless of commit.signOff"
    );
}
