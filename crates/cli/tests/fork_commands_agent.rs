// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A forked command picks its agent with `agent:` in its frontmatter. A plugin
//! agent only resolves with the `playbook:` prefix; a bare name falls back to a
//! general purpose agent on the session model, which ran every commit and pull
//! request on Sonnet or Opus instead of the Haiku `git` agent.

use std::fs;

fn frontmatter_value(file: &str, key: &str) -> Option<String> {
    let text = fs::read_to_string(file).expect("command file");
    text.lines()
        .skip(1)
        .take_while(|line| *line != "---")
        .find_map(|line| line.strip_prefix(&format!("{key}: ")))
        .map(|value| value.trim().to_string())
}

#[test]
fn commit_and_push_and_create_pull_request_fork_into_the_prefixed_git_agent() {
    for file in [
        "commands/commit-and-push.md",
        "commands/create-pull-request.md",
    ] {
        assert_eq!(
            frontmatter_value(file, "context").as_deref(),
            Some("fork"),
            "{file}"
        );
        assert_eq!(
            frontmatter_value(file, "agent").as_deref(),
            Some("playbook:git"),
            "{file}"
        );
    }
}

#[test]
fn the_git_agent_runs_on_haiku() {
    assert_eq!(
        frontmatter_value("agents/git.md", "model").as_deref(),
        Some("haiku")
    );
}

#[test]
fn repo_audit_forks_into_the_prefixed_auditor_agent() {
    let file = "commands/repo-audit.md";
    assert_eq!(
        frontmatter_value(file, "context").as_deref(),
        Some("fork"),
        "{file}"
    );
    assert_eq!(
        frontmatter_value(file, "agent").as_deref(),
        Some("playbook:auditor"),
        "{file}"
    );
}

#[test]
fn the_auditor_agent_runs_on_opus_at_high_effort() {
    assert_eq!(
        frontmatter_value("agents/auditor.md", "model").as_deref(),
        Some("opus")
    );
    assert_eq!(
        frontmatter_value("agents/auditor.md", "effort").as_deref(),
        Some("high")
    );
}

/// A forked command that re-invokes itself through the Skill tool starts a
/// second fork and wastes a turn (78 cases in 16 sessions). The first line of
/// the body, right under the title, tells the reader it is already the
/// executing subagent.
#[test]
fn every_forked_command_opens_with_the_do_not_reinvoke_note() {
    let mut checked = 0;
    for entry in fs::read_dir("commands").expect("commands dir") {
        let path = entry.expect("entry").path();
        let text = fs::read_to_string(&path).expect("command file");
        if !text.lines().any(|l| l == "context: fork") {
            continue;
        }
        let mut body = text.lines().skip_while(|l| !l.starts_with("# ")).skip(1);
        let first = body.find(|l| !l.trim().is_empty()).unwrap_or("");
        assert!(
            first.contains("already the executing subagent") && first.contains("Skill tool"),
            "{}: first body line must tell the reader not to call the Skill tool again, got: {first}",
            path.display()
        );
        checked += 1;
    }
    assert!(
        checked >= 3,
        "expected at least 3 forked commands, saw {checked}"
    );
}
