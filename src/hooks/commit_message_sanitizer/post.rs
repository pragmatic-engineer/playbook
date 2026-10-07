// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! PostToolUse backstop for what the PreToolUse rewrite did not see: another
//! hook rewriting the same command, an alias, an editor, a message built
//! outside the call. After a Bash call that mentions git or gh it looks at
//! the result, not the command.
//!
//! A commit on no remote is cleaned: HEAD is amended with the sanitised
//! message and an AI author is replaced by the configured identity. Older
//! unpushed commits, a pushed commit and a tag are only reported, since fixing
//! them rewrites history the agent should choose to rewrite. After `gh pr
//! create` or `edit` the PR's title and body are read back and edited clean.

use super::engine::git_output;
use crate::common::attribution::{drop_lines, is_ai_identity, problems, prose_problems, Shape};
use crate::common::payload::Payload;
use crate::common::proc::{run_with_input, run_with_timeout};
use crate::common::shell::commands;
use crate::common::{atomic_append, emit_pre_context, session_dir};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// A signed amend can wait on an agent or a hardware key.
const AMEND_TIMEOUT: Duration = Duration::from_secs(30);
const GH_TIMEOUT: Duration = Duration::from_secs(20);
/// How many unpushed commits are looked at, newest first.
const MAX_COMMITS: &str = "20";
const NOTIFIED_FILE: &str = "commit-sanitizer-notified";
/// Separates the fields and the records of the `git log` output read here.
const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

pub fn run(payload: &Payload, command: &str, dir: &Path) {
    let mut notes = Vec::new();
    if mentions_git(command) {
        commits(payload, dir, &mut notes);
    }
    if let Some(reference) = pr_reference(payload, command) {
        pull_request(&reference, dir, &mut notes);
    }
    if !notes.is_empty() {
        emit_pre_context("PostToolUse", &notes.join(" "));
    }
}

fn mentions_git(command: &str) -> bool {
    command.to_lowercase().contains("git")
}

/// One commit as `git log` reports it.
struct Commit {
    sha: String,
    author: (String, String),
    committer: (String, String),
    message: String,
}

const LOG_FORMAT: &str = "--format=%H%x1f%an%x1f%ae%x1f%cn%x1f%ce%x1f%B%x1e";

fn parse_commits(log: &str) -> Vec<Commit> {
    log.split(RECORD)
        .filter(|record| !record.trim().is_empty())
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').splitn(6, FIELD);
            let mut next = || fields.next().map(str::to_string);
            Some(Commit {
                sha: next()?,
                author: (next()?, next()?),
                committer: (next()?, next()?),
                message: next()?,
            })
        })
        .collect()
}

impl Commit {
    fn short(&self) -> &str {
        &self.sha[..self.sha.len().min(7)]
    }

    fn removed(&self) -> Vec<(usize, Shape)> {
        problems(&self.message)
    }

    fn has_ai_identity(&self) -> bool {
        is_ai_identity(&self.author.0, &self.author.1)
            || is_ai_identity(&self.committer.0, &self.committer.1)
    }

    fn is_dirty(&self) -> bool {
        !self.removed().is_empty() || self.has_ai_identity()
    }

    /// What is wrong with the commit, by line and shape, never by value.
    fn describe(&self) -> String {
        let mut found: Vec<String> = self
            .removed()
            .iter()
            .map(|(n, shape)| format!("line {n} ({})", shape.name()))
            .collect();
        if self.has_ai_identity() {
            found.push("an AI as the author or committer".to_string());
        }
        found.join(", ")
    }
}

