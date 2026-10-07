// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Structural checks for the verification sweep that every review flow runs
//! before findings are shown or posted.
//!
//! Each file that presents review findings must carry a heading-delimited
//! sweep section holding the three checks (true, label, anchor) and the
//! counts line. The two review commands must also run the sweep before the
//! section that presents or posts. These tests parse headings outside fenced
//! code instead of grepping, so a marker in prose or an example cannot pass
//! for a real section. Model behaviour is not testable here: they cover prose
//! structure only.

use std::fs;
use std::path::PathBuf;

/// Files that carry the sweep as a section of their own.
const SWEEP_FILES: &[&str] = &[
    "skills/grounding-review/SKILL.md",
    "commands/quick-review.md",
    "commands/deep-review.md",
    "commands/implement.md",
];

/// Reviewer agents that tell their output will be swept.
const SWEPT_AGENTS: &[&str] = &["agents/reviewer.md", "agents/cheap-checker.md"];

/// The three checks, as the numbered lead of each item.
const CHECKS: &[&str] = &["1. **True.**", "2. **Label.**", "3. **Anchor.**"];

struct Doc {
    path: String,
    lines: Vec<String>,
}

struct Heading {
    level: usize,
    title: String,
    line: usize,
}

/// A heading and the lines it owns, `start..end` (end exclusive).
struct Section {
    start: usize,
    end: usize,
}

fn load(rel: &str) -> Doc {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    Doc {
        path: rel.to_owned(),
        lines: text.lines().map(str::to_owned).collect(),
    }
}

impl Doc {
    /// Markdown headings outside fenced code blocks, so a `# comment` in a
    /// shell example is not mistaken for a section.
    fn headings(&self) -> Vec<Heading> {
        let mut out = Vec::new();
        let mut in_fence = false;
        for (line, text) in self.lines.iter().enumerate() {
            if text.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            let level = text.chars().take_while(|&c| c == '#').count();
            if !in_fence && level > 0 && text[level..].starts_with(' ') {
                out.push(Heading {
                    level,
                    title: text[level..].trim().to_owned(),
                    line,
                });
            }
        }
        out
    }

    /// The first section whose heading title satisfies `matches`. A section
    /// ends at the next heading of the same or a shallower level.
    fn section(&self, matches: impl Fn(&str) -> bool) -> Option<Section> {
        let headings = self.headings();
        let idx = headings.iter().position(|h| matches(&h.title))?;
        let end = headings[idx + 1..]
            .iter()
            .find(|h| h.level <= headings[idx].level)
            .map_or(self.lines.len(), |h| h.line);
        Some(Section {
            start: headings[idx].line,
            end,
        })
    }

    fn sweep(&self) -> Option<Section> {
        self.section(|t| t.to_lowercase().contains("verification sweep"))
    }

    fn step(&self, prefix: &str) -> Section {
        self.section(|t| t.starts_with(prefix))
            .unwrap_or_else(|| panic!("{} has no section titled `{prefix}...`", self.path))
    }

    fn text_of(&self, s: &Section) -> String {
        self.lines[s.start..s.end].join("\n")
    }
}

#[test]
fn every_review_flow_has_a_sweep_section_with_the_three_checks() {
    for rel in SWEEP_FILES {
        // Arrange
        let doc = load(rel);

        // Act
        let sweep = doc
            .sweep()
            .unwrap_or_else(|| panic!("{rel} has no verification sweep section"));
        let text = doc.text_of(&sweep);

        // Assert
        for check in CHECKS {
            assert!(text.contains(check), "{rel}: sweep lacks `{check}`");
        }
        assert!(
            text.contains("kept, dropped, relabelled"),
            "{rel}: sweep does not say it reports kept, dropped, relabelled and moved counts"
        );
        assert!(
            text.contains("[unverified]"),
            "{rel}: sweep does not say what happens to an `[unverified]` finding"
        );
    }
}

#[test]
fn quick_review_sweeps_before_it_relays_or_posts() {
    // Arrange
    let doc = load("commands/quick-review.md");

    // Act
    let contract = doc.step("Step 3:");
    let sweep = doc.sweep().expect("quick-review has a sweep section");
    let posting = doc.step("Step 4:");

    // Assert
    assert!(
        contract.start < sweep.start && sweep.start < posting.start,
        "quick-review: the sweep must sit after the report contract and before posting"
    );
    let text = doc.lines.join("\n");
    assert!(
        !text.contains("Relay the report to the user unchanged"),
        "quick-review still relays the reviewer report unchanged"
    );
    assert!(
        !text.contains("the orchestrator does NOT re-read source files"),
        "quick-review still forbids the orchestrator from re-reading cited lines"
    );
}

