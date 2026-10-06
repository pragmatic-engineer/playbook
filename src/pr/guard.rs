// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Mechanical text checks `pr create` runs before it pushes: no AI
//! attribution anywhere, and no em or en dashes in the title or body.

use crate::common::attribution::{attribution_hit, disallowed_trailers};

const EM_DASH: char = '\u{2014}';
const EN_DASH: char = '\u{2013}';

/// Attribution found in the title, the body, or a commit message. `commits`
/// is `(short sha, full message)`. Each hit names its place.
pub fn attribution_problems(title: &str, body: &str, commits: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    if attribution_hit(title) {
        out.push("title carries AI attribution".to_string());
    }
    for (i, line) in body.lines().enumerate() {
        if attribution_hit(line) {
            out.push(format!("body line {} carries AI attribution", i + 1));
        }
    }
    for (sha, message) in commits {
        if message.lines().any(attribution_hit) {
            out.push(format!("commit {sha} carries AI attribution"));
        } else if !disallowed_trailers(message).is_empty() {
            out.push(format!(
                "commit {sha} has a trailer other than Refs, Signed-off-by or Co-authored-by"
            ));
        }
    }
    out
}

/// Em or en dashes outside code. Lines inside a fenced block and text inside
/// inline backticks are ignored. The title is line 0, body lines start at 1.
pub fn dash_problems(title: &str, body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for ch in prose(title)
        .chars()
        .filter(|c| *c == EM_DASH || *c == EN_DASH)
    {
        out.push(format!("line 0 (title) has a {} dash", name(ch)));
    }
    let mut fenced = false;
    for (i, line) in body.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        for ch in prose(line)
            .chars()
            .filter(|c| *c == EM_DASH || *c == EN_DASH)
        {
            out.push(format!("line {} has a {} dash", i + 1, name(ch)));
        }
    }
    out
}

fn name(c: char) -> &'static str {
    if c == EM_DASH {
        "em (U+2014)"
    } else {
        "en (U+2013)"
    }
}

/// The line with every inline-backtick span removed.
fn prose(line: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for c in line.chars() {
        if c == '`' {
            in_code = !in_code;
        } else if !in_code {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(sha: &str, msg: &str) -> (String, String) {
        (sha.to_string(), msg.to_string())
    }

    #[test]
    fn clean_text_has_no_attribution_problems() {
        let got = attribution_problems(
            "feat: x",
            "## Summary\n\nbody\n",
            &[commit("abc1234", "feat: x")],
        );
        assert_eq!(got, Vec::<String>::new());
    }

    #[test]
    fn each_attribution_shape_is_found_in_its_place() {
        let body = "ok\nClaude-Session: https://x\nsee claude.ai/code/session_1\nCo-Authored-By: Claude <a@b>\nco-authored-by: Anthropic\nGenerated with Claude Code\nGenerated with [Claude Code](u)\n\u{1F916} Generated";
        let got = attribution_problems("t", body, &[]);
        let lines: Vec<String> = [2, 3, 4, 5, 6, 7, 8]
            .iter()
            .map(|n| format!("body line {n} carries AI attribution"))
            .collect();
        assert_eq!(got, lines);
    }

    #[test]
    fn title_and_commit_hits_name_their_place() {
        let got = attribution_problems(
            "Generated with Claude",
            "",
            &[
                commit("abc1234", "feat: x\n\nClaude-Session: u"),
                commit("def5678", "fine"),
            ],
        );
        assert_eq!(
            got,
            vec![
                "title carries AI attribution".to_string(),
                "commit abc1234 carries AI attribution".to_string()
            ]
        );
    }

    #[test]
    fn a_commit_trailer_outside_the_allowed_set_names_its_commit() {
        let got = attribution_problems("t", "", &[commit("abc1234", "feat: x\n\nChange-Id: I1")]);
        assert_eq!(
            got,
            vec![
                "commit abc1234 has a trailer other than Refs, Signed-off-by or Co-authored-by"
                    .to_string()
            ]
        );
    }

    #[test]
    fn a_coauthor_who_is_a_person_is_allowed() {
        let got = attribution_problems("t", "Co-Authored-By: Sam <s@x.y>", &[]);
        assert!(got.is_empty());
    }

    #[test]
    fn dashes_in_the_title_are_line_zero() {
        let got = dash_problems("feat \u{2014} x", "");
        assert_eq!(
            got,
            vec!["line 0 (title) has a em (U+2014) dash".to_string()]
        );
    }

    #[test]
    fn dashes_in_the_body_name_their_line_and_kind() {
        let got = dash_problems("t", "ok\na \u{2013} b\nfine\nc \u{2014} d");
        assert_eq!(
            got,
            vec![
                "line 2 has a en (U+2013) dash".to_string(),
                "line 4 has a em (U+2014) dash".to_string()
            ]
        );
    }

    #[test]
    fn dashes_in_fenced_and_inline_code_are_allowed() {
        let body = "```\nx \u{2014} y\n```\nuse `a \u{2013} b` here\nclean";
        assert!(dash_problems("t", body).is_empty());
    }

    #[test]
    fn plain_hyphens_are_allowed() {
        assert!(dash_problems("a-b", "c - d -- e").is_empty());
    }
}
