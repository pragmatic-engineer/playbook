// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! A mechanical check of the writing-style rules that a regex can decide, for
//! text a person will read: a dash, a curly quote, a banned word, and the
//! "it's not X, it's Y" contrast frame. It backs the prose rules with a
//! rejection that names the line, so the rules need not be carried in the
//! prompt of every agent that writes a message.
//!
//! Fenced blocks, inline code and trailer lines are skipped. The word list is
//! the unambiguous part of the skill's Banned Words: a word that has a real
//! technical meaning (harness, unlock, landscape) is left to the model.

use super::re::static_regex;
use regex::Regex;
use std::sync::LazyLock;

/// Banned words and their common forms, matched on whole words.
pub const BANNED: &[&str] = &[
    "delve",
    "delves",
    "delving",
    "embark",
    "embarks",
    "embarking",
    "tapestry",
    "illuminate",
    "unveil",
    "unveils",
    "elucidate",
    "furthermore",
    "additionally",
    "moreover",
    "however",
    "hence",
    "groundbreaking",
    "cutting-edge",
    "breathtaking",
    "game-changer",
    "revolutionize",
    "utilize",
    "utilizes",
    "utilized",
    "utilizing",
    "leverage",
    "leverages",
    "leveraged",
    "leveraging",
    "facilitate",
    "facilitates",
    "facilitated",
    "numerous",
    "pivotal",
    "intricate",
    "testament",
    "nestled",
    "vibrant",
    "profound",
    "showcase",
    "showcases",
    "showcased",
    "showcasing",
    "underscore",
    "underscores",
    "underscoring",
    "ever-evolving",
];

static WORDS: LazyLock<Regex> =
    LazyLock::new(|| static_regex(&format!(r"(?i)\b(?:{})\b", BANNED.join("|"))));

static CODE_SPAN: LazyLock<Regex> = LazyLock::new(|| static_regex(r"`[^`\n]*`"));

/// "it's not X, it's Y" and "not just X, but also Y".
static CONTRAST: LazyLock<Regex> = LazyLock::new(|| {
    static_regex(
        r"(?i)\bnot just\b[^.\n]{0,80}\bbut (?:also )?\b|\b(?:it|this|that)(?:'s| is| was) not (?:about )?[^.\n;,]{1,60}[,;]\s+(?:it|this|that)(?:'s| is| was)\b|\b(?:it|this|that) (?:isn't|wasn't) (?:about )?[^.\n;,]{1,60}[,;]\s+(?:it|this|that)(?:'s| is| was)\b",
    )
});

fn is_trailer(line: &str) -> bool {
    let l = line.trim_start();
    l.starts_with("Signed-off-by:") || l.starts_with("Co-authored-by:")
}

/// One message per problem, each starting `line N:`. Empty when the text is
/// clean.
pub fn check(text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut fenced = false;
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        if raw.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced || is_trailer(raw) {
            continue;
        }
        let line = CODE_SPAN.replace_all(raw, " ");
        if line.contains(['\u{2014}', '\u{2013}']) {
            problems.push(format!(
                "line {n}: an em or en dash. Use a comma, a colon or a new sentence."
            ));
        }
        if line.contains(['\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}']) {
            problems.push(format!("line {n}: a curly quote. Use a straight quote."));
        }
        for m in WORDS.find_iter(&line) {
            problems.push(format!(
                "line {n}: the banned word \"{}\". Say it plainly.",
                m.as_str()
            ));
        }
        if let Some(m) = CONTRAST.find(&line) {
            problems.push(format!(
                "line {n}: contrast framing \"{}\". State the point directly.",
                m.as_str().trim()
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_has_no_problems() {
        assert!(
            check("fix(api): retry the call once\n\nThe client gave up too early.\n").is_empty()
        );
    }

    #[test]
    fn a_dash_is_found_with_its_line() {
        let p = check("fix: a b\n\nwords \u{2014} more\n");
        assert_eq!(p.len(), 1);
        assert!(p[0].starts_with("line 3:"), "{p:?}");
        assert_eq!(check("a \u{2013} b").len(), 1);
    }

    #[test]
    fn curly_quotes_are_found() {
        assert_eq!(check("it\u{2019}s done").len(), 1);
    }

    #[test]
    fn banned_words_match_whole_words_only() {
        assert_eq!(check("Leverage the cache").len(), 1);
        assert_eq!(check("utilizing it and delving in").len(), 2);
        assert!(check("the leverager and hencefort").is_empty());
    }

    #[test]
    fn code_spans_fences_and_trailers_are_skipped() {
        let t = "fix: x\n\nThe `utilize` flag \u{2014} no.\n";
        assert_eq!(check(t).len(), 1, "only the bare dash counts");
        assert!(check("fix: x\n\n```\nleverage \u{2014} x\n```\n").is_empty());
        assert!(check("fix: x\n\nSigned-off-by: A \u{201C}B\u{201D} <a@b.c>\n").is_empty());
    }

    #[test]
    fn the_contrast_frame_is_found() {
        for s in [
            "It's not a cache problem, it's a lock problem.",
            "This isn't about speed, it's about safety.",
            "This is not about speed; this is about safety.",
            "Not just faster, but also safer.",
        ] {
            assert_eq!(check(s).len(), 1, "{s}");
        }
    }

    #[test]
    fn ordinary_negation_is_left_alone() {
        for s in [
            "The cache is not cleared on exit.",
            "It's not found, so return early.",
            "Do not retry. It's safe to call twice.",
            "Skip the lock, but keep the log.",
        ] {
            assert!(check(s).is_empty(), "{s}: {:?}", check(s));
        }
    }

    #[test]
    fn every_listed_word_is_in_the_skills_banned_words() {
        let skill = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/writing-style/SKILL.md"),
        )
        .unwrap();
        let section = &skill[skill.find("## Banned Words").unwrap()..];
        let section = &section[..section.find("\n---").unwrap()];
        for w in BANNED {
            let stem: String = w.chars().take(4).collect();
            let stem = stem.as_str();
            assert!(section.contains(stem), "{w} is not in the Banned Words");
        }
    }
}
