// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports hooks/auto-model-detect.py: a UserPromptSubmit hook that nudges the
//! main session toward delegating design/architecture-shaped prompts to an
//! Opus subagent, rather than reasoning inline on the default model.
//!
//! The python hook matches the prompt against one large case-insensitive,
//! word-boundary regex. SEGMENT-B-RULES.md forbids adding a `regex`
//! dependency for this port, so the alternation is expanded by hand into a
//! list of literal phrases plus one wildcard phrase, matched with an
//! explicit ASCII word-boundary scan that mirrors `\bphrase\b` semantics.
//!
//! One python oddity worth flagging: in the source regex, the `decompos`
//! alternative sits inside `\b(...|decompos|...)\b`, so the trailing `\b`
//! applies to it too. That means `decompos` never actually matches
//! "decompose" or "decomposition" (there is no word boundary between the
//! "s" and the following "e"/"i"); it only matches the literal standalone
//! token "decompos". That quirk is preserved here rather than fixed.

use crate::common::emit_prompt_context;
use crate::common::payload::Payload;

const MSG: &str = r#"This prompt looks like design / architecture work. Your main session runs on the default model. Before reasoning inline, consider delegating to an Opus subagent, its full deliberation stays in the subagent's context, only the conclusion returns to yours.

Recommended for design-heavy prompts:
  - Plan (Agent tool, `model: "opus"`), implementation planning and architecture with codebase grounding
  - /playbook:brainstorm, ideation / requirements before any code

If the prompt is actually small-scope (e.g. quick choice between two named options), staying on Sonnet inline is fine. Use judgment.

Routing policy: Opus only when Sonnet wasn't enough, keep Opus under 20% of total usage. Routine/mechanical/formatting/search subagents default to Haiku (3x cheaper); escalate to Sonnet for real coding."#;

/// Literal phrases the python regex's alternation expands to, once every
/// `?` optional group and `(a|b|c)` nested alternation is enumerated. Every
/// entry here is matched with a word boundary at its start and end, never
/// as a bare substring. Lowercase, since matching is case-insensitive.
const PHRASES: &[&str] = &[
    // Design/architecture nouns.
    "design",
    "architect",
    "architecture",
    "adr",
    "tradeoff",
    "tradeoffs",
    "alternative",
    "alternatives",
    "approach",
    "strategy",
    "paradigm",
    "pattern",
    "abstraction",
    "refactor plan",
    "migration",
    "decompos",
    "schema",
    "modeling",
    "data model",
    "contract",
    "interface design",
    // Decision verbs.
    "evaluate",
    "compare",
    "brainstorm",
    "propose",
    "recommend",
    "critique",
    "review the approach",
    "review the design",
    // Design-shaped questions, excluding "what.?s the best" which needs a
    // wildcard character and is handled by `matches_whats_the_best`.
    "should we",
    "how would we",
    "how would you",
    "how would i",
    "how should we",
    "how should you",
    "how should i",
    "which approach",
    "which design",
    "which pattern",
    "trade off",
    "pros and cons",
];

/// Phrases naming the pipeline's ideation stage: raw idea, no plan yet.
const BRAINSTORM_DIRECTIVE_PHRASES: &[&str] = &[
    "let's brainstorm this",
    "brainstorm this",
    "explore this idea",
    "not sure how to approach",
    "what are our options",
    "what are the options",
];

const BRAINSTORM_MSG: &str = r#"This prompt looks like ideation, exploring a raw idea with no plan yet. Consider running /playbook:brainstorm to work through the options before committing to a direction."#;

/// Phrases naming the pipeline's planning stage: a direction exists, ready
/// to become a concrete plan.
const SCOPE_DIRECTIVE_PHRASES: &[&str] = &[
    "let's plan this",
    "let's scope this",
    "break this down",
    "let's turn that into a plan",
    "how would we build",
    "what would it take to build",
];

const SCOPE_MSG: &str = r#"This prompt looks ready to turn a direction into a concrete plan. Consider running /playbook:scope to produce a verified implementation plan before writing code."#;

/// Phrases naming the pipeline's decision-record stage: a consequential,
/// hard-to-reverse call worth documenting.
const ADR_DIRECTIVE_PHRASES: &[&str] = &[
    "this is a big call",
    "let's not rush this",
    "we need to decide this properly",
    "document why we picked",
    "expensive to undo",
    "hard to reverse",
    "hard-to-reverse",
];

const ADR_MSG: &str = r#"This prompt looks like a consequential, hard-to-reverse decision. Consider running /playbook:adr to record it properly, with the reasoning captured for later."#;

/// Phrases naming the pipeline's execution stage: an approved plan or
/// decision already exists.
const IMPLEMENT_DIRECTIVE_PHRASES: &[&str] = &[
    "let's implement this",
    "let's build this",
    "go ahead",
    "ship it",
    "make it happen",
    "start building",
    "let's pick this back up",
    "start on it",
];

