// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! PostToolUse backstop for what the PreToolUse rewrite did not see: another
//! hook rewriting the same command, an alias, a message built outside the
//! call. It looks at the result, not the command, and only after a call that
//! runs `git commit` or `git tag`, or `gh pr create`, `new` or `edit`: any
//! other Bash call costs no git or gh process.
//!
//! HEAD is amended with the sanitised message, and an AI author replaced by
//! the configured identity, only when this very call made the commit with a
//! message of its own and did not push, and no rebase, merge, cherry-pick or
//! bisect is under way. The amend keeps the index, the author and the date. A
//! commit made any other way, an older unpushed commit, a pushed commit and an
//! annotated tag are reported once per session, since fixing them rewrites
//! history the agent should choose to rewrite. After `gh pr create` or `edit`
//! the PR's title and body are read back and edited clean.

use super::engine::git_output;
use super::git::{git_use, GitUse};
use crate::common::attribution::{drop_lines, is_ai_identity, problems, prose_problems, Shape};
use crate::common::payload::Payload;
use crate::common::proc::{run_with_input, run_with_timeout};
use crate::common::shell::commands;
use crate::common::{atomic_append, emit_pre_context, session_dir};
use serde_json::Value;
use std::path::{Path, PathBuf};
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
    let used = git_use(command);
    let mut notes = Vec::new();
    if used.commit {
        commits(payload, dir, &used, &mut notes);
    }
    if used.tag {
        tags(payload, dir, &mut notes);
    }
    if let Some(reference) = pr_reference(payload, command) {
        pull_request(&reference, dir, &mut notes);
    }
    if !notes.is_empty() {
        emit_pre_context("PostToolUse", &notes.join(" "));
    }
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

fn commits(payload: &Payload, dir: &Path, used: &GitUse, notes: &mut Vec<String>) {
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
        notify_commit(
            payload,
            commit,
            "is an older unpushed commit that carries",
            "Fix it with a rebase on your own branch, never on a shared one.",
            notes,
        );
    }
    if head_is_unpushed {
        if let Some(commit) = dirty.iter().find(|c| c.sha == head) {
            match refusal(dir, used) {
                None => amend(dir, commit, notes),
                Some((what, advice)) => notify_commit(payload, commit, what, advice, notes),
            }
        }
    } else if let Some(commit) = git_output(dir, &["log", "-1", LOG_FORMAT, "HEAD"])
        .and_then(|log| parse_commits(&log).into_iter().next())
        .filter(Commit::is_dirty)
    {
        notify_commit(payload, &commit, "is already on a remote branch and carries", "Fix it with an amend and `git push --force-with-lease` on your own branch only, never on a shared branch.", notes);
    }
}

/// Why HEAD is not amended, as the start of a note about it and what to do,
/// or `None` when it is safe to.
fn refusal(dir: &Path, used: &GitUse) -> Option<(&'static str, &'static str)> {
    const AMEND: &str = "Amend it with a clean message.";
    if used.push {
        return Some(("was pushed by this call and carries", AMEND));
    }
    if !used.explicit_message {
        return Some((
            "was written in an editor or made some other way and carries",
            AMEND,
        ));
    }
    in_progress(dir).map(|_| {
        (
            "is part of a rebase, merge, cherry-pick or bisect that is under way and carries",
            "Amend it with a clean message once that is done.",
        )
    })
}

/// Which operation is under way in the repository, if one is: one that a
/// commit made now would belong to.
fn in_progress(dir: &Path) -> Option<&'static str> {
    const STATE: [(&str, &str); 6] = [
        ("rebase-merge", "rebase"),
        ("rebase-apply", "rebase"),
        ("MERGE_HEAD", "merge"),
        ("CHERRY_PICK_HEAD", "cherry-pick"),
        ("REVERT_HEAD", "revert"),
        ("BISECT_LOG", "bisect"),
    ];
    let git_dir = git_dir(dir)?;
    STATE
        .iter()
        .find(|(file, _)| git_dir.join(file).exists())
        .map(|(_, operation)| *operation)
}

