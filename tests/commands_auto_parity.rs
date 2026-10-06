// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Structural parity checks for the command files under `commands/`.
//!
//! A command must read the run mode (`playbook mode status`) in a `Step 0`
//! section before it can ask the user anything, so unattended runs never hit
//! a question first. These tests parse each file into frontmatter and
//! heading-delimited sections instead of grepping for markers, because a
//! marker can sit in prose, in a fenced example or in the wrong section and
//! still match.
//!
//! The file table is `COMMANDS`; extending coverage to another command is a
//! new row plus the flags that apply to it. Model behaviour is not testable
//! here: these checks cover prose structure only.

use regex::Regex;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

/// One command file and which structural rules apply to it.
struct CommandSpec {
    name: &'static str,
    /// The command asks the user something outside Step 0. The positive
    /// control asserts this really is detected, so a detector that matches
    /// nothing cannot let the ordering checks pass vacuously.
    asks: bool,
    /// Step 0 must exist, read the mode, and precede the first ask.
    requires_step0: bool,
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "plan",
        asks: true,
        requires_step0: true,
    },
    CommandSpec {
        name: "implement",
        asks: true,
        requires_step0: true,
    },
    CommandSpec {
        name: "adr",
        asks: true,
        requires_step0: false,
    },
    CommandSpec {
        name: "address-pr-comments",
        asks: true,
        requires_step0: false,
    },
    CommandSpec {
        name: "learn-project",
        asks: true,
        requires_step0: false,
    },
    CommandSpec {
        name: "setup",
        asks: true,
        requires_step0: false,
    },
    CommandSpec {
        name: "quick-review",
        asks: true,
        requires_step0: false,
    },
];

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

struct CommandFile {
    name: String,
    lines: Vec<String>,
    /// Index of the first line after the closing frontmatter fence.
    body_start: usize,
}

struct Heading {
    level: usize,
    title: String,
    line: usize,
}

/// A heading and the lines it owns, `start..end` (end exclusive). A section
/// ends at the next heading of the same or a shallower level.
struct Section {
    title: String,
    start: usize,
    end: usize,
}

fn commands_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("commands")
}

fn load(name: &str) -> CommandFile {
    let path = commands_dir().join(format!("{name}.md"));
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let body_start = frontmatter_end(&lines)
        .unwrap_or_else(|| panic!("commands/{name}.md has no closed frontmatter block"));
    CommandFile {
        name: name.to_owned(),
        lines,
        body_start,
    }
}

/// Index just past the closing `---` of a leading frontmatter block.
fn frontmatter_end(lines: &[String]) -> Option<usize> {
    if lines.first().map(String::as_str) != Some("---") {
        return None;
    }
    lines[1..].iter().position(|l| l == "---").map(|i| i + 2)
}

impl CommandFile {
    fn frontmatter(&self) -> &[String] {
        &self.lines[1..self.body_start - 1]
    }

    /// The comma-separated `allowed-tools` frontmatter value as a list.
    fn allowed_tools(&self) -> Vec<String> {
        self.frontmatter()
            .iter()
            .find_map(|l| l.strip_prefix("allowed-tools:"))
            .unwrap_or_else(|| panic!("commands/{}.md has no allowed-tools key", self.name))
            .split(',')
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect()
    }

    /// Markdown headings outside fenced code blocks, so a `# comment` inside
    /// a shell example is not mistaken for a section.
    fn headings(&self) -> Vec<Heading> {
        let mut out = Vec::new();
        let mut in_fence = false;
        for (line, text) in self.lines.iter().enumerate().skip(self.body_start) {
            if text.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            let level = text.chars().take_while(|&c| c == '#').count();
            if (1..=6).contains(&level) && text[level..].starts_with(' ') {
                out.push(Heading {
                    level,
                    title: text[level..].trim().to_owned(),
                    line,
                });
            }
        }
        out
    }