const IMPLEMENT_MSG: &str = r#"This prompt looks like it's ready to build: an approved plan or decision already exists. Consider running /playbook:implement to execute it, rather than writing code inline from a standing start."#;

/// UserPromptSubmit entry point. Never panics: a missing prompt, a slash
/// command, a short prompt, or plain prose all fall through silently.
pub fn run(payload: &Payload) {
    let prompt = payload.field(".prompt");
    if prompt.is_empty() || prompt.starts_with('/') {
        return;
    }
    let lower: Vec<char> = prompt.to_lowercase().chars().collect();

    // Checked brainstorm, adr, scope, implement so an adr-shaped prompt isn't misread as scope.
    for (phrases, wildcard, msg) in [
        (
            BRAINSTORM_DIRECTIVE_PHRASES,
            None::<fn(&[char]) -> bool>,
            BRAINSTORM_MSG,
        ),
        (
            ADR_DIRECTIVE_PHRASES,
            Some(matches_named_alternatives as fn(&[char]) -> bool),
            ADR_MSG,
        ),
        (SCOPE_DIRECTIVE_PHRASES, None, SCOPE_MSG),
        (IMPLEMENT_DIRECTIVE_PHRASES, None, IMPLEMENT_MSG),
    ] {
        let hit = phrases.iter().any(|phrase| {
            let needle: Vec<char> = phrase.chars().collect();
            word_boundary_contains(&lower, &needle)
        }) || wildcard.is_some_and(|matches| matches(&lower));
        if hit {
            emit_prompt_context(msg);
            return;
        }
    }

    if prompt.chars().count() < 20 {
        return;
    }
    if !has_design_intent(&lower) {
        return;
    }
    emit_prompt_context(MSG);
}

fn has_design_intent(lower: &[char]) -> bool {
    let phrase_hit = PHRASES.iter().any(|phrase| {
        let needle: Vec<char> = phrase.chars().collect();
        word_boundary_contains(lower, &needle)
    });
    phrase_hit || matches_whats_the_best(lower)
}

/// Matches python's `\w` under `re.IGNORECASE` with no `re.ASCII` flag: any
/// Unicode letter or digit counts as a word character, not only ASCII ones.
/// An ASCII-only check here would make Rust see a word boundary where
/// python does not (CJK text has no spaces, and accented Latin is common),
/// so Rust could fire where python stays silent.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// True if `needle` occurs anywhere in `haystack` with a word boundary
/// immediately before and immediately after the match, mirroring
/// `\bneedle\b`. Internal characters of `needle` (including spaces for
/// multi-word phrases) are matched literally.
fn word_boundary_contains(haystack: &[char], needle: &[char]) -> bool {
    word_boundary_index(haystack, needle).is_some()
}

/// Like `word_boundary_contains`, but returns the match's start index, so a caller can search past it.
fn word_boundary_index(haystack: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .enumerate()
        .find_map(|(start, window)| {
            if window != needle {
                return None;
            }
            let before_ok = start == 0 || !is_word_char(haystack[start - 1]);
            let end = start + needle.len();
            let after_ok = end == haystack.len() || !is_word_char(haystack[end]);
            (before_ok && after_ok).then_some(start)
        })
}

/// True for "X vs Y" or "should we use X or Y", the canonical ADR-trigger examples named in SYSTEM_PROMPT.md.
fn matches_named_alternatives(haystack: &[char]) -> bool {
    let vs: Vec<char> = "vs".chars().collect();
    if word_boundary_contains(haystack, &vs) {
        return true;
    }
    let should_we_use: Vec<char> = "should we use".chars().collect();
    let Some(start) = word_boundary_index(haystack, &should_we_use) else {
        return false;
    };
    let or_word: Vec<char> = "or".chars().collect();
    word_boundary_contains(&haystack[start + should_we_use.len()..], &or_word)
}

/// True for a word-bounded occurrence of "what.?s the best", where `.?`
/// means zero or one arbitrary character, mirroring the python regex's
/// `what.?s the best`.
fn matches_whats_the_best(haystack: &[char]) -> bool {
    let prefix: Vec<char> = "what".chars().collect();
    let suffix: Vec<char> = "s the best".chars().collect();
    let n = haystack.len();
    if n < prefix.len() {
        return false;
    }
    haystack
        .windows(prefix.len())
        .enumerate()
        .any(|(start, window)| {
            if window != prefix.as_slice() {
                return false;
            }
            if start != 0 && is_word_char(haystack[start - 1]) {
                return false;
            }
            let after_prefix = start + prefix.len();
            (0..=1usize).any(|wildcard_len| {
                let suffix_start = after_prefix + wildcard_len;
                let end = suffix_start + suffix.len();
                end <= n
                    && haystack[suffix_start..end] == suffix[..]
                    && (end == n || !is_word_char(haystack[end]))
            })
        })
}