fn git_dir(dir: &Path) -> Option<PathBuf> {
    let out = git_output(dir, &["rev-parse", "--git-dir"])?;
    Some(dir.join(out.trim()))
}

/// Adds a note unless this session was already told about `key`.
fn once(payload: &Payload, key: &str, note: String, notes: &mut Vec<String>) {
    let marker = session_dir(payload);
    let seen = Path::new(&marker).join(NOTIFIED_FILE);
    if !marker.is_empty() {
        if std::fs::read_to_string(&seen).is_ok_and(|text| text.lines().any(|l| l == key)) {
            return;
        }
        atomic_append(&seen.to_string_lossy(), key);
    }
    notes.push(note);
}

/// A note about `commit`, given once per session.
fn notify_commit(
    payload: &Payload,
    commit: &Commit,
    what: &str,
    advice: &str,
    notes: &mut Vec<String>,
) {
    let note = format!(
        "commit-message-sanitizer: commit {} {what} AI attribution: {}. {advice}",
        commit.short(),
        commit.describe()
    );
    once(payload, &commit.sha, note, notes);
}

/// Amends HEAD with the sanitised message alone, and with the configured
/// identity in place of an AI author. `--only` with no paths leaves the index
/// as it is, so staged changes stay staged and out of the commit, and the
/// author and date are kept. `-s` adds the sign-off only when it is missing,
/// and signing follows the repository's own config.
fn amend(dir: &Path, commit: &Commit, notes: &mut Vec<String>) {
    let message = format!(
        "{}\n",
        drop_lines(&commit.message, &commit.removed()).trim_end()
    );
    let mut args = vec![
        "commit",
        "--amend",
        "--only",
        "--allow-empty",
        "-s",
        "-F",
        "-",
    ];
    if commit.has_ai_identity() {
        args.push("--reset-author");
    }
    let lock = git_dir(dir).map(|d| d.join("index.lock"));
    let had_lock = lock.as_ref().is_some_and(|l| l.exists());
    let mut amend = Command::new("git");
    amend.arg("-C").arg(dir).args(&args);
    let done = run_with_input(&mut amend, message.as_bytes(), AMEND_TIMEOUT)
        .is_some_and(|out| out.status.success());
    if done {
        notes.push(format!(
            "commit-message-sanitizer amended the unpushed HEAD {} to remove AI attribution: {}.",
            commit.short(),
            commit.describe()
        ));
        return;
    }
    notes.push(format!(
        "commit-message-sanitizer could not amend HEAD {}, which carries AI attribution: {}. Amend it with a clean message.",
        commit.short(),
        commit.describe()
    ));
    if let Some(lock) = lock.filter(|l| !had_lock && l.exists()) {
        notes.push(format!(
            "The stopped git left {} behind. Check that no git process is running, then remove it.",
            lock.display()
        ));
    }
}

