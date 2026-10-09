// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A command that can spawn agents reads the user's effort ceiling in Step 0,
//! so the agents it picks stay within `maxEffortLevel` and the per-component
//! keys (ADR-0017). The check parses the Step 0 section of each command file.

use std::fs;
use std::path::PathBuf;

fn commands_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("commands")
}

/// The `allowed-tools:` frontmatter line, or an empty string.
fn allowed_tools(text: &str) -> &str {
    text.lines()
        .take_while(|l| !l.is_empty() || true)
        .skip(1)
        .take_while(|l| *l != "---")
        .find_map(|l| l.strip_prefix("allowed-tools:"))
        .unwrap_or("")
}

/// The body of the `Step 0` section: from its heading to the next heading of
/// the same or a higher level.
fn step_zero(text: &str) -> Option<String> {
    let mut lines = text.lines();
    let level = loop {
        let line = lines.next()?;
        if let Some(rest) = line.strip_prefix('#') {
            let hashes = 1 + rest.chars().take_while(|c| *c == '#').count();
            if line
                .trim_start_matches('#')
                .trim_start()
                .starts_with("Step 0")
            {
                break hashes;
            }
        }
    };
    let mut body = String::new();
    for line in lines {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if hashes > 0 && hashes <= level && line.chars().nth(hashes) == Some(' ') {
            break;
        }
        body.push_str(line);
        body.push('\n');
    }
    Some(body)
}

#[test]
fn every_command_that_spawns_agents_reads_the_effort_ceiling_in_step_zero() {
    let mut checked = 0;
    for entry in fs::read_dir(commands_dir()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&path).unwrap();
        if !allowed_tools(&text).split(',').any(|t| t.trim() == "Agent") {
            continue;
        }
        let step = step_zero(&text).unwrap_or_else(|| panic!("{name}: no Step 0 section"));
        let call = format!("playbook effort resolve commands {name} --json");
        assert!(step.contains(&call), "{name}: Step 0 must run `{call}`");
        assert!(
            step.contains("delegating-subagents"),
            "{name}: Step 0 must point at the delegating-subagents skill"
        );
        checked += 1;
    }
    assert!(
        checked >= 7,
        "found only {checked} commands that spawn agents"
    );
}

#[test]
fn the_resolved_names_are_real_components() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for name in [
        "address-pr-comments",
        "adr",
        "deep-review",
        "implement",
        "learn-project",
        "plan",
        "quick-review",
    ] {
        assert!(
            root.join("commands").join(format!("{name}.md")).is_file(),
            "{name}"
        );
    }
}
