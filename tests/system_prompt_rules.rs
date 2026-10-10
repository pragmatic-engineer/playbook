// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The shipped system prompt follows its own rules, stays small, and does not
//! restate facts that live elsewhere (a model name, an effort variant), which
//! is how it used to contradict `agents/*.md`.

use std::fs;
use std::path::Path;

/// Characters. Loaded on every turn, so growth should be a deliberate edit.
const PROMPT_BUDGET_CHARS: usize = 8000;

fn prompt() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("prompts/SYSTEM_PROMPT.md");
    fs::read_to_string(path).expect("shipped system prompt")
}

#[test]
fn the_prompt_stays_under_its_budget() {
    let len = prompt().chars().count();

    assert!(
        len <= PROMPT_BUDGET_CHARS,
        "{len} chars, budget {PROMPT_BUDGET_CHARS}"
    );
}

#[test]
fn the_prompt_uses_no_em_or_en_dash() {
    let text = prompt();

    assert!(!text.contains('\u{2014}') && !text.contains('\u{2013}'));
}

#[test]
fn the_prompt_never_uses_a_so_consequence_clause() {
    let text = prompt();

    let hits: Vec<&str> = text.lines().filter(|l| l.contains(", so ")).collect();

    assert!(hits.is_empty(), "breaks its own rule: {hits:?}");
}

#[test]
fn the_prompt_does_not_restate_models_or_effort_variants() {
    // The agent files and `playbook route` own these. A copy here drifts.
    let text = prompt().to_lowercase();

    for word in ["opus", "sonnet", "haiku", "-xhigh", "-low", "high effort"] {
        assert!(
            !text.contains(word),
            "restates `{word}`: point at agents/ instead"
        );
    }
}

#[test]
fn the_prompt_points_at_the_files_that_own_routing_and_the_hooks_that_enforce_rules() {
    let text = prompt();

    for needle in [
        "playbook route",
        "playbook:delegating-subagents",
        "policy-guard",
        "commit-message-sanitizer",
        "no-slop-guard",
    ] {
        assert!(text.contains(needle), "missing `{needle}`");
    }
}
