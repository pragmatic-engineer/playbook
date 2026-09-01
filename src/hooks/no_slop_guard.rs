// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! PreToolUse guard denying AI-slop output: an em or en dash in a posting
//! command (`Bash`), or new code with a slop comment (`Edit`/`Write`).

use crate::common::emit_pre_deny;
use crate::common::payload::Payload;
use std::path::{Path, PathBuf};

pub fn run(payload: &Payload) {
    if std::env::var("NO_SLOP_GUARD").as_deref() == Ok("0") {
        return;
    }
    let cmd = payload.field(".tool_input.command");
    if !cmd.is_empty() {
        check_dash(&cmd);
        return;
    }
    check_comment_slop(payload);
}

// --- dash check: PreToolUse(Bash) ------------------------------------------

/// U+2012 figure, U+2013 en, U+2014 em, U+2015 horizontal bar.
const DASHES: [char; 4] = ['\u{2012}', '\u{2013}', '\u{2014}', '\u{2015}'];

/// Flags whose value names a file holding the prose to post.
const FILE_FLAGS: [&str; 4] = ["--body-file", "--input", "--file", "-F"];

/// `gh api` only posts prose when it targets one of these endpoints.
const API_ENDPOINTS: [&str; 4] = ["reviews", "comments", "issues", "pulls"];

fn check_dash(cmd: &str) {
    if !is_posting(cmd) {
        return;
    }
    if cmd.contains(DASHES) {
        deny_dash("the command text");
        return;
    }
    for path in referenced_files(cmd) {
        if file_has_dash(&path) {
            deny_dash(&format!("the file {path}"));
            return;
        }
    }
}

fn deny_dash(where_found: &str) {
    emit_pre_deny(&format!(
        "Blocked: this post contains an em or en dash in {where_found}. \
         Em and en dashes (and their lookalikes) are banned in anything posted: \
         PR titles and bodies, review and issue comments, and commit and tag \
         messages. Rewrite using commas, colons, parentheses, or separate \
         sentences, then run the command again."
    ));
}

/// True for a command that publishes prose: `gh pr|issue|release` writes,
/// `gh api` against a review/comment endpoint, or `git commit|tag`.
fn is_posting(cmd: &str) -> bool {
    command_starts(cmd).into_iter().any(|start| {
        let rest = &cmd[start..];
        let mut words = rest.split_whitespace();
        match (words.next(), words.next(), words.next()) {
            (Some("gh"), Some("pr" | "issue" | "release"), Some(verb)) => {
                matches!(verb, "create" | "edit" | "comment" | "review")
            }
            (Some("gh"), Some("api"), _) => API_ENDPOINTS.iter().any(|e| rest.contains(e)),
            (Some("git"), Some("commit" | "tag"), _) => true,
            _ => false,
        }
    })
}

/// Byte offsets where a command can begin: the start, or past a separator.
fn command_starts(cmd: &str) -> Vec<usize> {
    let mut out = vec![0usize];
    for (i, c) in cmd.char_indices() {
        if matches!(c, ';' | '&' | '|' | '(') || c.is_whitespace() {
            out.push(i + c.len_utf8());
        }
    }
    out
}

