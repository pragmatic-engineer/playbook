// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Pins the fresh-subagent rule: no dispatch relies on an omitted
//! `subagent_type` (which forks the whole parent history), and the brief
//! template stays present in the delegating-subagents skill.

use std::fs;
use std::path::PathBuf;

fn read(rel: &str) -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel))
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
