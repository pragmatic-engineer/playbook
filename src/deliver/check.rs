// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The text rules for a commit message, a PR title and a PR body. The AI
//! attribution rule is the one the commit-message sanitizer hook uses
//! (`common::attribution`), so both refuse the same lines.

use crate::common::attribution;

/// A title is refused above this many characters, prefix included.
pub const TITLE_LIMIT: usize = 72;

const TYPES: &[&str] = &[
    "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert",
];

/// Whether `title` is `type(scope)!: description` with a known type.
pub fn is_conventional(title: &str) -> bool {
    let Some((head, rest)) = title.split_once(": ") else {
        return false;
    };
    if rest.trim().is_empty() {
        return false;
    }
    let head = head.strip_suffix('!').unwrap_or(head);
    let kind = match head.split_once('(') {
        Some((kind, scope)) => {
            if !scope.ends_with(')') || scope.len() < 2 {
                return false;
            }
            kind
        }
        None => head,
    };
    TYPES.contains(&kind)
}

fn dash_lines(text: &str, label: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fenced = false;
    for (i, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let mut in_code = false;
        for c in line.chars() {
            if c == '`' {
                in_code = !in_code;
            } else if !in_code && (c == '\u{2014}' || c == '\u{2013}') {
                let name = if c == '\u{2014}' { "em" } else { "en" };
                out.push(format!("{label} line {} has an {name} dash", i + 1));
            }
        }
    }
    out
}

fn attribution_lines(text: &str, label: &str, prose: bool) -> Vec<String> {
    let found = if prose {
        attribution::prose_problems(text)
    } else {
        attribution::problems(text)
    };
    found
        .into_iter()
        .map(|(line, shape)| {
            format!(
                "{label} line {} carries AI attribution ({})",
                line + 1,
                shape.name()
            )
        })
        .collect()
}

/// Every problem with a commit message, empty when it is fine.
pub fn commit_message(message: &str) -> Vec<String> {
    let mut out = Vec::new();
    let title = message.lines().next().unwrap_or("").trim();
    if title.is_empty() {
        out.push("commit message is empty".to_string());
        return out;
    }
    out.extend(title_problems(title, "commit title"));
    out.extend(dash_lines(message, "commit message"));
    out.extend(attribution_lines(message, "commit message", false));
    out
}

/// Every problem with a PR title, empty when it is fine.
pub fn pr_title(title: &str) -> Vec<String> {
    let mut out = title_problems(title, "PR title");
    out.extend(dash_lines(title, "PR title"));
    out.extend(attribution_lines(title, "PR title", true));
    out
}

/// Every problem with a PR body, empty when it is fine.
pub fn pr_body(body: &str) -> Vec<String> {
    let mut out = dash_lines(body, "PR body");
    out.extend(attribution_lines(body, "PR body", true));
    out
}

fn title_problems(title: &str, label: &str) -> Vec<String> {
    let mut out = Vec::new();
    let length = title.chars().count();
    if length > TITLE_LIMIT {
        out.push(format!(
            "{label} is {length} characters, the limit is {TITLE_LIMIT}"
        ));
    }
    if !is_conventional(title) {
        out.push(format!(
            "{label} is not a conventional commit title (type(scope): description)"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scoped_conventional_title_passes() {
        assert!(commit_message("feat(deliver): add the command").is_empty());
    }

    #[test]
    fn a_title_without_a_type_is_refused() {
        assert!(!commit_message("add the command").is_empty());
    }

    #[test]
    fn a_title_over_the_limit_is_refused_with_its_length() {
        let title = format!("feat: {}", "x".repeat(70));

        let problems = pr_title(&title);

        assert!(
            problems.iter().any(|p| p.contains("76 characters")),
            "got {problems:?}"
        );
    }

    #[test]
    fn an_em_dash_in_the_body_of_a_commit_is_refused() {
        let problems = commit_message("fix: a thing\n\nbecause \u{2014} reasons");

        assert!(
            problems.iter().any(|p| p.contains("em dash")),
            "got {problems:?}"
        );
    }

    #[test]
    fn a_dash_inside_code_is_allowed() {
        assert!(pr_body("Use `a \u{2013} b` here\n\n```\nx \u{2014} y\n```\n").is_empty());
    }

    #[test]
    fn a_claude_trailer_is_refused() {
        let problems =
            commit_message("fix: a thing\n\nCo-Authored-By: Claude <noreply@anthropic.com>");

        assert!(
            problems.iter().any(|p| p.contains("AI attribution")),
            "got {problems:?}"
        );
    }
}
