// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Reads the first line of a review comment written in the Conventional
//! Comments format: `<label> [decorations]: <subject>`. The old bare
//! `blocking:` form is still read as `issue (blocking)`. Only the new form is
//! ever written.

/// The nine labels of the standard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    Nitpick,
    Suggestion,
    Issue,
    Todo,
    Question,
    Thought,
    Chore,
    Typo,
    Polish,
}

/// What a reader should do with a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    MustFix,
    Optional,
    NeedsAnswer,
    Judge,
}

/// A parsed comment head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub label: Label,
    pub decorations: Vec<String>,
    pub subject: String,
}

impl Parsed {
    pub fn is_blocking(&self) -> bool {
        self.decorations.iter().any(|d| d == "blocking")
    }

    /// `(blocking)` wins, then `(non-blocking)`, then the label default.
    pub fn action(&self) -> Action {
        if self.is_blocking() {
            return Action::MustFix;
        }
        if self.decorations.iter().any(|d| d == "non-blocking") {
            return Action::Optional;
        }
        match self.label {
            Label::Issue | Label::Chore | Label::Todo | Label::Typo => Action::MustFix,
            Label::Nitpick | Label::Thought | Label::Polish => Action::Optional,
            Label::Question => Action::NeedsAnswer,
            Label::Suggestion => Action::Judge,
        }
    }
}

fn label_of(word: &str) -> Option<Label> {
    Some(match word {
        "nitpick" => Label::Nitpick,
        "suggestion" => Label::Suggestion,
        "issue" => Label::Issue,
        "todo" => Label::Todo,
        "question" => Label::Question,
        "thought" => Label::Thought,
        "chore" => Label::Chore,
        "typo" => Label::Typo,
        "polish" => Label::Polish,
        _ => return None,
    })
}

/// Parses the first line of `body`. `None` when it has no known label.
pub fn parse(body: &str) -> Option<Parsed> {
    let line = body.trim_start().lines().next()?;
    let (head, subject) = line.split_once(':')?;
    let head = head.trim().to_lowercase();
    let (word, decorations) = match head.split_once('(') {
        Some((word, rest)) => {
            let inner = rest.strip_suffix(')')?;
            let list = inner
                .split(',')
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty())
                .collect();
            (word.trim().to_string(), list)
        }
        None => (head, Vec::new()),
    };
    if word == "blocking" && decorations.is_empty() {
        return Some(Parsed {
            label: Label::Issue,
            decorations: vec!["blocking".into()],
            subject: subject.trim().into(),
        });
    }
    Some(Parsed {
        label: label_of(&word)?,
        decorations,
        subject: subject.trim().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn must(body: &str) -> Parsed {
        parse(body).unwrap_or_else(|| panic!("no label in {body}"))
    }

    #[test]
    fn a_label_with_one_decoration_is_parsed() {
        let p = must("issue (blocking): the loop never ends");
        assert_eq!(p.label, Label::Issue);
        assert_eq!(p.decorations, vec!["blocking"]);
        assert_eq!(p.subject, "the loop never ends");
    }

    #[test]
    fn a_label_with_several_decorations_keeps_them_in_order() {
        let p = must("suggestion (security, non-blocking): hash it");
        assert_eq!(p.decorations, vec!["security", "non-blocking"]);
        assert_eq!(p.action(), Action::Optional);
    }

    #[test]
    fn a_label_without_decoration_uses_its_default_action() {
        let p = must("typo: recieve");
        assert!(p.decorations.is_empty());
        assert_eq!(p.action(), Action::MustFix);
    }

    #[test]
    fn an_unknown_label_is_not_parsed() {
        assert_eq!(parse("praise: nice work"), None);
    }

    #[test]
    fn an_uppercase_label_is_parsed() {
        let p = must("NITPICK (Non-Blocking): spacing");
        assert_eq!(p.label, Label::Nitpick);
        assert_eq!(p.action(), Action::Optional);
    }

    #[test]
    fn the_old_bare_blocking_form_reads_as_a_blocking_issue() {
        let p = must("blocking: no retry limit");
        assert_eq!(p.label, Label::Issue);
        assert!(p.is_blocking());
        assert_eq!(p.action(), Action::MustFix);
    }

    #[test]
    fn a_question_needs_an_answer_and_a_suggestion_is_judged() {
        assert_eq!(
            must("question: can it be empty?").action(),
            Action::NeedsAnswer
        );
        assert_eq!(must("suggestion: use a map").action(), Action::Judge);
    }

    #[test]
    fn only_the_first_line_is_read() {
        let p = must("thought: later\n\nissue: not this");
        assert_eq!(p.label, Label::Thought);
    }
}