#[test]
fn deep_review_sweeps_before_it_presents_or_posts() {
    // Arrange
    let doc = load("commands/deep-review.md");

    // Act
    let consolidate = doc.step("Step 4:");
    let sweep = doc.sweep().expect("deep-review has a sweep section");
    let present = doc.step("Step 5:");
    let posting = doc.step("Step 6:");

    // Assert
    assert!(
        consolidate.start < sweep.start,
        "deep-review: the sweep must follow consolidation"
    );
    assert!(
        sweep.start < present.start && sweep.start < posting.start,
        "deep-review: the sweep must come before presenting or posting"
    );
}

#[test]
fn implement_sweeps_inside_step_9_before_acting_on_findings() {
    // Arrange
    let doc = load("commands/implement.md");

    // Act
    let step9 = doc.step("Step 9:");
    let sweep = doc.sweep().expect("implement has a sweep section");
    let act = doc
        .section(|t| t.starts_with("Fix, open the PRs"))
        .expect("implement has a section that applies the fixes");

    // Assert
    assert!(
        step9.start < sweep.start && sweep.end <= step9.end,
        "implement: the sweep must live inside Step 9"
    );
    assert!(
        sweep.start < act.start,
        "implement: the sweep must come before the fixes are applied"
    );
}

#[test]
fn every_review_flow_recomputes_the_verdict_from_the_swept_list() {
    for rel in SWEEP_FILES {
        // Arrange
        let doc = load(rel);

        // Act
        let text = doc.lines.join("\n");

        // Assert
        assert!(
            text.contains(
                "recompute the verdict, confidence and finding order from the swept list"
            ),
            "{rel}: does not recompute the verdict, confidence and order after the sweep"
        );
    }
}

#[test]
fn quick_review_sweep_traces_but_never_runs_pr_code() {
    // Arrange
    let doc = load("commands/quick-review.md");

    // Act
    let sweep = doc.sweep().expect("quick-review has a sweep section");
    let text = doc.text_of(&sweep);

    // Assert
    assert!(
        text.contains("Trace the failure scenario"),
        "quick-review: the sweep does not say to trace the failure scenario"
    );
    assert!(
        !text.contains("or run"),
        "quick-review: the sweep must not tell the orchestrator to run PR code"
    );
    assert!(
        text.contains("never run PR code"),
        "quick-review: the sweep does not forbid running PR code"
    );
}

#[test]
fn implement_step_8_sweeps_before_it_fixes() {
    // Arrange
    let doc = load("commands/implement.md");

    // Act
    let text = doc.text_of(&doc.step("Step 8:"));
    let sweep_at = text.find("verification sweep");
    let fix_at = text.find("Fix only");

    // Assert
    let (sweep_at, fix_at) = (
        sweep_at.expect("implement Step 8 does not mention the verification sweep"),
        fix_at.expect("implement Step 8 has no fix instruction"),
    );
    assert!(
        sweep_at < fix_at,
        "implement Step 8: the sweep must come before the fix instruction"
    );
}

#[test]
fn reviewer_agents_say_their_findings_are_swept() {
    for rel in SWEPT_AGENTS {
        // Arrange
        let doc = load(rel);

        // Act
        let text = doc.lines.join("\n");

        // Assert
        assert!(
            text.contains("The orchestrator sweeps every finding you return"),
            "{rel}: does not tell the reviewer its findings will be swept"
        );
    }
}

#[test]
fn sweep_detector_finds_nothing_in_a_file_without_one() {
    // Arrange: a fenced heading is not a section, and nothing else matches.
    let doc = Doc {
        path: "fixture".to_owned(),
        lines: vec![
            "## Step 3: Report".to_owned(),
            "```".to_owned(),
            "## Verification sweep".to_owned(),
            "```".to_owned(),
        ],
    };

    // Act
    let sweep = doc.sweep();

    // Assert
    assert!(sweep.is_none(), "a fenced heading must not count");
}