    /// The first section whose heading title satisfies `matches`.
    fn section(&self, matches: impl Fn(&str) -> bool) -> Option<Section> {
        let headings = self.headings();
        let idx = headings.iter().position(|h| matches(&h.title))?;
        let end = headings[idx + 1..]
            .iter()
            .find(|h| h.level <= headings[idx].level)
            .map_or(self.lines.len(), |h| h.line);
        Some(Section {
            title: headings[idx].title.clone(),
            start: headings[idx].line,
            end,
        })
    }

    /// The `Step 0` section, at whatever heading level the file uses.
    fn step0(&self) -> Option<Section> {
        self.section(|t| t.starts_with("Step 0"))
    }

    /// The section that documents Step `n`, e.g. `Step 6: Present the design`.
    fn step(&self, n: &str) -> Option<Section> {
        let prefix = format!("Step {n}");
        self.section(|t| {
            t.strip_prefix(&prefix)
                .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_digit() || c == '.'))
        })
    }

    /// The named auto-path subsection: a heading titled `Auto path`,
    /// optionally followed by a qualifier. It holds every instruction that
    /// applies only when the command runs unattended.
    fn auto_path(&self) -> Option<Section> {
        self.section(|t| t.to_lowercase().starts_with("auto path"))
    }

    fn text_of(&self, s: &Section) -> String {
        self.lines[s.start..s.end].join("\n")
    }

    /// First body line outside `skip` that the ask detector matches.
    fn first_ask_outside(&self, skip: Option<&Section>) -> Option<(usize, &str)> {
        self.lines
            .iter()
            .enumerate()
            .skip(self.body_start)
            .filter(|(i, _)| skip.is_none_or(|s| !(s.start..s.end).contains(i)))
            .find(|(_, l)| is_ask(l))
            .map(|(i, l)| (i, l.as_str()))
    }
}

/// A line that poses a question to the user. Anchored on question forms and
/// whole words: a bare `asks` substring would also match "tasks".
fn is_ask(line: &str) -> bool {
    static ASK: OnceLock<Regex> = OnceLock::new();
    ASK.get_or_init(|| {
        Regex::new(
            r"AskUserQuestion|(?i:ask the user)|\bAsk:|\basks\b|\[Y/n\]|\bDoes this\b|Resume it\?",
        )
        .expect("ask detector pattern is valid")
    })
    .is_match(line)
}

fn require_section(file: &CommandFile, found: Option<Section>, what: &str) -> Section {
    found.unwrap_or_else(|| panic!("commands/{}.md has no {what} section", file.name))
}

// ---------------------------------------------------------------------------
// Step 0 reads the mode before anything asks
// ---------------------------------------------------------------------------

#[test]
fn step0_section_reads_mode_status() {
    // Arrange
    let required: Vec<&CommandSpec> = COMMANDS.iter().filter(|c| c.requires_step0).collect();
    let mut failures = Vec::new();

    for spec in required {
        let file = load(spec.name);

        // Act
        let step0 = file.step0();

        // Assert
        match step0 {
            None => failures.push(format!("{}: no `Step 0` section", spec.name)),
            Some(s) if !file.text_of(&s).contains("playbook mode status") => {
                failures.push(format!(
                    "{}: `{}` never calls `playbook mode status`",
                    spec.name, s.title
                ))
            }
            Some(_) => {}
        }
    }

    assert!(
        failures.is_empty(),
        "Step 0 problems:\n{}",
        failures.join("\n")
    );
}

