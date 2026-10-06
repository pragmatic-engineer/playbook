// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Walks a shell command, finds the `git` and `gh` calls that write a message
//! and rewrites the command so those messages carry no AI attribution.
//!
//! The command is rewritten as text: each message, heredoc body or option that
//! needs to change becomes one edit on the original characters, so everything
//! else in the command stays exactly as written. A message that lives in a
//! file is rewritten on disk instead.

use super::{git, sources};
use crate::common::shell::{
    apply_edits, commands, program_index, program_name, quote, Command, Span,
};
use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// How deep `bash -c "..."` strings and substitutions are followed.
const MAX_NESTING: usize = 3;
const SHELLS: [&str; 5] = ["bash", "sh", "zsh", "dash", "ksh"];
/// Programs that run another program given among their words.
const CARRIERS: [&str; 12] = [
    "rtk", "nohup", "setsid", "stdbuf", "ionice", "chrt", "xargs", "find", "parallel", "watch",
    "script", "unbuffer",
];

/// What the walk found, in words that never carry a message's text.
#[derive(Default)]
pub struct Findings {
    /// One entry per removal: where, the line and the shape of attribution.
    pub notes: Vec<String>,
    /// Message sources the walk could not read, so were not checked.
    pub unread: Vec<String>,
}

pub struct Rewritten {
    pub command: String,
    pub findings: Findings,
}

/// `command` with the AI attribution removed from every message it writes.
pub fn rewrite(command: &str, dir: &Path) -> Rewritten {
    let mut findings = Findings::default();
    let command = walk(command, dir, 0, &mut findings);
    Rewritten { command, findings }
}

/// One simple command in its script, with what a handler needs around it.
#[derive(Clone, Copy)]
pub struct Call<'a> {
    pub chars: &'a [char],
    pub cmds: &'a [Command],
    pub index: usize,
    /// Index of the program word in the command.
    pub at: usize,
    pub dir: &'a Path,
}

impl Call<'_> {
    pub fn cmd(&self) -> &Command {
        &self.cmds[self.index]
    }

    pub fn span(&self, word: usize) -> &Span {
        &self.cmd().spans[word]
    }

    /// The source text of the characters `range` covers.
    pub fn raw(&self, range: Range<usize>) -> String {
        self.chars[range].iter().collect()
    }
}

/// The edits the handlers make to the script, in characters of its text.
#[derive(Default)]
pub struct Plan {
    pub edits: Vec<(Range<usize>, String)>,
}

fn walk(script: &str, dir: &Path, depth: usize, findings: &mut Findings) -> String {
    let cmds = commands(script);
    let chars: Vec<char> = script.chars().collect();
    let mut plan = Plan::default();
    let mut dir = dir.to_path_buf();
    for index in 0..cmds.len() {
        follow_cd(&cmds[index].words, &mut dir);
        let Some((at, name)) = locate(&cmds[index].words) else {
            continue;
        };
        let call = Call {
            chars: &chars,
            cmds: &cmds,
            index,
            at,
            dir: &dir,
        };
        let mut used = HashSet::new();
        match name.as_str() {
            "git" => git::handle(&call, &mut plan, &mut used, findings),
            shell if SHELLS.contains(&shell) && depth < MAX_NESTING => {
                shell_script(&call, depth, &mut plan, &mut used, findings);
            }
            _ if depth < MAX_NESTING && cmds[index].words[at].contains(char::is_whitespace) => {
                // A whole script in the program position, as `env -S '...'` leaves it.
                nested_word(&call, at, depth, &mut plan, findings);
            }
            _ => {}
        }
        scan_words(&call, &used, depth, &mut plan, findings);
    }
    apply_edits(script, &mut plan.edits)
}

fn follow_cd(words: &[String], dir: &mut PathBuf) {
    if words.first().map(String::as_str) != Some("cd") {
        return;
    }
    if let Some(target) = words.get(1).filter(|t| t.as_str() != "-") {
        *dir = sources::resolve(target, dir);
    }
}

/// The program a command runs and the index of its word: past wrappers such as
/// `env`, and through the carriers that run a `git` or `gh` given among their
/// own words.
fn locate(words: &[String]) -> Option<(usize, String)> {
    let at = program_index(words)?;
    let name = program_name(&words[at]);
    if !CARRIERS.contains(&name.as_str()) {
        return Some((at, name));
    }
    let found = words[at + 1..]
        .iter()
        .position(|w| matches!(program_name(w).as_str(), "git" | "gh"));
    found.map_or(Some((at, name)), |i| {
        Some((at + 1 + i, program_name(&words[at + 1 + i])))
    })
}

/// The script a shell runs: the word after `-c`, or a heredoc fed to it.
fn shell_script(
    call: &Call,
    depth: usize,
    plan: &mut Plan,
    used: &mut HashSet<usize>,
    findings: &mut Findings,
) {
    let words = &call.cmd().words;
    let flag = (call.at + 1..words.len()).find(|&i| {
        words[i].starts_with('-') && !words[i].starts_with("--") && words[i].contains('c')
    });
    if let Some(at) = flag.filter(|at| at + 1 < words.len()) {
        used.insert(at + 1);
        nested_word(call, at + 1, depth, plan, findings);
        return;
    }
    let producer = call
        .cmd()
        .piped
        .then(|| call.index.checked_sub(1).map(|i| &call.cmds[i]))
        .flatten();
    for heredoc in call
        .cmd()
        .heredocs
        .iter()
        .chain(producer.into_iter().flat_map(|c| c.heredocs.iter()))
    {
        let rewritten = walk(&heredoc.text, call.dir, depth + 1, findings);
        if rewritten != heredoc.text {
            plan.edits.push((heredoc.body.clone(), rewritten));
        }
    }
}

/// Words that hold a script of their own, as in `git rebase -x '...'` or
/// `env -S '...'`, are rewritten as scripts.
fn scan_words(
    call: &Call,
    used: &HashSet<usize>,
    depth: usize,
    plan: &mut Plan,
    findings: &mut Findings,
) {
    if depth >= MAX_NESTING {
        return;
    }
    for (at, word) in call.cmd().words.iter().enumerate().skip(call.at + 1) {
        let mentions_a_call = {
            let lower = word.to_lowercase();
            lower.contains("git") || lower.contains("gh ")
        };
        if !used.contains(&at) && mentions_a_call && word.contains(char::is_whitespace) {
            nested_word(call, at, depth, plan, findings);
        }
    }
}

/// Rewrites the word at `at` as a script. A word the shell expands first is
/// left alone, since requoting it would change what it expands to.
fn nested_word(call: &Call, at: usize, depth: usize, plan: &mut Plan, findings: &mut Findings) {
    let span = call.span(at);
    if span.dynamic {
        return;
    }
    let text = &call.cmd().words[at];
    let rewritten = walk(text, call.dir, depth + 1, findings);
    if &rewritten != text {
        plan.edits.push((span.start..span.end, quote(&rewritten)));
    }
}
