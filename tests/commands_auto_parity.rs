// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Structural parity checks for the command files under `commands/`.
//!
//! A command must read the run mode (`playbook mode status`) in a `Step 0`
//! section before it can ask the user anything, so unattended runs never hit
//! a question first. These tests parse each file into frontmatter and
//! heading-delimited sections instead of grepping for markers, because a
//! marker can sit in prose, in a fenced example or in the wrong section and
//! still match.
//!
//! The file table is `COMMANDS`. A row is covered by the Step 0 and ordering
//! checks, and by the per-row checks for its `Auto` kind where one exists. The
//! checks for the `Dedicated`, `ParksInsteadOfForcing`, `Stages` and
//! `SkipsSelfReview` kinds name their files, so a new command of those kinds
//! needs its own test. Model behaviour is not testable here: these checks
//! cover prose structure only.

use regex::Regex;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

/// What a command does differently when the run mode is auto. Each variant
/// selects the structural checks that apply to the row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Auto {
    /// Checked by its own dedicated tests further down.
    Dedicated,
    /// Stops at Step 0 with a one-line refusal naming the command and why.
    Refuses,
    /// Behaves as `--self`: reports locally and never posts a review.
    ReportsOnly,
    /// Behaves as `--stage`: collects candidates and skips the write prompt.
    Stages,
    /// Never force-pushes; parks the work when a plain push cannot succeed.
    ParksInsteadOfForcing,
    /// Skips the `/clear` self-review and says in the final report that no
    /// self-review ran when the caller was not `implement`.
    SkipsSelfReview,
    /// Asks nothing and behaves the same in both modes, so it has no Step 0.
    Unchanged,
    /// Asks only inside an explicit `if mode is ask` branch, so an
    /// unattended run never reaches a question.
    AsksOnlyInAskMode,
}

/// One command file and which structural rules apply to it.
struct CommandSpec {
    name: &'static str,
    /// The command asks the user something outside Step 0. The positive
    /// control asserts this really is detected, so a detector that matches
    /// nothing cannot let the ordering checks pass vacuously. Rows that never
    /// ask leave it false: their prose only mentions asking in order to
    /// forbid it, so ordering against a detected ask would be meaningless.
    asks: bool,
    auto: Auto,
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "plan",
        asks: true,
        auto: Auto::Dedicated,
    },
    CommandSpec {
        name: "implement",
        asks: true,
        auto: Auto::Dedicated,
    },
    CommandSpec {
        name: "commit-and-push",
        asks: false,
        auto: Auto::ParksInsteadOfForcing,
    },
    CommandSpec {
        name: "create-pull-request",
        asks: false,
        auto: Auto::SkipsSelfReview,
    },
    CommandSpec {
        name: "setup",
        asks: true,
        auto: Auto::Refuses,
    },
    CommandSpec {
        name: "adr",
        asks: true,
        auto: Auto::Refuses,
    },
    CommandSpec {
        name: "address-pr-comments",
        asks: true,
        auto: Auto::Refuses,
    },
    CommandSpec {
        name: "quick-review",
        asks: true,
        auto: Auto::ReportsOnly,
    },
    CommandSpec {
        name: "deep-review",
        asks: true,
        auto: Auto::ReportsOnly,
    },
    CommandSpec {
        name: "learn-project",
        asks: true,
        auto: Auto::Stages,
    },
    CommandSpec {
        name: "repo-audit",
        asks: false,
        auto: Auto::Unchanged,
    },
    CommandSpec {
        name: "doctor",
        asks: false,
        auto: Auto::Unchanged,
    },
    CommandSpec {
        name: "session-start",
        asks: false,
        auto: Auto::Unchanged,
    },
    CommandSpec {
        name: "fix",
        asks: true,
        auto: Auto::AsksOnlyInAskMode,
    },
];