#[test]
fn step0_comes_before_the_first_ask_outside_it() {
    // Arrange
    let required: Vec<&CommandSpec> = COMMANDS.iter().filter(|c| c.requires_step0).collect();
    let mut failures = Vec::new();

    for spec in required {
        let file = load(spec.name);
        let Some(step0) = file.step0() else {
            failures.push(format!("{}: no `Step 0` section to order", spec.name));
            continue;
        };

        // Act
        let first_ask = file.first_ask_outside(Some(&step0));

        // Assert
        if let Some((line, text)) = first_ask {
            if line < step0.start {
                failures.push(format!(
                    "{}: ask on line {} precedes Step 0 (line {}): {}",
                    spec.name,
                    line + 1,
                    step0.start + 1,
                    text.trim()
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "ordering problems:\n{}",
        failures.join("\n")
    );
}

#[test]
fn positive_control_every_asking_command_has_an_ask_outside_step0() {
    // Arrange
    let asking: Vec<&CommandSpec> = COMMANDS.iter().filter(|c| c.asks).collect();
    assert!(
        asking.iter().any(|c| c.name == "plan") && asking.iter().any(|c| c.name == "implement"),
        "the control table must include plan and implement"
    );

    for spec in asking {
        let file = load(spec.name);
        let step0 = file.step0();

        // Act
        let first_ask = file.first_ask_outside(step0.as_ref());

        // Assert
        assert!(
            first_ask.is_some(),
            "commands/{}.md is expected to ask something outside Step 0, but the ask \
             detector found nothing, so the ordering checks would pass vacuously",
            spec.name
        );
    }
}

// ---------------------------------------------------------------------------
// implement
// ---------------------------------------------------------------------------

#[test]
fn implement_allowed_tools_include_ask_user_question() {
    // Arrange
    let file = load("implement");

    // Act
    let tools = file.allowed_tools();

    // Assert
    assert!(
        tools.iter().any(|t| t == "AskUserQuestion"),
        "implement allowed-tools {tools:?} lacks AskUserQuestion"
    );
}

#[test]
fn implement_never_hard_resets() {
    // Arrange
    let file = load("implement");

    // Act
    let offenders: Vec<usize> = file
        .lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains("reset --hard"))
        .map(|(i, _)| i + 1)
        .collect();

    // Assert
    assert!(
        offenders.is_empty(),
        "commands/implement.md mentions `reset --hard` on lines {offenders:?}"
    );
}

#[test]
fn implement_auto_path_never_forces_and_parks_on_non_fast_forward() {
    // Arrange
    let file = load("implement");
    let auto_path = require_section(&file, file.auto_path(), "`Auto path`");

    // Act
    let text = file.text_of(&auto_path);

    // Assert
    assert!(
        !text.contains("--force"),
        "the auto path section must not mention --force or --force-with-lease"
    );
    let parks = Regex::new(r"(?is)non-fast-forward[^.]*\bparks?\b[^.]*\bSegment\b")
        .expect("park pattern is valid");
    assert!(
        parks.is_match(&text),
        "the auto path section must state that a non-fast-forward push parks the Segment"
    );
}

// ---------------------------------------------------------------------------
// plan
// ---------------------------------------------------------------------------

#[test]
fn plan_step6_says_plain_auto_stops_with_a_message() {
    // Arrange
    let file = load("plan");
    let step6 = require_section(&file, file.step("6"), "`Step 6`");

    // Act
    let text = file.text_of(&step6);

    // Assert
    let stops = Regex::new(r"(?i)plain `--auto`[^.\n]*\bstops?\b").expect("stop pattern is valid");
    assert!(
        stops.is_match(&text),
        "Step 6 must say that plain `--auto` stops"
    );
    assert!(
        text.to_lowercase().contains("message"),
        "Step 6 must say plain `--auto` stops with a clear message"
    );
}

#[test]
fn auto_design_appears_in_plan_and_in_no_other_command() {
    // Arrange
    let mut with_flag: Vec<String> = fs::read_dir(commands_dir())
        .expect("commands directory should be readable")
        .map(|e| e.expect("directory entry should be readable").path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter(|p| {
            fs::read_to_string(p)
                .expect("command file should be readable")
                .contains("--auto-design")
        })
        .map(|p| {
            p.file_stem()
                .expect("command file has a stem")
                .to_string_lossy()
                .into_owned()
        })
        .collect();

    // Act
    with_flag.sort();

    // Assert
    assert_eq!(with_flag, vec!["plan".to_owned()]);
}
