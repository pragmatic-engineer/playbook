// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Structural checks that the PR and commit commands read the
//! `autoReview.fix`, `autoMerge.enabled` and `commit.signOff` settings and
//! keep the merge guard rails. They cover prose structure only: model
//! behaviour is not testable here.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

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
fn the_merge_flow_marks_ready_polls_checks_to_a_deadline_and_pins_the_head_commit() {
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
        step.contains("gh pr checks <n> --json name,bucket"),
        "Step 5 must poll `gh pr checks --json name,bucket`"
    );
    assert!(
        !step.contains("--watch"),
        "Step 5 must not use the unbounded `gh pr checks --watch`"
    );
    for needle in [
        "every 30 seconds",
        "45 minutes",
        "own short Bash call",
        "`skipping`",
        "`cancel`",
        "`pending`",
        "two minutes after `gh pr ready`",
    ] {
        assert!(step.contains(needle), "Step 5 must say `{needle}`");
    }
    let line = merge_lines
        .iter()
        .find(|l| l.contains("--auto"))
        .expect("`gh pr merge` must use --auto");
    let start = line.find("gh pr merge").unwrap();
    let primary = &line[start..start + line[start..].find('`').unwrap()];
    assert!(
        primary.contains("--match-head-commit <recorded headRefOid>"),
        "`gh pr merge` must pin the recorded head commit"
    );
    assert!(
        !primary.contains("--squash")
            && !primary.contains("--merge ")
            && !primary.contains("--rebase"),
        "the first `gh pr merge` must not pick a method"
    );
    assert!(
        step.contains("squashMergeAllowed,mergeCommitAllowed,rebaseMergeAllowed"),
        "Step 5 must read the allowed merge methods when gh requires one"
    );
}

#[test]
fn the_merge_flow_reports_which_outcome_the_post_merge_read_shows() {
    // Arrange
    let step = self_review_step();

    // Act
    let reads_both = step.contains("gh pr view <n> --json state,autoMergeRequest");

    // Assert
    assert!(reads_both, "Step 5 must read state and autoMergeRequest");
    for case in [
        "`MERGED`",
        "auto-merge armed",
        "auto-merge was cleared",
        "`CLOSED`",
    ] {
        assert!(step.contains(case), "Step 5 must name the {case} case");
    }
}

#[test]
fn auto_mode_with_no_review_marks_ready_but_never_merges() {
    // Arrange
    let step = self_review_step();

    // Act
    let line = step
        .lines()
        .find(|l| l.contains("**Reviewed:**"))
        .expect("Step 5 must have a Reviewed gate");

    // Assert
    assert!(line.contains("auto mode"), "the gate must name auto mode");
    assert!(
        line.contains("do not merge"),
        "the gate must refuse to merge"
    );
    assert!(
        line.contains("`/playbook:implement`"),
        "the gate must exempt only /playbook:implement"
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

/// The `hook_writes_signoff` shell function from commit-and-push Step 4.
fn hook_function() -> String {
    let text = step_section(&command_text("commit-and-push"), "4");
    let start = text
        .find("hook_writes_signoff() {")
        .expect("Step 4 must define hook_writes_signoff");
    let end = text[start..]
        .find("\n}\n")
        .expect("hook_writes_signoff must close with a bare brace");
    text[start..start + end + 3].to_string()
}

/// Run the extracted function against a hook script; true when it counts the
/// hook as writing the trailer.
fn hook_counts_as_signing_off(script: &str) -> bool {
    let dir = std::env::temp_dir().join(format!(
        "playbook-hook-{}-{}",
        std::process::id(),
        script.len()
    ));
    fs::create_dir_all(&dir).unwrap();
    let hook = dir.join("hook");
    fs::write(&hook, script).unwrap();
    let status = Command::new("bash")
        .arg("-c")
        .arg(format!(
            "{}\nHOOK_FILE=\"$1\"\nhook_writes_signoff",
            hook_function()
        ))
        .arg("bash")
        .arg(&hook)
        .status()
        .expect("bash must run");
    fs::remove_dir_all(&dir).ok();
    status.success()
}

#[test]
fn a_hook_that_only_checks_for_the_trailer_does_not_stand_the_flag_down() {
    // Arrange
    let checks = [
        "#!/bin/sh\ngrep -qi '^Signed-off-by:' \"$1\" || { echo 'add one'; exit 1; }\n",
        "#!/bin/sh\nif ! grep -qiE '^Signed-off-by:' \"$1\"; then\n  echo 'Run git commit --signoff' >&2\n  exit 1\nfi\n",
        "#!/bin/sh\n# this hook would run git interpret-trailers --trailer Signed-off-by\ngrep -q DCO \"$1\"\n",
    ];

    for script in checks {
        // Act
        let counted = hook_counts_as_signing_off(script);

        // Assert
        assert!(!counted, "a check-only hook must not count: {script}");
    }
}

#[test]
fn a_hook_that_writes_the_trailer_stands_the_flag_down() {
    // Arrange
    let writers = [
        "#!/bin/sh\ngit interpret-trailers --in-place --trailer \"Signed-off-by: $NAME <$MAIL>\" \"$1\"\n",
        "#!/bin/sh\ngit interpret-trailers --in-place \\\n  --trailer 'Signed-off-by: a' \"$1\"\n",
        "#!/bin/sh\nprintf '\\nSigned-off-by: %s\\n' \"$NAME\" >> \"$1\"\n",
        "#!/bin/sh\nexec git commit --amend --no-edit --signoff\n",
    ];

    for script in writers {
        // Act
        let counted = hook_counts_as_signing_off(script);

        // Assert
        assert!(counted, "a writing hook must count: {script}");
    }
}

#[test]
fn commit_and_push_signs_when_a_key_is_set_and_the_docs_say_so() {
    // Arrange
    let step4 = step_section(&command_text("commit-and-push"), "4");
    let guide = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/guides/02-review-and-pr-flow.md"),
    )
    .unwrap();

    // Assert
    assert!(step4.contains("git commit ${AMEND_FLAG} ${SIGNOFF_FLAG} ${GPG_FLAG}"));
    assert!(step4.contains("user.signingkey"));
    assert!(guide.contains("signed whenever"));
    assert!(!guide.contains("already passes `-s`"));
}

#[test]
fn the_create_pull_request_command_documents_the_pr_draft_setting() {
    // Arrange
    let text = command_text("create-pull-request");

    // Act
    let step4 = step_section(&text, "4");

    // Assert
    assert!(step4.contains("pr.draft"), "Step 4 must name `pr.draft`");
    assert!(
        !step4.contains("unconditionally"),
        "Step 4 must not claim the draft is unconditional"
    );
}

#[test]
fn the_self_review_step_runs_review_triage_for_auto_and_falls_back_to_deep() {
    // Arrange
    let step = self_review_step();

    // Act
    let runs_triage = step.contains("playbook pr review-triage");
    let fails_to_deep = step.contains("a triage failure never skips or lightens the review");

    // Assert
    assert!(runs_triage, "Step 5 must run `playbook pr review-triage`");
    assert!(
        fails_to_deep,
        "Step 5 must resolve a triage failure to deep"
    );
}
