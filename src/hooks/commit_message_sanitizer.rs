// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! PreToolUse hook that removes AI attribution from the messages a Bash call
//! writes with `git commit`, `git tag` or `git merge`. It never blocks. When a
//! message changes, the hook hands Claude Code the same call with the message
//! cleaned and no permission decision, so the normal permission flow still
//! applies. A message kept in a file is read from a heredoc instead, and the
//! file itself is never rewritten.
//!
//! Every `git commit` that would carry no `Signed-off-by` line gets `-s`, so
//! the DCO check passes. The hook stands down when the command has `-s`,
//! `--signoff`, `--no-signoff` or `--dry-run`, when the message already has a
//! sign-off, when the `commit.signOff` setting is false, or when a
//! `prepare-commit-msg` or `commit-msg` hook of the repository adds the line
//! itself, by `--signoff` or by `git interpret-trailers --trailer` for it. A
//! hook that only checks for the line does not count.
//!
//! Best effort against an agent drifting, not a security boundary: it does
//! not expand variables in a message, an alias or a script that builds one.

mod engine;
mod gh;
mod git;
mod sources;

use crate::common::payload::Payload;
use crate::common::{emit_pre_context, emit_pre_updated_input};
use serde_json::Value;
use std::path::PathBuf;

pub fn run(payload: &Payload) {
    let command = payload.field(".tool_input.command");
    if !may_run_git_or_gh(&command) {
        return;
    }
    let rewritten = engine::rewrite(&command, &start_dir(payload));
    let note = context(&rewritten.findings);
    if rewritten.command != command {
        if let Some(mut input) = payload.value(".tool_input").cloned() {
            input["command"] = Value::String(rewritten.command);
            emit_pre_updated_input(&input, note.as_deref());
            return;
        }
    }
    if let Some(note) = note {
        emit_pre_context("PreToolUse", &note);
    }
}

/// A cheap check that rules out nearly every Bash call before any parsing,
/// file or git work. Quotes and backslashes are dropped first, so `g''it` and
/// `g\it` still pass.
fn may_run_git_or_gh(command: &str) -> bool {
    let bare: String = command
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\'))
        .collect::<String>()
        .to_lowercase();
    bare.contains("git") || bare.contains("gh")
}

fn start_dir(payload: &Payload) -> PathBuf {
    let cwd = payload.field(".cwd");
    if cwd.is_empty() {
        std::env::current_dir().unwrap_or_default()
    } else {
        PathBuf::from(cwd)
    }
}

/// One line for the agent: what was removed, by line and shape, and what
/// could not be read. Never the text of a removed line.
fn context(findings: &engine::Findings) -> Option<String> {
    let mut parts = Vec::new();
    if !findings.notes.is_empty() {
        parts.push(format!(
            "commit-message-sanitizer removed AI attribution before this ran: {}.",
            findings.notes.join("; ")
        ));
    }
    if !findings.unread.is_empty() {
        parts.push(format!(
            "commit-message-sanitizer could not read {}, so it was not checked.",
            findings.unread.join("; ")
        ));
    }
    if findings.signed_off {
        parts.push(
            "commit-message-sanitizer added -s so the commit carries a Signed-off-by line."
                .to_string(),
        );
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}
