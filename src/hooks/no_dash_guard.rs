// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Ports hooks/no-dash-guard.sh: a PreToolUse(Bash) guard denying a posting
//! command that carries an em or en dash, in the command text or in a body
//! file it references.
//!
//! Scoped to posting commands because this is the last chokepoint before prose
//! reaches GitHub or git history, where a dash stops being cheap to fix.
//!
//! The shell version shelled out to python only to compare code points, since
//! bash and BSD grep cannot portably match multibyte classes, and it failed
//! open when python3 was absent. Rust decodes UTF-8 natively, so both the
//! dependency and that fail-open branch are gone.

use crate::common::emit_pre_deny;
use crate::common::payload::Payload;
use std::path::{Path, PathBuf};

/// U+2012 figure, U+2013 en, U+2014 em, U+2015 horizontal bar, matching the
/// range the shell version used.
const DASHES: [char; 4] = ['\u{2012}', '\u{2013}', '\u{2014}', '\u{2015}'];

/// Flags whose value names a file holding the prose to post.
const FILE_FLAGS: [&str; 4] = ["--body-file", "--input", "--file", "-F"];

/// `gh api` only posts prose when it targets one of these endpoints.
const API_ENDPOINTS: [&str; 4] = ["reviews", "comments", "issues", "pulls"];

pub fn run(payload: &Payload) {
    if std::env::var("NO_DASH_GUARD").as_deref() == Ok("0") {
        return;
    }
    let cmd = payload.field(".tool_input.command");
    if cmd.is_empty() || !is_posting(&cmd) {
        return;
    }

    if cmd.contains(DASHES) {
        deny("the command text");
        return;
    }
    for path in referenced_files(&cmd) {
        if file_has_dash(&path) {
            deny(&format!("the file {path}"));
            return;
        }
    }
}

fn deny(where_found: &str) {
    emit_pre_deny(&format!(
        "Blocked: this post contains an em or en dash in {where_found}. \
         Em and en dashes (and their lookalikes) are banned in anything posted: \
         PR titles and bodies, review and issue comments, and commit and tag \
         messages. Rewrite using commas, colons, parentheses, or separate \
         sentences, then run the command again."
    ));
}

/// True for commands that publish prose: `gh pr|issue|release` writes, `gh api`
/// against a review or comment endpoint, and `git commit|tag`.
///
/// A token only counts at the start of a command, so a `--title "gh pr create"`
/// carrying those words as prose does not arm the guard.
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

/// Byte offsets where a command can begin: the start of the string, or just
/// past a separator or whitespace.
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

/// An unreadable path passes rather than blocks: the guard catches prose it can
/// see, and a path it cannot open is the shell's problem to report.
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
