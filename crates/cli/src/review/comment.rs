// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Reads the first line of a review comment written in the Conventional
//! Comments format: `<label> [decorations]: <subject>`. The old bare
//! `blocking:` form is still read as `issue (blocking)`, and a bare `issue:`
//! reads as must-fix like every `issue`. Only the new form is ever written.

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
pub struct CommentHead {
    pub label: Label,
    pub decorations: Vec<String>,
    pub subject: String,
}

impl CommentHead {
    /// True when the comment must be resolved before acceptance, by an
    /// explicit `(blocking)` or by the label default.
    pub fn is_blocking(&self) -> bool {
        self.action() == Action::MustFix
    }

    fn has_decoration(&self, name: &str) -> bool {
        self.decorations.iter().any(|d| d == name)
    }

    /// `(blocking)` wins, then `(non-blocking)`, then the label default.
    pub fn action(&self) -> Action {
        if self.has_decoration("blocking") {
            return Action::MustFix;
        }
        if self.has_decoration("non-blocking") {
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
pub fn parse_head(body: &str) -> Option<CommentHead> {
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
        return Some(CommentHead {
            label: Label::Issue,
            decorations: vec!["blocking".into()],
            subject: subject.trim().into(),
        });
    }
    Some(CommentHead {
        label: label_of(&word)?,
        decorations,
        subject: subject.trim().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn must(body: &str) -> CommentHead {
        parse_head(body).unwrap_or_else(|| panic!("no label in {body}"))
    }

    #[test]
    fn a_label_with_one_decoration_is_parsed() {
        // Arrange
        let body = "issue (blocking): the loop never ends";
        // Act
        let p = must(body);
        // Assert
        assert_eq!(p.label, Label::Issue);
        assert_eq!(p.decorations, vec!["blocking"]);
        assert_eq!(p.subject, "the loop never ends");
    }

    #[test]
    fn a_label_with_several_decorations_keeps_them_in_order() {
        // Arrange
        let body = "suggestion (security, non-blocking): hash it";
        // Act
        let p = must(body);
        // Assert
        assert_eq!(p.decorations, vec!["security", "non-blocking"]);
        assert_eq!(p.action(), Action::Optional);
    }

    #[test]
    fn a_label_without_decoration_has_none() {
        // Arrange
        let body = "typo: recieve";
        // Act
        let p = must(body);
        // Assert
        assert!(p.decorations.is_empty());
        assert_eq!(p.subject, "recieve");
    }

    #[test]
    fn every_label_has_its_default_action() {
        // Arrange
        let cases = [
            ("nitpick", Label::Nitpick, Action::Optional),
            ("suggestion", Label::Suggestion, Action::Judge),
            ("issue", Label::Issue, Action::MustFix),
            ("todo", Label::Todo, Action::MustFix),
            ("question", Label::Question, Action::NeedsAnswer),
            ("thought", Label::Thought, Action::Optional),
            ("chore", Label::Chore, Action::MustFix),
            ("typo", Label::Typo, Action::MustFix),
            ("polish", Label::Polish, Action::Optional),
        ];
        for (word, label, action) in cases {
            // Act
            let p = must(&format!("{word}: x"));
            // Assert
            assert_eq!(p.label, label, "{word}");
            assert_eq!(p.action(), action, "{word}");
        }
    }

    #[test]
    fn an_unknown_label_is_not_parsed() {
        // Arrange
        let body = "praise: nice work";
        // Act
        let p = parse_head(body);
        // Assert
        assert_eq!(p, None);
    }

    #[test]
    fn text_without_a_valid_head_is_not_parsed() {
        // Arrange
        let bodies = [
            "",
            "no colon here",
            "issue (blocking: x",
            "blocking (security): x",
        ];
        for body in bodies {
            // Act
            let p = parse_head(body);
            // Assert
            assert_eq!(p, None, "{body}");
        }
    }

    #[test]
    fn an_uppercase_label_and_decoration_are_lowercased() {
        // Arrange
        let body = "ISSUE (Non-Blocking): Spacing";
        // Act
        let p = must(body);
        // Assert
        assert_eq!(p.label, Label::Issue);
        assert_eq!(p.decorations, vec!["non-blocking"]);
        assert_eq!(p.subject, "Spacing");
        assert_eq!(p.action(), Action::Optional);
    }

    #[test]
    fn a_blocking_decoration_beats_an_optional_label_and_non_blocking() {
        // Arrange
        let a = "nitpick (blocking): x";
        let b = "suggestion (blocking, non-blocking): x";
        // Act
        let (pa, pb) = (must(a), must(b));
        // Assert
        assert_eq!(pa.action(), Action::MustFix);
        assert_eq!(pb.action(), Action::MustFix);
        assert!(pa.is_blocking());
    }

    #[test]
    fn the_old_bare_blocking_form_reads_as_a_blocking_issue() {
        // Arrange
        let body = "blocking: no retry limit";
        // Act
        let p = must(body);
        // Assert
        assert_eq!(p.label, Label::Issue);
        assert_eq!(p.decorations, vec!["blocking"]);
        assert_eq!(p.subject, "no retry limit");
        assert_eq!(p.action(), Action::MustFix);
    }

    #[test]
    fn only_the_first_line_is_read() {
        // Arrange
        let body = "thought: later\n\nissue: not this";
        // Act
        let p = must(body);
        // Assert
        assert_eq!(p.label, Label::Thought);
    }
}
