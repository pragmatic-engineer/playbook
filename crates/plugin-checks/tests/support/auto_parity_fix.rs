// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The comment and ask-branch checks of `commands_fix.rs`.

use super::ap::{heading_title, require_section, CommandFile, Section};
use regex::Regex;
use std::sync::OnceLock;

impl CommandFile {
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

/// The `escalat` section of `fix.md`, where the hand-off rules live.
pub fn escalation_section(file: &CommandFile) -> Section {
    require_section(
        file,
        file.section(|t| t.to_lowercase().contains("escalat")),
        "escalation",
    )
}

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