/// Paths passed to a body-file flag, in `--flag value` or `--flag=value` form.
fn referenced_files(cmd: &str) -> Vec<String> {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        match tok.split_once('=') {
            Some((flag, value)) if FILE_FLAGS.contains(&flag) && !value.is_empty() => {
                out.push(unquote(value));
            }
            _ if FILE_FLAGS.contains(&tok) => {
                if let Some(value) = tokens.get(i + 1) {
                    out.push(unquote(value));
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

fn unquote(s: &str) -> String {
    s.trim_matches(['"', '\'']).to_string()
}

/// An unreadable path passes rather than blocks: a missing file is not this
/// guard's problem to report.
fn file_has_dash(path: &str) -> bool {
    match std::fs::read(expand_tilde(path)) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).contains(DASHES),
        Err(_) => false,
    }
}

fn expand_tilde(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => Path::new(&home).join(rest),
        _ => PathBuf::from(path),
    }
}

// --- comment-slop check: PreToolUse(Edit|Write) -----------------------------

/// Extensions whose line-comment marker this guard knows.
const MARKERS: [(&str, &str); 4] = [(".rs", "//"), (".sh", "#"), (".zsh", "#"), (".py", "#")];

/// Phrases a comment should never carry; see `ORCHESTRATION_TERMS.md` intent
/// below: a future reader never saw the dispatch that produced the change.
const ORCHESTRATION_TERMS: [&str; 3] = ["Work Unit", "the brief", "done-when"];

fn check_comment_slop(payload: &Payload) {
    let file_path = payload.field(".tool_input.file_path");
    let Some(marker) = marker_for(&file_path) else {
        return;
    };
    let text = new_text(payload);
    if text.is_empty() {
        return;
    }
    if let Some(reason) = comment_violation(&text, marker) {
        emit_pre_deny(&format!(
            "Blocked: {reason} in {file_path}. Comments are one line by \
             default, a second only for a genuinely non-obvious mechanism, \
             never a third. Never name a plan, brief, dispatch id, or \
             completion criterion in a comment; a future reader never saw \
             the dispatch. Rewrite the comment, then run the edit again."
        ));
    }
}

/// Edit sends replacement text in `new_string`; Write sends the whole file
/// in `content`. Exactly one is ever non-empty, so trying both covers either.
fn new_text(payload: &Payload) -> String {
    let new_string = payload.field(".tool_input.new_string");
    if !new_string.is_empty() {
        return new_string;
    }
    payload.field(".tool_input.content")
}

fn marker_for(file_path: &str) -> Option<&'static str> {
    MARKERS
        .iter()
        .find(|(suffix, _)| file_path.ends_with(suffix))
        .map(|(_, marker)| *marker)
}

/// First violation found: a 3+ line comment run, or a comment line naming
/// an orchestration term. A non-comment line resets the run.
fn comment_violation(text: &str, marker: &str) -> Option<String> {
    let mut run = 0u32;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with(marker) {
            run = 0;
            continue;
        }
        if let Some(term) = ORCHESTRATION_TERMS.iter().find(|t| trimmed.contains(**t)) {
            return Some(format!(
                "a comment names an orchestration artifact (\"{term}\")"
            ));
        }
        if has_ticket_id(trimmed) {
            return Some("a comment names a dispatch id".to_string());
        }
        run += 1;
        if run >= 3 {
            return Some("a comment block runs 3 or more lines".to_string());
        }
    }
    None
}

/// True for a comment line carrying a two-letter prefix, a hyphen, then a
/// digit: this repo's historical dispatch-id shape, any case.
fn has_ticket_id(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower
        .match_indices("wu-")
        .any(|(i, _)| lower.as_bytes().get(i + 3).is_some_and(u8::is_ascii_digit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_line_rust_comment_run_is_a_violation() {
        // Arrange
        let text = "//! line one\n//! line two\n//! line three\n";

        // Act
        let got = comment_violation(text, "//");

        // Assert
        assert!(got.is_some());
    }

    #[test]
    fn two_line_rust_comment_run_is_not_a_violation() {
        // Arrange
        let text = "// line one\n// line two\nfn f() {}\n";

        // Act
        let got = comment_violation(text, "//");

        // Assert
        assert!(got.is_none());
    }

    #[test]
    fn orchestration_phrase_in_a_single_comment_line_is_a_violation() {
        // Arrange, Act, Assert
        for phrase in ORCHESTRATION_TERMS {
            let text = format!("// see {phrase} for context\n");
            assert!(comment_violation(&text, "//").is_some(), "{phrase}");
        }
    }

    #[test]
    fn ticket_id_in_a_single_comment_line_is_a_violation() {
        // Arrange
        let prefix = "W";
        let text = format!("// ported alongside {prefix}U-13's other guard bodies\n");

        // Act
        let got = comment_violation(&text, "//");

        // Assert
        assert!(got.is_some());
    }

    #[test]
    fn non_comment_line_resets_the_run() {
        // Arrange
        let text = "// line one\n// line two\nlet x = 1;\n// line three\n// line four\n";

        // Act
        let got = comment_violation(text, "//");

        // Assert
        assert!(got.is_none());
    }

    #[test]
    fn marker_for_unknown_extension_is_none() {
        // Arrange, Act, Assert
        assert_eq!(marker_for("README.md"), None);
        assert_eq!(marker_for("settings.json"), None);
    }

    #[test]
    fn marker_for_known_extensions() {
        // Arrange, Act, Assert
        assert_eq!(marker_for("src/lib.rs"), Some("//"));
        assert_eq!(marker_for("shell/setup.sh"), Some("#"));
        assert_eq!(marker_for("shell/gen.py"), Some("#"));
    }
}
