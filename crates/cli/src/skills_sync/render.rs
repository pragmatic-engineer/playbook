// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Turns playbook skill and command text into skills Codex can load. Pure
//! functions over text: nothing here reads or writes a file.
//!
//! `known` is the set of bare plugin names (skill folder names and command
//! file names, without the `playbook-` prefix). `name` arguments are the
//! full target names, such as `playbook-fix`.

use super::plan::NAME_PREFIX;
use std::collections::BTreeSet;

/// Codex truncates a skill's main prompt at this many bytes on one of its
/// read paths, so a longer `SKILL.md` is split into a stub and a body file.
pub const MAX_SKILL_PROMPT_BYTES: usize = 8000;
/// Where the body of a split skill is written, relative to the skill folder.
pub const BODY_PATH: &str = "references/playbook-body.md";

const FENCE: &str = "---";

/// Renders a skill's `SKILL.md`: the `name:` value becomes `name`, and
/// references to other playbook names are rewritten.
pub fn render_skill_md(text: &str, name: &str, known: &BTreeSet<String>) -> Result<String, String> {
    let (front, body) = split_frontmatter(text).ok_or_else(|| {
        format!("skill {name} has no closed frontmatter, so it cannot be rendered")
    })?;
    let front = rename_in_frontmatter(front, name);
    let rendered = format!("{FENCE}\n{front}{FENCE}\n{body}");
    Ok(rewrite_refs(&rendered, known))
}

/// Renders a command as the `SKILL.md` of a skill: only `name` and
/// `description` survive from the frontmatter, and a fixed preamble tells
/// Codex how to run it.
pub fn render_command(text: &str, name: &str, known: &BTreeSet<String>) -> Result<String, String> {
    let (front, body) = split_frontmatter(text)
        .ok_or_else(|| format!("command {name} has no frontmatter, so it cannot be rendered"))?;
    let description = front
        .lines()
        .find(|l| l.starts_with("description:"))
        .ok_or_else(|| format!("command {name} has no description in its frontmatter"))?;
    let rendered = format!(
        "{FENCE}\nname: {name}\n{description}\n{FENCE}\n\n{}\n\n{}",
        command_preamble(name),
        body.trim_start_matches('\n')
    );
    Ok(rewrite_refs(&rendered, known))
}

