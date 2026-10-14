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

#![allow(dead_code)]

use regex::Regex;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

/// What a command does differently when the run mode is auto. Each variant
/// selects the structural checks that apply to the row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Auto {
    /// Checked by its own dedicated tests further down.
    Dedicated,
    /// Stops at Step 0 with a one-line refusal naming the command and why.
    Refuses,
    /// Auto mode does not skip the post question; only `--self` and
    /// `--no-post` do.
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
pub struct CommandSpec {
    pub name: &'static str,
    /// The command asks the user something outside Step 0. The positive
    /// control asserts this really is detected, so a detector that matches
    /// nothing cannot let the ordering checks pass vacuously. Rows that never
    /// ask leave it false: their prose only mentions asking in order to
    /// forbid it, so ordering against a detected ask would be meaningless.
    pub asks: bool,
    pub auto: Auto,
}

pub const COMMANDS: &[CommandSpec] = &[
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
        name: "session-handoff",
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

pub fn rows(auto: Auto) -> impl Iterator<Item = &'static CommandSpec> {
    COMMANDS.iter().filter(move |c| c.auto == auto)
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

pub struct CommandFile {
    pub name: String,
    pub lines: Vec<String>,
    /// Index of the first line after the closing frontmatter fence.
    pub body_start: usize,
}

pub struct Heading {
    pub level: usize,
    pub title: String,
    pub line: usize,
}

/// A heading and the lines it owns, `start..end` (end exclusive). A section
/// ends at the next heading of the same or a shallower level.
pub struct Section {
    pub title: String,
    pub start: usize,
    pub end: usize,
}

pub fn commands_dir() -> PathBuf {
    PathBuf::from(env!("PLAYBOOK_PLUGIN_ROOT")).join("commands")
}

pub fn load(name: &str) -> CommandFile {
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
pub fn frontmatter_end(lines: &[String]) -> Option<usize> {
    if lines.first().map(String::as_str) != Some("---") {
        return None;
    }
    lines[1..].iter().position(|l| l == "---").map(|i| i + 2)
}

impl CommandFile {
    pub fn frontmatter(&self) -> &[String] {
        &self.lines[1..self.body_start - 1]
    }

    /// The comma-separated `allowed-tools` frontmatter value as a list.
    pub fn allowed_tools(&self) -> Vec<String> {
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
    pub fn frontmatter_value(&self, key: &str) -> Option<String> {
        let prefix = format!("{key}:");
        self.frontmatter()
            .iter()
            .find_map(|l| l.strip_prefix(&prefix))
            .map(|v| v.trim().to_owned())
    }

    /// Markdown headings outside fenced code blocks, so a `# comment` inside
    /// a shell example is not mistaken for a section.
    pub fn headings(&self) -> Vec<Heading> {
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
    pub fn section(&self, matches: impl Fn(&str) -> bool) -> Option<Section> {
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
    pub fn step0(&self) -> Option<Section> {
        self.section(|t| t.starts_with("Step 0"))
    }

    /// The section that documents Step `n`, e.g. `Step 6: Present the design`.
    pub fn step(&self, n: &str) -> Option<Section> {
        let prefix = format!("Step {n}");
        self.section(|t| {
            t.strip_prefix(&prefix)
                .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_digit() || c == '.'))
        })
    }

    /// The named auto-path subsection: a heading titled `Auto path`,
    /// optionally followed by a qualifier. It holds every instruction that
    /// applies only when the command runs unattended.
    pub fn auto_path(&self) -> Option<Section> {
        self.section(|t| t.to_lowercase().starts_with("auto path"))
    }

    pub fn text_of(&self, s: &Section) -> String {
        self.lines[s.start..s.end].join("\n")
    }

    /// Blank-line separated blocks inside `s`, each with its first line
    /// index. A sentence wrapped over several lines stays in one block, so
    /// checks on a block do not depend on where the prose was wrapped.
    pub fn paragraphs(&self, s: &Section) -> Vec<(usize, String)> {
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
    pub fn comments(&self) -> Vec<(usize, String)> {
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
    pub fn first_ask_outside(&self, skip: Option<&Section>) -> Option<(usize, &str)> {
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
pub fn is_ask(line: &str) -> bool {
    static ASK: OnceLock<Regex> = OnceLock::new();
    ASK.get_or_init(|| {
        Regex::new(
            r#"AskUserQuestion|(?i:ask the user)|(?i:ask once)|\bAsk:|\basks\b|\[Y/n\]|\bDoes this\b|Resume it\?|\bQ\d\b[:*\s]*"[^"\n]*\?[^"\n]*""#,
        )
        .expect("ask detector pattern is valid")
    })
    .is_match(line)
}

pub fn require_section(file: &CommandFile, found: Option<Section>, what: &str) -> Section {
    found.unwrap_or_else(|| panic!("commands/{}.md has no {what} section", file.name))
}

/// The phrase that marks a branch taken only when the user can be asked.
pub const ASK_BRANCH: &str = "if mode is ask";

/// Per line: true for a fence delimiter and for every line inside a fenced
/// code block.
pub fn fence_flags(lines: &[String]) -> Vec<bool> {
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

pub fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

pub fn heading_title(line: &str) -> Option<&str> {
    let level = line.chars().take_while(|&c| c == '#').count();
    ((1..=6).contains(&level) && line[level..].starts_with(' ')).then(|| line[level..].trim())
}

pub fn is_bullet(line: &str) -> bool {
    static BULLET: OnceLock<Regex> = OnceLock::new();
    BULLET
        .get_or_init(|| Regex::new(r"^\s*([-*+]|\d+[.)])\s+").expect("bullet pattern"))
        .is_match(line)
}

/// Whether the ask on `lines[idx]` sits behind an `if mode is ask` branch:
/// the title of its nearest enclosing heading, or the first line of its own
/// bullet or of any bullet enclosing it, contains the phrase. Lines before
/// `first` (the frontmatter) are never read.
pub fn is_behind_ask_branch(lines: &[String], first: usize, idx: usize) -> bool {
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

/// Step 0 reads the run mode through the `run-mode` or `mode-status` intent,
/// (the older call forms still count).
pub fn reads_mode(text: &str) -> bool {
    text.contains("playbook mode status")
        || text.contains("playbook run-context")
        || text.contains("Intent: run-mode")
        || text.contains("Intent: mode-status")
        || text.contains("references/run-context.md")
}

// ---------------------------------------------------------------------------
// implement
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// plan
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Commands that refuse to run unattended: setup, adr, address-pr-comments
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// quick-review and deep-review: auto never posts to GitHub
// ---------------------------------------------------------------------------

/// The section that creates and submits the GitHub review.
pub fn posting_section(file: &CommandFile) -> Section {
    require_section(
        file,
        file.section(|t| t.to_lowercase().contains("orchestrate posting")),
        "`Orchestrate posting`",
    )
}

/// A shell line that calls the GitHub API to create or submit a review.
pub fn is_post_call(line: &str) -> bool {
    static POST: OnceLock<Regex> = OnceLock::new();
    POST.get_or_init(|| Regex::new(r"gh api[^\n]*(-X POST|/reviews)").expect("post pattern"))
        .is_match(line)
}

// ---------------------------------------------------------------------------
// commit-and-push
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// learn-project
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// create-pull-request
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// repo-audit, doctor, session-start: no mode block at all
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// fix
// ---------------------------------------------------------------------------

/// The `escalat` section of `fix.md`, where the hand-off rules live.
pub fn escalation_section(file: &CommandFile) -> Section {
    require_section(
        file,
        file.section(|t| t.to_lowercase().contains("escalat")),
        "escalation",
    )
}

// ---------------------------------------------------------------------------
// Comments describe the code, not the work that produced it
// ---------------------------------------------------------------------------

/// True when `text` points at internal planning: a work-unit label, a
/// numbered ticket or a planning term that means nothing to a later reader.
pub fn references_planning(text: &str) -> bool {
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
