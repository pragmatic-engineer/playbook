// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Pins the stack rules for both review commands: they detect the stack, say
//! which PRs are below and above, and review only the PR's own diff against
//! its real base unless the whole stack is asked for.

use std::fs;
use std::path::PathBuf;

fn read(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn both_commands_run_the_stack_check() {
    for name in ["quick-review", "deep-review"] {
        let text = read(&format!("commands/{name}.md"));
        assert!(
            text.contains("## Step 1b: Stack check"),
            "{name}: no stack step"
        );
        assert!(
            text.contains("playbook skill ref grounding-review stack-review"),
            "{name}: stack reference not loaded"
        );
    }
}

#[test]
fn the_stack_reference_names_neighbours_and_the_real_base() {
    let text = read("skills/grounding-review/references/stack-review.md");
    assert!(text.contains("playbook pr stack"));
    assert!(text.contains("Below: "), "no below line");
    assert!(text.contains("Above: "), "no above line");
    assert!(text.contains("Own diff base: <base>"), "no base line");
    assert!(
        text.contains("the parent PR's branch, not the default branch"),
        "base is not the parent branch"
    );
}

#[test]
fn this_pr_only_is_the_default_and_the_auto_choice() {
    let text = read("skills/grounding-review/references/stack-review.md");
    assert!(text.contains("1. This PR only. This is the default."));
    assert!(text.contains("Assumed: this PR only (auto mode)."));
    assert!(text.contains("never the default branch"));
    assert!(!text.contains("Assumed: whole stack"));
}
