// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! One place that compiles regexes written as literals in the source.

use regex::Regex;

/// Compile a pattern that is a literal in the source. A bad pattern is a bug
/// in the program, found by the first test that reaches it, never by user input.
#[allow(
    clippy::expect_used,
    reason = "the pattern is a source literal, so a failure is a programming error"
)]
pub fn static_regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("source literal is a valid regex")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_literal_compiles() {
        assert!(static_regex(r"^a+$").is_match("aaa"));
    }

    #[test]
    #[should_panic(expected = "valid regex")]
    fn an_invalid_literal_panics_with_a_clear_message() {
        let _ = static_regex("(");
    }
}