fn rows(auto: Auto) -> impl Iterator<Item = &'static CommandSpec> {
    COMMANDS.iter().filter(move |c| c.auto == auto)
}

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

    /// The raw value of a top-level frontmatter key, quotes included.
    fn frontmatter_value(&self, key: &str) -> Option<String> {
        let prefix = format!("{key}:");
        self.frontmatter()
            .iter()
            .find_map(|l| l.strip_prefix(&prefix))
            .map(|v| v.trim().to_owned())
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
            if let Some(title) = heading_title(text) {
                out.push(Heading {
                    level: text.chars().take_while(|&c| c == '#').count(),
                    title: title.to_owned(),
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

    /// Blank-line separated blocks inside `s`, each with its first line
    /// index. A sentence wrapped over several lines stays in one block, so
    /// checks on a block do not depend on where the prose was wrapped.
    fn paragraphs(&self, s: &Section) -> Vec<(usize, String)> {
        let mut out: Vec<(usize, String)> = Vec::new();
        let mut open = false;
        for i in s.start..s.end {
            let line = &self.lines[i];
            if line.trim().is_empty() {
                open = false;
            } else if open {
                let last = out.last_mut().expect("an open block exists");
                last.1.push('\n');
                last.1.push_str(line);
            } else {
                out.push((i, line.clone()));
                open = true;
            }
        }
        out
    }

    /// Comment text inside shell code fences (a whole `#` line or a trailing
    /// ` # note`) and inside HTML comments, with its line index.
    fn comments(&self) -> Vec<(usize, String)> {
        static HTML: OnceLock<Regex> = OnceLock::new();
        let html = HTML.get_or_init(|| Regex::new(r"<!--.*?-->").expect("html comment pattern"));
        let mut out = Vec::new();
        let mut shell_fence = false;
        let mut in_fence = false;
        for (i, line) in self.lines.iter().enumerate().skip(self.body_start) {
            let trimmed = line.trim_start();
            if let Some(info) = trimmed.strip_prefix("```") {
                in_fence = !in_fence;
                shell_fence = in_fence && matches!(info.trim(), "bash" | "sh" | "shell" | "zsh");
                continue;
            }
            if shell_fence {
                if trimmed.starts_with('#') && !trimmed.starts_with("#!") {
                    out.push((i, trimmed.to_owned()));
                } else if let Some(at) = line.find(" # ") {
                    out.push((i, line[at..].to_owned()));
                }
            }
            out.extend(html.find_iter(line).map(|m| (i, m.as_str().to_owned())));
        }
        out
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
/// whole words: a bare `asks` substring would also match "tasks", and a bare
/// "ask" would match prose that only forbids asking. The `Q<n>` form is a
/// labelled prompt whose quoted text holds a question mark, the shape the
/// review commands use for their posting questions.
fn is_ask(line: &str) -> bool {
    static ASK: OnceLock<Regex> = OnceLock::new();
    ASK.get_or_init(|| {
        Regex::new(
            r#"AskUserQuestion|(?i:ask the user)|(?i:ask once)|\bAsk:|\basks\b|\[Y/n\]|\bDoes this\b|Resume it\?|\bQ\d\b[:*\s]*"[^"\n]*\?[^"\n]*""#,
        )
        .expect("ask detector pattern is valid")
    })
    .is_match(line)
}

fn require_section(file: &CommandFile, found: Option<Section>, what: &str) -> Section {
    found.unwrap_or_else(|| panic!("commands/{}.md has no {what} section", file.name))
}

/// The phrase that marks a branch taken only when the user can be asked.
const ASK_BRANCH: &str = "if mode is ask";

/// Per line: true for a fence delimiter and for every line inside a fenced
/// code block.
fn fence_flags(lines: &[String]) -> Vec<bool> {
    let mut in_fence = false;
    lines
        .iter()
        .map(|l| {
            if l.trim_start().starts_with("```") {
                in_fence = !in_fence;
                true
            } else {
                in_fence
            }
        })
        .collect()
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn heading_title(line: &str) -> Option<&str> {
    let level = line.chars().take_while(|&c| c == '#').count();
    ((1..=6).contains(&level) && line[level..].starts_with(' ')).then(|| line[level..].trim())
}

fn is_bullet(line: &str) -> bool {
    static BULLET: OnceLock<Regex> = OnceLock::new();
    BULLET
        .get_or_init(|| Regex::new(r"^\s*([-*+]|\d+[.)])\s+").expect("bullet pattern"))
        .is_match(line)
}

/// Whether the ask on `lines[idx]` sits behind an `if mode is ask` branch:
/// the title of its nearest enclosing heading, or the first line of its own
/// bullet or of any bullet enclosing it, contains the phrase. Lines before
/// `first` (the frontmatter) are never read.
fn is_behind_ask_branch(lines: &[String], first: usize, idx: usize) -> bool {
    let fenced = fence_flags(lines);
    let mut indent = indent_of(&lines[idx]);
    for i in (first..=idx).rev().filter(|&i| !fenced[i]) {
        if let Some(title) = heading_title(&lines[i]) {
            return title.to_lowercase().contains(ASK_BRANCH);
        }
        let encloses = i == idx || indent_of(&lines[i]) < indent;
        if is_bullet(&lines[i]) && encloses {
            if lines[i].to_lowercase().contains(ASK_BRANCH) {
                return true;
            }
            indent = indent_of(&lines[i]);
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Step 0 reads the mode before anything asks
// ---------------------------------------------------------------------------

#[test]
fn every_command_file_has_a_row_in_the_table_and_the_reverse() {
    // Arrange
    let mut on_disk: Vec<String> = fs::read_dir(commands_dir())
        .expect("commands/ is readable")
        .map(|e| e.expect("directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    let mut in_table: Vec<String> = COMMANDS.iter().map(|c| c.name.to_owned()).collect();

    // Act
    on_disk.sort();
    in_table.sort();

    // Assert
    assert_eq!(
        on_disk, in_table,
        "commands/*.md and the COMMANDS table must list the same commands"
    );
}

#[test]
fn step0_section_reads_mode_status() {
    // Arrange
    let mut failures = Vec::new();

    for spec in COMMANDS.iter().filter(|c| c.auto != Auto::Unchanged) {
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
    let mut failures = Vec::new();

    for spec in COMMANDS.iter().filter(|c| c.asks) {
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
    for required in ["plan", "implement", "deep-review"] {
        assert!(
            asking.iter().any(|c| c.name == required),
            "the control table must include {required}"
        );
    }

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

#[test]
fn ask_detector_matches_question_forms_and_not_mentions() {
    // Arrange
    let asks = [
        r#"- **Q1:** "Post which findings as a pending review?" Offer six tiers."#,
        r#"**Q1**: "Post which findings as a pending review?" Offer six tiers."#,
        r#"- **Q2:** "Submit verb? approve / comment / request-changes / skip." Reaching it."#,
        r#"Plan the rest. Ask once: "Write these to memory?""#,
        "call the AskUserQuestion tool ONCE",
    ];
    let not_asks = [
        "8. **Never ask whether to run.** Invoking the command IS the instruction to run.",
        "Q2 stays pending until the author submits it.",
        "Run every task in order and report each result.",
        "The reviewer asked for changes on three files.",
    ];

    // Act
    let missed: Vec<&&str> = asks.iter().filter(|l| !is_ask(l)).collect();
    let false_hits: Vec<&&str> = not_asks.iter().filter(|l| is_ask(l)).collect();

    // Assert
    assert!(missed.is_empty(), "detector missed real asks: {missed:?}");
    assert!(
        false_hits.is_empty(),
        "detector matched prose that is not an ask: {false_hits:?}"
    );
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
fn auto_design_appears_in_plan_and_in_the_fix_hand_off_only() {
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
    assert_eq!(with_flag, vec!["fix".to_owned(), "plan".to_owned()]);
}

// ---------------------------------------------------------------------------
// Commands that refuse to run unattended: setup, adr, address-pr-comments
// ---------------------------------------------------------------------------

#[test]
fn refuse_list_step0_stops_in_auto_naming_the_command_and_why() {
    // Arrange
    let mut failures = Vec::new();

    for spec in rows(Auto::Refuses) {
        let file = load(spec.name);
        let Some(step0) = file.step0() else {
            failures.push(format!("{}: no `Step 0` section", spec.name));
            continue;
        };
        let command = format!("/playbook:{}", spec.name);
        let blocks = file.paragraphs(&step0);

        // Act
        let read = blocks
            .iter()
            .find(|(_, b)| b.contains("playbook mode status"));
        let refusal = blocks.iter().find(|(_, b)| {
            let b = b.to_lowercase();
            b.contains("auto")
                && b.contains("stop")
                && b.contains(&command)
                && b.contains("because")
        });

        // Assert
        let Some((refusal_line, _)) = refusal else {
            failures.push(format!(
                "{}: Step 0 has no refusal that mentions auto, stop, `{command}` and because",
                spec.name
            ));
            continue;
        };
        match read {
            None => failures.push(format!("{}: Step 0 never reads the mode", spec.name)),
            Some((read_line, _)) if read_line > refusal_line => failures.push(format!(
                "{}: the refusal (line {}) precedes the mode read (line {})",
                spec.name,
                refusal_line + 1,
                read_line + 1
            )),
            Some(_) => {}
        }
        if let Some((line, text)) = file.first_ask_outside(None) {
            if line < *refusal_line {
                failures.push(format!(
                    "{}: ask on line {} precedes the refusal (line {}): {}",
                    spec.name,
                    line + 1,
                    refusal_line + 1,
                    text.trim()
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "refusal problems:\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// quick-review and deep-review: auto never posts to GitHub
// ---------------------------------------------------------------------------

/// The section that creates and submits the GitHub review.
fn posting_section(file: &CommandFile) -> Section {
    require_section(
        file,
        file.section(|t| t.to_lowercase().contains("orchestrate posting")),
        "`Orchestrate posting`",
    )
}

/// A shell line that calls the GitHub API to create or submit a review.
fn is_post_call(line: &str) -> bool {
    static POST: OnceLock<Regex> = OnceLock::new();
    POST.get_or_init(|| Regex::new(r"gh api[^\n]*(-X POST|/reviews)").expect("post pattern"))
        .is_match(line)
}

#[test]
fn review_step0_says_auto_implies_self() {
    // Arrange
    let implies = Regex::new(r"(?is)\bauto\b[^.]*\bimplies\b[^.]*--self").expect("implies pattern");
    let mut failures = Vec::new();

    for spec in rows(Auto::ReportsOnly) {
        let file = load(spec.name);

        // Act
        let step0 = file.step0();

        // Assert
        match step0 {
            None => failures.push(format!("{}: no `Step 0` section", spec.name)),
            Some(s) if !implies.is_match(&file.text_of(&s)) => failures.push(format!(
                "{}: Step 0 never says auto implies `--self`",
                spec.name
            )),
            Some(_) => {}
        }
    }

    assert!(
        failures.is_empty(),
        "review Step 0 problems:\n{}",
        failures.join("\n")
    );
}

#[test]
fn review_posting_section_is_gated_on_not_self_and_holds_every_post_call() {
    // Arrange
    let mut failures = Vec::new();

    for spec in rows(Auto::ReportsOnly) {
        let file = load(spec.name);
        let posting = posting_section(&file);

        // Act
        let gate = file
            .paragraphs(&posting)
            .into_iter()
            .find(|(_, b)| b.contains("SELF_MODE") && b.to_lowercase().contains("stop"));
        let outside: Vec<usize> = (file.body_start..file.lines.len())
            .filter(|i| !(posting.start..posting.end).contains(i))
            .filter(|&i| is_post_call(&file.lines[i]))
            .collect();

        // Assert
        match gate {
            None => failures.push(format!(
                "{}: `{}` has no `SELF_MODE` ... stop gate",
                spec.name, posting.title
            )),
            Some((gate_line, _)) => {
                let reached_first = (posting.start..gate_line)
                    .find(|&i| is_post_call(&file.lines[i]) || is_ask(&file.lines[i]));
                if let Some(i) = reached_first {
                    failures.push(format!(
                        "{}: line {} posts or asks before the `SELF_MODE` gate (line {})",
                        spec.name,
                        i + 1,
                        gate_line + 1
                    ));
                }
            }
        }
        if !outside.is_empty() {
            failures.push(format!(
                "{}: review post calls outside `{}` on lines {:?}",
                spec.name,
                posting.title,
                outside.iter().map(|i| i + 1).collect::<Vec<_>>()
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "posting gate problems:\n{}",
        failures.join("\n")
    );
}

#[test]
fn review_auto_path_text_never_posts_a_review() {
    // Arrange
    let mut failures = Vec::new();

    for spec in rows(Auto::ReportsOnly) {
        let file = load(spec.name);
        let posting = posting_section(&file);
        let Some(step0) = file.step0() else {
            failures.push(format!(
                "{}: no `Step 0` section, so no auto path to inspect",
                spec.name
            ));
            continue;
        };
        let auto_text: Vec<Section> = [Some(step0), file.auto_path()]
            .into_iter()
            .flatten()
            .collect();

        // Act
        for section in &auto_text {
            let overlaps = section.start < posting.end && posting.start < section.end;
            let posts = (section.start..section.end).any(|i| is_post_call(&file.lines[i]));

            // Assert
            if overlaps {
                failures.push(format!(
                    "{}: `{}` overlaps the posting section",
                    spec.name, section.title
                ));
            }
            if posts {
                failures.push(format!(
                    "{}: `{}` contains a call that posts a review",
                    spec.name, section.title
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "auto path problems:\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// commit-and-push
// ---------------------------------------------------------------------------

#[test]
fn commit_and_push_auto_path_never_forces_and_parks_when_a_force_is_needed() {
    // Arrange
    let file = load("commit-and-push");
    let auto_path = require_section(&file, file.auto_path(), "`Auto path`");

    // Act
    let text = file.text_of(&auto_path);

    // Assert
    assert!(
        !text.contains("--force"),
        "the auto path must not mention --force or --force-with-lease"
    );
    let stops = Regex::new(r"(?is)\bforce\b[^.]*\bstops and reports\b").expect("stop pattern");
    assert!(
        stops.is_match(&text),
        "the auto path must say a push that would need a force stops and reports"
    );
}

#[test]
fn commit_and_push_auto_path_pushes_plainly_when_the_remote_branch_is_absent() {
    // Arrange
    let file = load("commit-and-push");
    let auto_path = require_section(&file, file.auto_path(), "`Auto path`");

    // Act
    let text = file.text_of(&auto_path);

    // Assert
    assert!(
        text.contains("git ls-remote"),
        "the auto path must check the remote branch with `git ls-remote`"
    );
    let plain = Regex::new(
        r"(?is)\b(absent|missing)\b[^.]*\bplain(ly)?\b|\bplain(ly)?\b[^.]*\b(absent|missing)\b",
    )
    .expect("plain push pattern");
    assert!(
        plain.is_match(&text),
        "the auto path must say it pushes plainly when the remote branch is absent"
    );
}

// ---------------------------------------------------------------------------
// learn-project
// ---------------------------------------------------------------------------

#[test]
fn learn_project_step0_stages_in_auto_and_skips_the_write_question() {
    // Arrange
    let file = load("learn-project");
    let step0 = require_section(&file, file.step0(), "`Step 0`");
    let stages = Regex::new(r"(?is)\bauto\b[^.]*--stage").expect("stage pattern");
    let skips = Regex::new(
        r"(?is)\bskips?\b[^.]*(write question|Write these to memory)|(write question|Write these to memory)[^.]*\bskipped\b",
    )
    .expect("skip pattern");

    // Act
    let text = file.text_of(&step0);

    // Assert
    assert!(
        stages.is_match(&text),
        "Step 0 must say auto behaves as `--stage`"
    );
    assert!(
        skips.is_match(&text),
        "Step 0 must say the write question is skipped in that path"
    );
}

// ---------------------------------------------------------------------------
// create-pull-request
// ---------------------------------------------------------------------------

#[test]
fn create_pull_request_step0_skips_the_clear_self_review_in_auto() {
    // Arrange
    let file = load("create-pull-request");
    let step0 = require_section(&file, file.step0(), "`Step 0`");
    let skips = Regex::new(r"(?is)\bauto\b[^.]*\bskips?\b[^.]*/clear").expect("skip pattern");
    let why = Regex::new(r"(?is)\bimplement\b[^.]*\bopened\b[^.]*\breview\b[^.]*\bcovered\b")
        .expect("why pattern");
    let reports =
        Regex::new(r"(?is)final report[^.]*\bno self-review ran\b").expect("report pattern");

    // Act
    let text = file.text_of(&step0);

    // Assert
    assert!(
        skips.is_match(&text),
        "Step 0 must say that in auto the `/clear` self-review is skipped"
    );
    assert!(
        why.is_match(&text),
        "Step 0 must say that implement's review covered a PR that implement opened"
    );
    assert!(
        reports.is_match(&text),
        "Step 0 must say the final report states that no self-review ran for other callers"
    );
}

// ---------------------------------------------------------------------------
// repo-audit, doctor, session-start: no mode block at all
// ---------------------------------------------------------------------------

#[test]
fn commands_that_never_ask_carry_no_mode_block() {
    // Arrange
    let mut failures = Vec::new();

    for spec in rows(Auto::Unchanged) {
        let file = load(spec.name);

        // Act
        if let Some((line, text)) = file.first_ask_outside(None) {
            failures.push(format!(
                "{}: expected no asks, but line {} matches: {}",
                spec.name,
                line + 1,
                text.trim()
            ));
        }
        if file.step0().is_some() {
            failures.push(format!(
                "{}: has a `Step 0`, but behaves the same in either mode",
                spec.name
            ));
        }
    }

    // Assert
    assert!(
        failures.is_empty(),
        "no-mode-block problems:\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// fix
// ---------------------------------------------------------------------------

/// The `escalat` section of `fix.md`, where the hand-off rules live.
fn escalation_section(file: &CommandFile) -> Section {
    require_section(
        file,
        file.section(|t| t.to_lowercase().contains("escalat")),
        "escalation",
    )
}

#[test]
fn fix_command_file_exists() {
    // Arrange
    let path = commands_dir().join("fix.md");

    // Act
    let exists = path.is_file();

    // Assert
    assert!(exists, "{} does not exist", path.display());
}

#[test]
fn fix_frontmatter_has_description_tools_and_the_argument_hint() {
    // Arrange
    let file = load("fix");

    // Act
    let description = file.frontmatter_value("description");
    let hint = file.frontmatter_value("argument-hint");
    let tools = file.allowed_tools();

    // Assert
    assert!(
        description.is_some_and(|d| !d.is_empty()),
        "fix needs a non-empty description"
    );
    assert!(
        hint.as_deref()
            .is_some_and(|h| h.contains("[#issue | description] [--auto] [--ask]")),
        "fix argument-hint {hint:?} must contain `[#issue | description] [--auto] [--ask]`"
    );
    for needed in ["Bash", "Read", "Skill"] {
        assert!(
            tools.iter().any(|t| t == needed),
            "fix allowed-tools {tools:?} lacks {needed}"
        );
    }
}

#[test]
fn fix_escalation_names_all_five_rules() {
    // Arrange
    let file = load("fix");
    let section = escalation_section(&file);
    let text = file.text_of(&section);
    let rules = [
        (
            "more than fix.maxFiles files",
            r"(?is)more than[^.]*fix\.maxFiles[^.]*files",
        ),
        (
            "more than fix.maxLines lines, excluding the new test",
            r"(?is)more than[^.]*fix\.maxLines[^.]*excluding the new test",
        ),
        (
            "root cause unclear after two tested hypotheses",
            r"(?is)root cause[^.]*unclear[^.]*two tested hypotheses",
        ),
        (
            "new dependency, schema, config format or public interface",
            r"(?is)new dependency[^.]*schema[^.]*config[- ]format[^.]*public[- ]interface",
        ),
        ("no failing test possible", r"(?i)\bno failing test\b"),
    ];

    // Act
    let missing: Vec<&str> = rules
        .iter()
        .filter(|(_, pattern)| !Regex::new(pattern).expect("rule pattern").is_match(&text))
        .map(|(name, _)| *name)
        .collect();

    // Assert
    assert!(
        missing.is_empty(),
        "the `{}` section does not state these escalation rules: {missing:?}",
        section.title
    );
}

#[test]
fn fix_reads_both_thresholds_through_playbook_config_get() {
    // Arrange
    let file = load("fix");
    let text = file.lines[file.body_start..].join("\n");

    // Act
    let missing: Vec<&str> = ["fix.maxFiles", "fix.maxLines"]
        .into_iter()
        .filter(|key| !text.contains(&format!("playbook config get {key}")))
        .collect();

    // Assert
    assert!(
        missing.is_empty(),
        "fix never runs `playbook config get` for {missing:?}"
    );
}

#[test]
fn fix_escalation_hands_off_to_plan_and_states_which_rule_fired() {
    // Arrange
    let file = load("fix");
    let section = escalation_section(&file);
    let text = file.text_of(&section);
    let states_rule = Regex::new(r"(?is)\bwhich rule (fired|triggered)\b").expect("rule pattern");

    // Act
    let hands_off = text.contains("/playbook:plan");

    // Assert
    assert!(
        hands_off,
        "the `{}` section must hand off to `/playbook:plan`",
        section.title
    );
    assert!(
        states_rule.is_match(&text),
        "the `{}` section must say it states which rule fired",
        section.title
    );
}

#[test]
fn fix_every_ask_sits_behind_an_if_mode_is_ask_branch() {
    // Arrange
    let mut failures = Vec::new();

    for spec in rows(Auto::AsksOnlyInAskMode) {
        let file = load(spec.name);
        let fenced = fence_flags(&file.lines);

        // Act
        for (i, line) in file.lines.iter().enumerate().skip(file.body_start) {
            if fenced[i] || !is_ask(line) {
                continue;
            }

            // Assert
            if !is_behind_ask_branch(&file.lines, file.body_start, i) {
                failures.push(format!(
                    "{}: line {} asks outside an `{ASK_BRANCH}` branch: {}",
                    spec.name,
                    i + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "unconditional asks:\n{}",
        failures.join("\n")
    );
}

#[test]
fn ask_branch_parser_accepts_headings_and_bullets_and_rejects_the_rest() {
    // Arrange
    let lines = |text: &str| -> Vec<String> { text.lines().map(str::to_owned).collect() };
    let behind = [
        "### If mode is ask\nAsk the user which test to keep.",
        "## Step 3\n- If mode is ask, ask the user which test to keep.",
        "## Step 3\n- If mode is ask:\n  - Ask the user which test to keep.",
        "## Step 3\n- If mode is ask:\n  Ask the user which test to keep.",
    ];
    let not_behind = [
        "## Step 3\nAsk the user which test to keep.",
        "## Step 3\n- Ask the user which test to keep.",
        "### If mode is ask\n## Step 4\nAsk the user which test to keep.",
        "- If mode is ask:\n  - Pick one.\n\n- Ask the user which test to keep.",
        "## Step 3\n```\n# if mode is ask\n```\nAsk the user which test to keep.",
    ];
    let ask_index = |l: &[String]| {
        l.iter()
            .position(|x| is_ask(x))
            .expect("fixture has an ask")
    };

    // Act
    let missed: Vec<&&str> = behind
        .iter()
        .filter(|t| {
            let l = lines(t);
            !is_behind_ask_branch(&l, 0, ask_index(&l))
        })
        .collect();
    let false_hits: Vec<&&str> = not_behind
        .iter()
        .filter(|t| {
            let l = lines(t);
            is_behind_ask_branch(&l, 0, ask_index(&l))
        })
        .collect();

    // Assert
    assert!(missed.is_empty(), "parser rejected gated asks: {missed:?}");
    assert!(
        false_hits.is_empty(),
        "parser accepted ungated asks: {false_hits:?}"
    );
}

#[test]
fn fix_opens_its_pull_request_through_the_create_pull_request_skill() {
    // Arrange
    let file = load("fix");
    let fenced = fence_flags(&file.lines);
    let tools = file.allowed_tools();

    // Act
    let names_skill = file.lines[file.body_start..]
        .iter()
        .any(|l| l.contains("/playbook:create-pull-request"));
    let raw_calls: Vec<usize> = (file.body_start..file.lines.len())
        .filter(|&i| fenced[i] && file.lines[i].contains("gh pr create"))
        .map(|i| i + 1)
        .collect();

    // Assert
    assert!(
        names_skill,
        "fix must open its PR through `/playbook:create-pull-request`"
    );
    assert!(
        tools.iter().any(|t| t == "Skill"),
        "fix allowed-tools {tools:?} lacks Skill, so it cannot invoke the skill"
    );
    assert!(
        raw_calls.is_empty(),
        "fix runs `gh pr create` directly on lines {raw_calls:?}"
    );
}

// ---------------------------------------------------------------------------
// Comments describe the code, not the work that produced it
// ---------------------------------------------------------------------------

/// True when `text` points at internal planning: a work-unit label, a
/// numbered ticket or a planning term that means nothing to a later reader.
fn references_planning(text: &str) -> bool {
    static PLANNING: OnceLock<Regex> = OnceLock::new();
    PLANNING
        .get_or_init(|| {
            Regex::new(
                r"(?i)\bWU-?\d+\b|\bwork units?\b|\bgh-\d+\b|#\d+\b|\bissue\s+\d+\b|\bsegment\s+S\d+\b|\bthe brief\b",
            )
            .expect("planning reference pattern")
        })
        .is_match(text)
}

#[test]
fn planning_reference_scan_flags_references_and_leaves_plain_comments() {
    // Arrange
    let flagged = [
        "// ported in WU-16",
        "# see gh-123 for the design",
        "<!-- Work Unit 3 only -->",
        "# fixes #123",
        "// the brief says so",
    ];
    let clean = [
        "// a section ends at the next heading of the same level",
        "# Set each flag from $ARGUMENTS: true when the flag was passed",
        "# 1) read the mode first",
        "<!-- keep in sync with the help block -->",
    ];

    // Act
    let missed: Vec<&&str> = flagged.iter().filter(|t| !references_planning(t)).collect();
    let false_hits: Vec<&&str> = clean.iter().filter(|t| references_planning(t)).collect();

    // Assert
    assert!(missed.is_empty(), "scan missed references: {missed:?}");
    assert!(
        false_hits.is_empty(),
        "scan flagged plain comments: {false_hits:?}"
    );
}

#[test]
fn comments_in_this_test_and_the_commands_carry_no_planning_references() {
    // Arrange
    let mut offenders = Vec::new();
    let own_source = include_str!("commands_auto_parity.rs");
    assert!(
        !load("implement").comments().is_empty(),
        "the comment extractor found nothing in a file full of shell comments"
    );

    // Act
    for (i, line) in own_source.lines().enumerate() {
        if line.trim_start().starts_with("//") && references_planning(line) {
            offenders.push(format!(
                "tests/commands_auto_parity.rs:{}: {}",
                i + 1,
                line.trim()
            ));
        }
    }
    for spec in COMMANDS {
        let file = load(spec.name);
        for (i, text) in file.comments() {
            if references_planning(&text) {
                offenders.push(format!(
                    "commands/{}.md:{}: {}",
                    spec.name,
                    i + 1,
                    text.trim()
                ));
            }
        }
    }

    // Assert
    assert!(
        offenders.is_empty(),
        "comments reference planning artefacts:\n{}",
        offenders.join("\n")
    );
}
