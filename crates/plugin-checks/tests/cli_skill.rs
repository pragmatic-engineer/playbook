// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The `playbook:cli` skill holds no list of commands. Its body is one line
//! that sends a session to `playbook commands`, which the binary builds from
//! its own command tree, so the skill cannot drift. `scripts/check-intents.sh`
//! checks against a built binary that the commands it names exist.

use std::fs;
use std::path::Path;

fn body() -> String {
    let path = Path::new(env!("PLAYBOOK_PLUGIN_ROOT")).join("skills/cli/SKILL.md");
    let text = fs::read_to_string(path).expect("skills/cli/SKILL.md");
    let rest = text.strip_prefix("---\n").expect("frontmatter");
    let end = rest.find("\n---\n").expect("closing ---");
    rest[end + 5..].trim().to_string()
}

#[test]
fn the_body_is_one_line_that_points_to_playbook_commands() {
    let body = body();
    assert_eq!(
        body.lines().count(),
        1,
        "the body must stay one line: {body}"
    );
    for needle in [
        "`playbook commands`",
        "`playbook commands <group>`",
        "user-only",
        "with `!`",
    ] {
        assert!(body.contains(needle), "the body must name {needle}: {body}");
    }
}
