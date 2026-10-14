// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! Removes AI attribution from the messages a Bash call writes: a commit, tag,
//! merge or PR title and body. Removing attribution never blocks.
//!
//! On PreToolUse, when a message changes, the hook hands Claude Code the same
//! call with the message cleaned and no permission decision, so the normal
//! permission flow still applies. A message kept in a file is read from a
//! heredoc instead, and the file itself is never rewritten. On PostToolUse it
//! is a backstop that cleans what the rewrite could not see, such as a call
//! another hook rewrote too (see `post`).
//!
//! Every `git commit` that would carry no `Signed-off-by` line gets `-s`, so
//! the DCO check passes. The hook stands down when the command has `-s`,
//! `--signoff`, `--no-signoff` or `--dry-run`, when the message already has a
//! sign-off, when the `commit.signOff` setting is false, or when a
//! `prepare-commit-msg` or `commit-msg` hook of the repository adds the line
//! itself, by `--signoff` or by `git interpret-trailers --trailer` for it. A
//! hook that only checks for the line does not count.
//!
//! The same pass lints the prose of those messages, and of `gh pr comment`,
//! `gh pr review` and `gh issue` text, against the writing-style phrase and
//! structure table (`common::prose_lint`). In the default `warn` mode the
//! findings go to the model as context. With `hooks.proseLint.mode` set to
//! `block` a hard finding denies the call, and `off` turns the lint off.
//!
//! Best effort against an agent drifting, not a security boundary: it does
//! not expand variables in a message, an alias or a script that builds one.

mod amend;
mod engine;
mod gh;
mod git;
mod post;
mod sources;
mod state;

use crate::common::payload::Payload;
use crate::common::{emit_pre_context, emit_pre_deny, emit_pre_updated_input};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn run(payload: &Payload) {
    let command = payload.field(".tool_input.command");
    if !may_run_git_or_gh(&command) {
        return;
    }
    let dir = start_dir(payload);
    if payload.field(".hook_event_name") == "PostToolUse" {
        post::run(payload, &command, &dir);
    } else {
        pre(payload, &command, &dir);
    }
}

fn pre(payload: &Payload, command: &str, dir: &Path) {
    for repo in &git::git_use(command, dir).commit_dirs {
        state::record_head(payload, repo);
    }
    let rewritten = engine::rewrite(command, dir);
    let mode = lint_mode();
    if mode == "block" && rewritten.findings.lint_hard {
        emit_pre_deny(&lint_denial(&rewritten.findings.lint));
        return;
    }
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

/// The `hooks.proseLint.mode` setting: `warn` (the default), `block` or `off`.
pub(crate) fn lint_mode() -> String {
    static MODE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    MODE.get_or_init(read_lint_mode).clone()
}

fn read_lint_mode() -> String {
    let home = crate::common::home_dir();
    let slug = crate::common::repo_slug();
    let slug = (!slug.is_empty()).then_some(slug);
    crate::config::resolve_valid("hooks.proseLint.mode", &home, slug.as_deref())
        .ok()
        .and_then(|(v, _, _)| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "warn".to_string())
}

/// The most findings put in front of the model, so a long message cannot
/// flood its context.
const MAX_LINT_SHOWN: usize = 20;

fn lint_list(lint: &[String]) -> String {
    let mut text = lint
        .iter()
        .take(MAX_LINT_SHOWN)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    if lint.len() > MAX_LINT_SHOWN {
        text.push_str(&format!(
            " And {} more: run `playbook prose check` on a file for the full list.",
            lint.len() - MAX_LINT_SHOWN
        ));
    }
    text
}

fn lint_denial(lint: &[String]) -> String {
    format!(
        "prose-lint blocked this call, since hooks.proseLint.mode is block. Fix the text and run it again: {}",
        lint_list(lint)
    )
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
    if !findings.lint.is_empty() {
        parts.push(format!(
            "prose-lint found writing-style problems, so fix them in the text you write next time or amend it: {}",
            lint_list(&findings.lint)
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}