fn commits(payload: &Payload, dir: &Path, notes: &mut Vec<String>) {
    let Some(head) = git_output(dir, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string()) else {
        return;
    };
    let unpushed = git_output(
        dir,
        &[
            "log",
            "HEAD",
            "--not",
            "--remotes",
            "-n",
            MAX_COMMITS,
            LOG_FORMAT,
        ],
    )
    .map(|log| parse_commits(&log))
    .unwrap_or_default();
    let head_is_unpushed = unpushed.iter().any(|c| c.sha == head);
    let dirty: Vec<&Commit> = unpushed.iter().filter(|c| c.is_dirty()).collect();
    for commit in dirty.iter().filter(|c| c.sha != head) {
        notify_once(
            payload,
            commit,
            "is an older unpushed commit that carries",
            "Fix it with a rebase on your own branch, never on a shared one.",
            notes,
        );
    }
    if head_is_unpushed {
        if let Some(commit) = dirty.iter().find(|c| c.sha == head) {
            amend(dir, commit, notes);
        }
    } else if let Some(commit) = git_output(dir, &["log", "-1", LOG_FORMAT, "HEAD"])
        .and_then(|log| parse_commits(&log).into_iter().next())
        .filter(Commit::is_dirty)
    {
        notify_once(payload, &commit, "is already on a remote branch and carries", "Fix it with an amend and `git push --force-with-lease` on your own branch only, never on a shared branch.", notes);
    }
    tags(dir, notes);
}

/// Adds a note about `commit` unless this session was already told.
fn notify_once(
    payload: &Payload,
    commit: &Commit,
    what: &str,
    advice: &str,
    notes: &mut Vec<String>,
) {
    let marker = session_dir(payload);
    let seen = Path::new(&marker).join(NOTIFIED_FILE);
    let key = commit.sha.as_str();
    if !marker.is_empty() {
        if std::fs::read_to_string(&seen).is_ok_and(|text| text.lines().any(|l| l == key)) {
            return;
        }
        atomic_append(&seen.to_string_lossy(), key);
    }
    notes.push(format!(
        "commit-message-sanitizer: commit {} {what} AI attribution: {}. {advice}",
        commit.short(),
        commit.describe()
    ));
}

/// Amends HEAD with the sanitised message, keeping the signature and the
/// sign-off, and with the configured identity in place of an AI one.
fn amend(dir: &Path, commit: &Commit, notes: &mut Vec<String>) {
    let message = format!(
        "{}\n",
        drop_lines(&commit.message, &commit.removed()).trim_end()
    );
    let mut args = vec!["commit", "--amend", "--allow-empty", "-s", "-F", "-"];
    if commit.has_ai_identity() {
        args.push("--reset-author");
    }
    if signing_is_configured(dir) {
        args.push("-S");
    }
    let mut amend = Command::new("git");
    amend.arg("-C").arg(dir).args(&args);
    let done = run_with_input(&mut amend, message.as_bytes(), AMEND_TIMEOUT)
        .is_some_and(|out| out.status.success());
    notes.push(if done {
        format!(
            "commit-message-sanitizer amended the unpushed HEAD {} to remove AI attribution: {}.",
            commit.short(),
            commit.describe()
        )
    } else {
        format!(
            "commit-message-sanitizer could not amend HEAD {}, which carries AI attribution: {}. Amend it with a clean message.",
            commit.short(),
            commit.describe()
        )
    });
}

fn signing_is_configured(dir: &Path) -> bool {
    let signing = git_output(dir, &["config", "--type=bool", "--get", "commit.gpgsign"]);
    let signed_head = git_output(dir, &["log", "-1", "--format=%G?", "HEAD"]);
    signing.is_some_and(|v| v.trim() == "true")
        || signed_head.is_some_and(|v| !matches!(v.trim(), "" | "N"))
}

