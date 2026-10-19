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

#[test]
fn the_body_names_no_other_command() {
    let body = body();
    let named = regex::Regex::new(r"`playbook ([a-z][a-z-]*)").unwrap();
    for cap in named.captures_iter(&body) {
        assert_eq!(
            &cap[1], "commands",
            "the skill names `playbook {}`",
            &cap[1]
        );
    }
}

/// Runs scripts/check-intents.sh against a stub binary whose `tools map
/// --json` and `commands --json` print the given text (`None`: exit 2).
fn check_with(name: &str, commands: Option<&str>, strict: bool) -> std::process::Output {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("cli-skill-{name}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let map = r#"[{"intent":"pr-reply","group":"pr","verb":"reply"},{"intent":"plans-dir","group":"path","verb":"plans"}]"#;
    let list = match commands {
        Some(json) => format!("echo '{json}'"),
        None => "exit 2".to_string(),
    };
    let stub = format!(
        "#!/bin/sh\ncase \"$1 $2\" in\n  'tools map') echo '{map}' ;;\n  'commands --json') {list} ;;\n  *) echo 'playbook 0.0.0' ;;\nesac\n"
    );
    let bin = dir.join("playbook");
    fs::write(&bin, stub).unwrap();
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    let script = Path::new(env!("PLAYBOOK_REPO_ROOT")).join("scripts/check-intents.sh");
    let mut cmd = std::process::Command::new("bash");
    cmd.arg(script).arg(&bin).env_remove("INTENTS_STRICT");
    if strict {
        cmd.env("INTENTS_STRICT", "1");
    }
    cmd.output().expect("bash runs")
}

const FULL: &str = r#"[{"command":"commands","subcommands":[]},{"command":"path","subcommands":[]},{"command":"pr","subcommands":["reply"]},{"command":"pr reply","subcommands":[]}]"#;

#[test]
fn the_check_passes_when_every_named_command_exists() {
    let out = check_with("pass", Some(FULL), true);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(
        text.contains("PASS every command the tool map and the cli skill name exists"),
        "{text}"
    );
}

#[test]
fn the_check_fails_on_a_missing_subcommand_or_skill_command() {
    let no_reply = r#"[{"command":"commands","subcommands":[]},{"command":"path","subcommands":[]},{"command":"pr","subcommands":["reply"]}]"#;
    let out = check_with("no-reply", Some(no_reply), false);
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("FAIL pr reply is named"), "{text}");
    let no_commands = r#"[{"command":"path","subcommands":[]},{"command":"pr","subcommands":["reply"]},{"command":"pr reply","subcommands":[]}]"#;
    let out = check_with("no-commands", Some(no_commands), false);
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("FAIL commands is named"), "{text}");
}

#[test]
fn a_binary_without_the_list_fails_only_when_strict() {
    let out = check_with("strict", None, true);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("gave no command list"));
    let out = check_with("lenient", None, false);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("has no command list yet"));
}
