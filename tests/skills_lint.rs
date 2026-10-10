// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Structural lint for `skills/*/SKILL.md`, derived from Anthropic's Agent
//! Skills best practices: name matches the directory, a third-person
//! description under the length caps, a bounded body, and references that
//! stay one level deep. Descriptions sit in every session's context, so the
//! description cap is deliberately tighter than the platform's 1024.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_BODY_LINES: usize = 500;
const MAX_DESCRIPTION_CHARS: usize = 400;
const PLATFORM_DESCRIPTION_CHARS: usize = 1024;
const TOC_THRESHOLD_LINES: usize = 100;

fn skills_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("skills")
}

fn skill_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(skills_dir())
        .expect("skills dir should exist")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// Returns (name, description, body) from a SKILL.md with `---` frontmatter.
fn parse(text: &str) -> (String, String, String) {
    let rest = text.strip_prefix("---\n").expect("frontmatter must open");
    let (front, body) = rest.split_once("\n---\n").expect("frontmatter must close");
    let field = |key: &str| {
        front
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{key}:")))
            .map(|v| v.trim().trim_matches('"').to_string())
            .unwrap_or_default()
    };
    (field("name"), field("description"), body.to_string())
}

fn md_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("](") {
        rest = &rest[i + 2..];
        if let Some(j) = rest.find(')') {
            out.push(rest[..j].to_string());
        }
    }
    out
}

#[test]
fn every_skill_passes_structural_lint() {
    let mut problems = Vec::new();
    for dir in skill_dirs() {
        let dir_name = dir.file_name().unwrap().to_string_lossy().to_string();
        let text = fs::read_to_string(dir.join("SKILL.md"))
            .unwrap_or_else(|_| panic!("{dir_name} needs a SKILL.md"));
        let (name, desc, body) = parse(&text);
        if name != dir_name {
            problems.push(format!(
                "{dir_name}: name `{name}` must match the directory"
            ));
        }
        if name.len() > 64
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            || name.contains("anthropic")
            || name.contains("claude")
        {
            problems.push(format!("{dir_name}: invalid name `{name}`"));
        }
        if desc.is_empty() {
            problems.push(format!("{dir_name}: description is empty"));
        }
        if desc.chars().count() > MAX_DESCRIPTION_CHARS {
            problems.push(format!(
                "{dir_name}: description is {} chars, cap {MAX_DESCRIPTION_CHARS} (platform {PLATFORM_DESCRIPTION_CHARS})",
                desc.chars().count()
            ));
        }
        if desc.contains('<') || desc.contains('>') {
            problems.push(format!("{dir_name}: description must not contain XML tags"));
        }
        let lines = body.lines().count();
        if lines > MAX_BODY_LINES {
            problems.push(format!(
                "{dir_name}: body is {lines} lines, cap {MAX_BODY_LINES}"
            ));
        }
        let refs = dir.join("references");
        if let Ok(entries) = fs::read_dir(&refs) {
            for entry in entries.filter_map(Result::ok) {
                let p = entry.path();
                if p.is_dir() {
                    problems.push(format!("{dir_name}: references/ must be flat, found {p:?}"));
                    continue;
                }
                let t = fs::read_to_string(&p).unwrap_or_default();
                let fname = p.file_name().unwrap().to_string_lossy().to_string();
                for link in md_links(&t) {
                    if !link.starts_with("http") && link.contains(".md") && !link.starts_with('#') {
                        problems.push(format!(
                            "{dir_name}/references/{fname}: links to `{link}`, references must be one level deep"
                        ));
                    }
                }
                if t.lines().count() > TOC_THRESHOLD_LINES && !t.contains("## Contents") {
                    problems.push(format!(
                        "{dir_name}/references/{fname}: over {TOC_THRESHOLD_LINES} lines needs a `## Contents` section"
                    ));
                }
            }
        }
    }
    assert!(
        problems.is_empty(),
        "skill lint failures:\n{}",
        problems.join("\n")
    );
}

fn read_skill(rel: &str) -> String {
    fs::read_to_string(skills_dir().join(rel)).unwrap_or_else(|_| panic!("{rel} should exist"))
}

#[test]
fn lens_reviewers_do_not_load_the_orchestrator_sections() {
    let skill = read_skill("grounding-review/SKILL.md");
    for heading in [
        "## Review Report Format",
        "## Verification Sweep",
        "## Subject Lines",
        "## Severity",
    ] {
        assert!(
            !skill.contains(heading),
            "grounding-review SKILL.md must not carry `{heading}`, it is orchestrator-only"
        );
    }
    let orch = read_skill("grounding-review/references/orchestrator.md");
    assert!(orch.contains("## Review Report Format"));
    assert!(orch.contains("## Verification Sweep (MUST)"));
    assert!(skill.contains("playbook skill ref grounding-review orchestrator"));
}

#[test]
fn writing_style_keeps_the_examples_heading_stub() {
    let skill = read_skill("writing-style/SKILL.md");
    assert!(
        skill.lines().any(|l| l.starts_with("## Examples")),
        "src/pr/rules.rs ends the GitHub slice at this heading"
    );
    for name in ["examples", "imperfections", "shell-quoting"] {
        assert!(skill.contains(&format!("playbook skill ref writing-style {name}")));
        read_skill(&format!("writing-style/references/{name}.md"));
    }
}
