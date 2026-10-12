// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Pins the fresh-subagent rule: no dispatch relies on an omitted
//! `subagent_type` (which forks the whole parent history), and the brief
//! template stays present in the delegating-subagents skill.

use std::fs;
use std::path::PathBuf;

fn read(rel: &str) -> String {
    fs::read_to_string(PathBuf::from(env!("PLAYBOOK_REPO_ROOT")).join(rel))
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn skill_has_the_fresh_section_and_brief_template() {
    let s = read("skills/delegating-subagents/SKILL.md");
    assert!(s.contains("## Fresh, not forked"));
    for key in ["Goal:", "Inputs:", "Output:", "Constraints:"] {
        assert!(s.contains(key), "brief template lacks {key}");
    }
    assert!(s.contains("references/briefs.md"));
}

#[test]
fn briefs_reference_exists_and_has_toc() {
    let s = read("skills/delegating-subagents/references/briefs.md");
    assert!(s.contains("## Contents"));
    assert!(s.contains("## Classification of spawn sites"));
}

#[test]
fn system_prompt_requires_subagent_type_for_fan_out() {
    let s = read("prompts/SYSTEM_PROMPT.md");
    let line = s
        .lines()
        .find(|l| l.starts_with("**Parallel work**"))
        .expect("parallel work line");
    assert!(line.contains("subagent_type"));
    assert!(line.contains("brief"));
}

#[test]
fn plan_and_implement_name_the_agent_type() {
    let plan = read("commands/plan.md");
    assert!(plan.contains("`subagent_type: Explore`"));
    assert!(plan.contains("`subagent_type: general-purpose`"));
    let imp = read("commands/implement.md");
    assert!(imp.contains("fresh `playbook:implementer` subagent"));
}

#[test]
fn forked_commands_use_a_prefixed_agent() {
    for cmd in ["commit-and-push", "create-pull-request"] {
        let s = read(&format!("commands/{cmd}.md"));
        let agent = s
            .lines()
            .find(|l| l.starts_with("agent:"))
            .unwrap_or_else(|| panic!("{cmd}: no agent line"));
        assert!(agent.contains("playbook:"), "{cmd}: {agent}");
    }
}

#[test]
fn review_brief_builder_keeps_its_four_parts() {
    let s = read("skills/delegating-subagents/references/briefs.md");
    assert!(s.contains("## Review brief builder"));
    assert!(s.contains("## Implementer brief builder"));
    for part in [
        "**Diff.**",
        "**Excerpts.**",
        "**Test command.**",
        "**Limit.**",
    ] {
        assert!(s.contains(part), "review brief builder lacks {part}");
    }
    assert!(s.contains("Read or search beyond it only when a claim cannot be settled"));
}

#[test]
fn dispatching_commands_name_the_brief_builder() {
    for cmd in ["deep-review", "quick-review", "implement"] {
        let s = read(&format!("commands/{cmd}.md"));
        assert!(
            s.contains("Review brief builder"),
            "commands/{cmd}.md does not point at the review brief builder"
        );
    }
    let imp = read("commands/implement.md");
    assert!(imp.contains("Implementer brief builder"));
}

#[test]
fn reviewer_and_implementer_agents_allow_reading_beyond_the_brief_only_when_needed() {
    let r = read("agents/reviewer.md");
    assert!(r.contains("do not Read a file the excerpts already cover"));
    // The lens scope guardrail must survive the change.
    assert!(r.contains("Stay within the assigned focus"));
    let i = read("agents/implementer.md");
    assert!(i.contains("do not explore the repository"));
    assert!(i.contains("Read every file before you modify it"));
}

#[test]
fn every_returning_agent_carries_the_return_cap() {
    for agent in [
        "reviewer",
        "critic",
        "fact-checker",
        "test-reviewer",
        "analyst",
        "cheap-checker",
        "implementer",
        "collector",
    ] {
        let s = read(&format!("agents/{agent}.md"));
        assert!(
            s.contains("about 300 words"),
            "agents/{agent}.md has no return cap"
        );
    }
}

#[test]
fn the_cap_never_applies_to_the_output_file_or_drops_findings() {
    let r = read("agents/reviewer.md");
    assert!(r.contains("never drop a finding to fit"));
    assert!(r.contains("does not apply to a report file you write"));
    let skill = read("skills/delegating-subagents/SKILL.md");
    assert!(skill
        .contains("Return: status, findings as file:line, output path; at most about 300 words"));
    assert!(skill.contains("Write your full report to <absolute path>."));
    let sp = read("prompts/SYSTEM_PROMPT.md");
    assert!(sp.contains("about 300 words"));
}