/// An annotated tag at HEAD whose message carries attribution is reported once
/// per session: recreating a tag is the agent's call. A lightweight tag has no
/// message of its own, so it is skipped.
fn tags(payload: &Payload, dir: &Path, notes: &mut Vec<String>) {
    let format = format!("--format=%(refname:short){FIELD}%(objecttype){FIELD}%(contents){RECORD}");
    let Some(out) = git_output(dir, &["tag", "--points-at", "HEAD", &format]) else {
        return;
    };
    for record in out.split(RECORD).filter(|r| !r.trim().is_empty()) {
        let mut fields = record.trim_start().splitn(3, FIELD);
        let (Some(name), Some("tag"), Some(message)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let removed = problems(message);
        if !removed.is_empty() {
            let lines: Vec<String> = removed
                .iter()
                .map(|(n, s)| format!("line {n} ({})", s.name()))
                .collect();
            let note = format!(
                "commit-message-sanitizer: the tag {name} carries AI attribution: {}. Recreate it with a clean message.",
                lines.join(", ")
            );
            once(payload, &format!("tag:{name}"), note, notes);
        }
    }
}

/// The PR a `gh pr create` or `edit` call produced, and the repository the
/// call named for it with `-R` or `--repo`, which a bare number needs.
struct PrRef {
    reference: String,
    repo: Option<String>,
}

/// The PR a `gh pr create` or `edit` call produced: the URL in its output,
/// else the number or URL given to `edit`.
fn pr_reference(payload: &Payload, command: &str) -> Option<PrRef> {
    let words = commands(command).into_iter().find_map(|cmd| {
        let at = cmd.words.iter().position(|w| w == "gh")?;
        let writes = cmd.words.get(at + 1).map(String::as_str) == Some("pr")
            && matches!(
                cmd.words.get(at + 2).map(String::as_str),
                Some("create" | "new" | "edit")
            );
        writes.then(|| cmd.words[at + 2..].to_vec())
    })?;
    let mut output = payload.field(".tool_response.stdout");
    if output.is_empty() {
        output = payload.field(".tool_response");
    }
    if let Some(url) = pull_url(&output) {
        return Some(PrRef {
            reference: url,
            repo: None,
        });
    }
    let reference = (words[0] == "edit")
        .then(|| words.get(1))
        .flatten()
        .filter(|w| !w.starts_with('-'))?;
    Some(PrRef {
        reference: reference.clone(),
        repo: repo_option(&words),
    })
}

/// The first pull request URL in `text`: it ends in `/pull/` and a number, so
/// the `/pull/new/<branch>` hint `git push` prints is not one.
fn pull_url(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|word| {
        let url = word.trim_end_matches(['.', ',', ')']);
        let number = url.strip_prefix("https://")?.split_once("/pull/")?.1;
        number
            .starts_with(|c: char| c.is_ascii_digit())
            .then(|| url.to_string())
    })
}

/// The repository given with `-R` or `--repo`.
fn repo_option(words: &[String]) -> Option<String> {
    words
        .iter()
        .enumerate()
        .find_map(|(at, word)| match word.as_str() {
            "-R" | "--repo" => words.get(at + 1).cloned(),
            w => w
                .strip_prefix("--repo=")
                .or_else(|| {
                    w.strip_prefix("-R")
                        .filter(|r| !r.is_empty() && !r.starts_with('-'))
                })
                .map(str::to_string),
        })
}

/// Reads the PR back and edits whatever still carries attribution.
fn pull_request(pr: &PrRef, dir: &Path, notes: &mut Vec<String>) {
    let reference = pr.reference.as_str();
    let repo_args: Vec<&str> = pr
        .repo
        .iter()
        .flat_map(|repo| ["--repo", repo.as_str()])
        .collect();
    let mut view = Command::new("gh");
    view.current_dir(dir)
        .args(["pr", "view", reference, "--json", "title,body"])
        .args(&repo_args);
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
    edit.current_dir(dir)
        .args(["pr", "edit", reference])
        .args(&repo_args);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pull_request_url_ends_in_a_number_and_the_push_hint_is_not_one() {
        let output = "remote: Create a pull request:\nremote:   https://github.com/o/r/pull/new/feat\nhttps://github.com/o/r/pull/12.\n";

        assert_eq!(
            pull_url(output).as_deref(),
            Some("https://github.com/o/r/pull/12")
        );
        assert_eq!(pull_url("https://github.com/o/r/pull/new/feat"), None);
        assert_eq!(pull_url("see https://github.com/o/r/pulls"), None);
    }

    #[test]
    fn the_repo_option_is_read_in_every_spelling() {
        let words = |text: &str| -> Vec<String> { text.split(' ').map(str::to_string).collect() };

        assert_eq!(
            repo_option(&words("edit 7 -R o/r --body x")).as_deref(),
            Some("o/r")
        );
        assert_eq!(
            repo_option(&words("edit 7 --repo o/r")).as_deref(),
            Some("o/r")
        );
        assert_eq!(
            repo_option(&words("edit 7 --repo=o/r")).as_deref(),
            Some("o/r")
        );
        assert_eq!(repo_option(&words("edit 7 -Ro/r")).as_deref(), Some("o/r"));
        assert_eq!(repo_option(&words("edit 7 --body x")), None);
    }
}