/// An annotated tag at HEAD whose message carries attribution is reported:
/// recreating a tag is the agent's call.
fn tags(dir: &Path, notes: &mut Vec<String>) {
    let format = format!("--format=%(refname:short){FIELD}%(contents){RECORD}");
    let Some(out) = git_output(dir, &["tag", "--points-at", "HEAD", &format]) else {
        return;
    };
    for record in out.split(RECORD).filter(|r| !r.trim().is_empty()) {
        let Some((name, message)) = record.trim_start().split_once(FIELD) else {
            continue;
        };
        let removed = problems(message);
        if !removed.is_empty() {
            let lines: Vec<String> = removed
                .iter()
                .map(|(n, s)| format!("line {n} ({})", s.name()))
                .collect();
            notes.push(format!(
                "commit-message-sanitizer: the tag {name} carries AI attribution: {}. Recreate it with a clean message.",
                lines.join(", ")
            ));
        }
    }
}

/// The PR a `gh pr create` or `edit` call produced: the URL in its output,
/// else the number or URL given to `edit`.
fn pr_reference(payload: &Payload, command: &str) -> Option<String> {
    let calls_pr_write = commands(command).iter().any(|cmd| {
        let words = &cmd.words;
        words.iter().position(|w| w == "gh").is_some_and(|at| {
            words.get(at + 1).map(String::as_str) == Some("pr")
                && matches!(
                    words.get(at + 2).map(String::as_str),
                    Some("create" | "new" | "edit")
                )
        })
    });
    if !calls_pr_write {
        return None;
    }
    let mut output = payload.field(".tool_response.stdout");
    if output.is_empty() {
        output = payload.field(".tool_response");
    }
    pull_url(&output).or_else(|| edited_pr(command))
}

fn pull_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|w| w.starts_with("https://") && w.contains("/pull/"))
        .map(|w| w.trim_end_matches(['.', ',', ')']).to_string())
}

fn edited_pr(command: &str) -> Option<String> {
    commands(command).iter().find_map(|cmd| {
        let at = cmd.words.iter().position(|w| w == "edit")?;
        cmd.words
            .get(at + 1)
            .filter(|w| !w.starts_with('-'))
            .cloned()
    })
}

/// Reads the PR back and edits whatever still carries attribution.
fn pull_request(reference: &str, dir: &Path, notes: &mut Vec<String>) {
    let mut view = Command::new("gh");
    view.current_dir(dir)
        .args(["pr", "view", reference, "--json", "title,body"]);
    let Some(out) = run_with_timeout(&mut view, GH_TIMEOUT).filter(|o| o.status.success()) else {
        return;
    };
    let Ok(pr) = serde_json::from_slice::<Value>(&out.stdout) else {
        return;
    };
    let field = |name: &str| pr[name].as_str().unwrap_or_default().to_string();
    let (title, body) = (field("title"), field("body"));
    let title_removed = prose_problems(&title);
    let body_removed = prose_problems(&body);
    if title_removed.is_empty() && body_removed.is_empty() {
        return;
    }
    let mut edit = Command::new("gh");
    edit.current_dir(dir).args(["pr", "edit", reference]);
    if !title_removed.is_empty() {
        edit.arg("--title")
            .arg(drop_lines(&title, &title_removed).trim());
    }
    let clean_body = drop_lines(&body, &body_removed);
    if !body_removed.is_empty() {
        edit.args(["--body-file", "-"]);
    }
    let done = run_with_input(&mut edit, clean_body.as_bytes(), GH_TIMEOUT)
        .is_some_and(|out| out.status.success());
    let places = |label: &str, removed: &[(usize, Shape)]| -> Option<String> {
        (!removed.is_empty()).then(|| {
            let lines: Vec<String> = removed
                .iter()
                .map(|(n, s)| format!("line {n} ({})", s.name()))
                .collect();
            format!("the PR {label} {}", lines.join(", "))
        })
    };
    let found: Vec<String> = [
        places("title", &title_removed),
        places("body", &body_removed),
    ]
    .into_iter()
    .flatten()
    .collect();
    notes.push(if done {
        format!("commit-message-sanitizer edited the PR to remove AI attribution: {}.", found.join("; "))
    } else {
        format!("commit-message-sanitizer could not edit the PR, which carries AI attribution: {}. Edit it with `gh pr edit`.", found.join("; "))
    });
}