/// Rewrites `/playbook:<n>`, `playbook:<n>` and `../skills/<n>/` for known
/// names `<n>`. A match must start outside a longer word, URL or path and end
/// at a word boundary, so `playbook:fixer` or `https://x.com/playbook:fix`
/// stay as written.
pub fn rewrite_refs(text: &str, known: &BTreeSet<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if let Some((used, replacement)) = match_ref(text, i, known) {
            out.push_str(&replacement);
            i += used;
        } else if let Some(ch) = text[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Splits a rendered `SKILL.md` that is over `MAX_SKILL_PROMPT_BYTES` into a
/// stub and the body for `BODY_PATH`. A skill within the limit comes back
/// whole with no body.
pub fn split_if_large(skill_md: &str) -> (String, Option<String>) {
    if skill_md.len() <= MAX_SKILL_PROMPT_BYTES {
        return (skill_md.to_string(), None);
    }
    let Some((front, body)) = split_frontmatter(skill_md) else {
        return (skill_md.to_string(), None);
    };
    let body = body.trim_start_matches('\n');
    let preamble = front
        .lines()
        .find_map(|l| l.strip_prefix("name:"))
        .map(|n| command_preamble(n.trim()))
        .filter(|p| body.starts_with(p.as_str()));
    let mut stub = format!("{FENCE}\n{front}{FENCE}\n\n");
    if let Some(preamble) = preamble {
        stub.push_str(&preamble);
        stub.push_str("\n\n");
    }
    stub.push_str(&format!(
        "Read `{BODY_PATH}` and follow it. Resolve every relative path in it \
         from this skill folder, not from the body file.\n"
    ));
    (stub, Some(body.to_string()))
}

fn command_preamble(name: &str) -> String {
    let command = name.strip_prefix(NAME_PREFIX).unwrap_or(name);
    format!(
        "This is the playbook `/{command}` command for Codex. Run it as `${name}`. \
         The text after the skill name is `$ARGUMENTS`. Where a step names the \
         `Agent` tool, `subagent_type`, `SendMessage` or `AskUserQuestion` and they \
         are not available, do that step yourself or ask in chat."
    )
}

/// The lines between the first two `---` lines, and the text after the
/// second one. `None` when the text does not open with a frontmatter block.
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let mut lines = text.split_inclusive('\n');
    let first = lines.next()?;
    if first.trim_end() != FENCE {
        return None;
    }
    let mut end = first.len();
    for line in lines {
        if line.trim_end() == FENCE {
            return Some((&text[first.len()..end], &text[end + line.len()..]));
        }
        end += line.len();
    }
    None
}

fn rename_in_frontmatter(front: &str, name: &str) -> String {
    let mut out = String::with_capacity(front.len() + name.len());
    let mut renamed = false;
    for line in front.split_inclusive('\n') {
        if line.starts_with("name:") {
            out.push_str(&format!("name: {name}\n"));
            renamed = true;
        } else {
            out.push_str(line);
        }
    }
    if !renamed {
        out.insert_str(0, &format!("name: {name}\n"));
    }
    out
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

/// The longest run of name characters at the start of `text`.
fn name_run(text: &str) -> &str {
    let end = text.find(|c: char| !is_name_char(c)).unwrap_or(text.len());
    &text[..end]
}

/// Whether a reference may begin right after `prev`. A name character, `.`,
/// `/` or `:` before it means it sits inside a longer word, URL or path.
fn starts_clean(prev: Option<char>) -> bool {
    prev.is_none_or(|c| !is_name_char(c) && !matches!(c, '.' | '/' | ':'))
}

/// A reference starting at byte `i`: the bytes it covers and its replacement.
fn match_ref(text: &str, i: usize, known: &BTreeSet<String>) -> Option<(usize, String)> {
    const COMMAND: &str = "/playbook:";
    const SKILL: &str = "playbook:";
    const PATH: &str = "../skills/";
    let rest = &text[i..];
    let prev = text[..i].chars().next_back();
    if let Some(after) = rest.strip_prefix(PATH) {
        let n = name_run(after);
        let closed = after[n.len()..].starts_with('/');
        let clean = prev.is_none_or(|c| !is_name_char(c) && c != '.');
        return (clean && closed && known.contains(n))
            .then(|| (PATH.len() + n.len() + 1, format!("../{NAME_PREFIX}{n}/")));
    }
    if let Some(after) = rest.strip_prefix(COMMAND) {
        let n = name_run(after);
        return (starts_clean(prev) && known.contains(n))
            .then(|| (COMMAND.len() + n.len(), format!("${NAME_PREFIX}{n}")));
    }
    if let Some(after) = rest.strip_prefix(SKILL) {
        let n = name_run(after);
        return (starts_clean(prev) && known.contains(n))
            .then(|| (SKILL.len() + n.len(), format!("{NAME_PREFIX}{n}")));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> BTreeSet<String> {
        [
            "fix",
            "plan",
            "implement",
            "writing-style",
            "systematic-debugging",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn a_skill_keeps_its_body_and_gets_the_prefixed_name() {
        // Arrange
        let input = include_str!("testdata/skill_input.md");

        // Act
        let out = render_skill_md(input, "playbook-writing-style", &known()).unwrap();

        // Assert
        assert_eq!(out, include_str!("testdata/skill_expected.md"));
    }

    #[test]
    fn a_command_becomes_a_skill_with_a_preamble_and_rewrites() {
        // Arrange
        let input = include_str!("testdata/command_fix_input.md");

        // Act
        let out = render_command(input, "playbook-fix", &known()).unwrap();

        // Assert
        assert_eq!(out, include_str!("testdata/command_fix_expected.md"));
    }

    #[test]
    fn rewrites_reach_inside_fenced_code_blocks() {
        // Arrange
        let input = include_str!("testdata/command_plan_input.md");

        // Act
        let out = render_command(input, "playbook-plan", &known()).unwrap();

        // Assert
        assert_eq!(out, include_str!("testdata/command_plan_expected.md"));
    }

    #[test]
    fn a_command_without_frontmatter_is_an_error_naming_it() {
        // Arrange
        let input = "# Fix\n\nNo frontmatter here.\n";

        // Act
        let err = render_command(input, "playbook-fix", &known()).unwrap_err();

        // Assert
        assert!(err.contains("playbook-fix"), "{err}");
        assert!(err.contains("no frontmatter"), "{err}");
    }

    #[test]
    fn a_skill_without_closed_frontmatter_is_an_error_naming_it() {
        for input in [
            "# Skill\n\nNo frontmatter.\n",
            "---\nname: x\nno closing fence\n",
        ] {
            // Arrange
            let name = "playbook-x";

            // Act
            let err = render_skill_md(input, name, &known()).unwrap_err();

            // Assert
            assert!(err.contains(name), "{err}");
            assert!(err.contains("no closed frontmatter"), "{err}");
        }
    }

    #[test]
    fn text_that_is_not_a_known_reference_stays_unchanged() {
        // Arrange
        let input = include_str!("testdata/rewrite_negative.md");

        // Act
        let out = rewrite_refs(input, &known());

        // Assert
        assert_eq!(out, input);
    }

    #[test]
    fn known_references_are_rewritten_at_line_edges_and_before_a_period() {
        // Arrange
        let input = include_str!("testdata/rewrite_positive_input.md");

        // Act
        let out = rewrite_refs(input, &known());

        // Assert
        assert_eq!(out, include_str!("testdata/rewrite_positive_expected.md"));
    }

    /// `split_head.md` plus filler so the whole text is `total` bytes.
    fn sized(total: usize) -> String {
        let head = include_str!("testdata/split_head.md");
        format!("{head}{}", "a".repeat(total - head.len()))
    }

    #[test]
    fn a_skill_of_exactly_the_limit_stays_whole() {
        // Arrange
        let text = sized(MAX_SKILL_PROMPT_BYTES);

        // Act
        let (stub, body) = split_if_large(&text);

        // Assert
        assert_eq!(stub, text);
        assert_eq!(body, None);
    }

    #[test]
    fn a_skill_one_byte_over_the_limit_splits_into_a_stub_and_a_body() {
        // Arrange
        let text = sized(MAX_SKILL_PROMPT_BYTES + 1);

        // Act
        let (stub, body) = split_if_large(&text);

        // Assert
        assert!(stub.len() < MAX_SKILL_PROMPT_BYTES);
        let body = body.unwrap();
        assert!(body.contains("Read references/x.md and ../playbook-y/ first."));
        assert!(text.ends_with(&body));
    }

    #[test]
    fn the_stub_tells_the_agent_to_resolve_paths_from_the_skill_folder() {
        // Arrange
        let text = sized(MAX_SKILL_PROMPT_BYTES + 1);

        // Act
        let (stub, _) = split_if_large(&text);

        // Assert
        assert_eq!(stub, include_str!("testdata/split_stub_expected.md"));
    }

    #[test]
    fn a_large_skill_without_a_preamble_gets_a_stub_without_one() {
        // Arrange
        let head = include_str!("testdata/split_skill_head.md");
        let text = format!("{head}{}", "a".repeat(MAX_SKILL_PROMPT_BYTES));

        // Act
        let (stub, body) = split_if_large(&text);

        // Assert
        assert_eq!(
            stub,
            include_str!("testdata/split_stub_no_preamble_expected.md")
        );
        assert!(body.unwrap().starts_with("# Big skill"));
    }

    #[test]
    fn the_real_fix_command_and_writing_style_skill_render() {
        // Arrange
        let root = std::path::Path::new(env!("PLAYBOOK_PLUGIN_ROOT"));
        let command = std::fs::read_to_string(root.join("commands/fix.md")).unwrap();
        let skill = std::fs::read_to_string(root.join("skills/writing-style/SKILL.md")).unwrap();
        let known: BTreeSet<String> = super::super::plan::plan(root)
            .unwrap()
            .iter()
            .filter_map(|t| t.name.strip_prefix(NAME_PREFIX).map(str::to_string))
            .collect();

        // Act
        let command = render_command(&command, "playbook-fix", &known).unwrap();
        let skill = render_skill_md(&skill, "playbook-writing-style", &known).unwrap();
        let (stub, body) = split_if_large(&skill);

        // Assert
        assert!(command.contains("name: playbook-fix\n"));
        assert!(skill.contains("name: playbook-writing-style\n"));
        for text in [&command, &skill] {
            for (at, _) in text.match_indices("playbook:") {
                let after = name_run(&text[at + "playbook:".len()..]);
                assert!(!known.contains(after), "leftover playbook:{after}");
            }
        }
        assert!(stub.len() < MAX_SKILL_PROMPT_BYTES);
        assert!(body.is_some());
    }
}
