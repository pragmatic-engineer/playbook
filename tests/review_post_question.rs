// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Pins the rule that both review commands ask what to post. Only an
//! explicit `--self` or `--no-post`, or auto mode (which prints the post
//! command instead), skips the question. Naming no PR or reviewing your own
//! PR must not skip it.

use std::fs;
use std::path::PathBuf;

fn read(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("commands")
        .join(format!("{name}.md"));
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

const COMMANDS: [&str; 2] = ["quick-review", "deep-review"];

#[test]
fn both_commands_name_no_post_and_ask_what_to_post() {
    for name in COMMANDS {
        let text = read(name);
        assert!(text.contains("--no-post"), "{name}: no --no-post flag");
        assert!(
            text.contains("Post which findings as a pending review?"),
            "{name}: lost the post question"
        );
    }
}

#[test]
fn auto_prints_the_post_command_and_never_posts() {
    for name in COMMANDS {
        let text = read(name);
        let line = format!("To post: /playbook:{name} <PR_NUMBER> --ask");
        assert!(text.contains(&line), "{name}: no `{line}`");
        assert!(
            text.contains("Auto mode never posts"),
            "{name}: auto rule missing"
        );
    }
}

#[test]
fn no_pr_number_and_own_pr_do_not_skip_the_question() {
    for name in COMMANDS {
        let text = read(name);
        for stale in [
            "nothing was named to post to",
            "no PR number/branch was given",
            "or the resolved PR is authored by",
            "never posts either, even with a PR number",
        ] {
            assert!(!text.contains(stale), "{name}: still says `{stale}`");
        }
        assert!(
            text.contains("offer only `comment` and `skip`"),
            "{name}: own-PR verbs not narrowed"
        );
    }
}
