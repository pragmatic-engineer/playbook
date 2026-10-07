// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! Walks a shell command, finds the `git` and `gh` calls that write a message
//! and rewrites the command so those messages carry no AI attribution.
//!
//! The command is rewritten as text: each message, heredoc body or option that
//! needs to change becomes one edit on the original characters, so everything
//! else in the command stays exactly as written. A message that lives in a
//! file is never rewritten on disk: the command reads the cleaned text from a
//! heredoc instead, and the file stays as it is.

use super::{gh, git, sources};
use crate::common::proc::run_with_timeout;
use crate::common::shell::{
    apply_edits, commands, program_index, program_name, quote, Command, Heredoc, Span,
};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::Duration;

/// How deep `bash -c "..."` strings and substitutions are followed.
const MAX_NESTING: usize = 3;
const SHELLS: [&str; 5] = ["bash", "sh", "zsh", "dash", "ksh"];
/// Programs that run another program given among their words.
const CARRIERS: [&str; 12] = [
    "rtk", "nohup", "setsid", "stdbuf", "ionice", "chrt", "xargs", "find", "parallel", "watch",
    "script", "unbuffer",
];
/// Carriers that run the program they are given more than once or with an
/// input of their own, so a heredoc given to them is not the program's.
const STDIN_CARRIERS: [&str; 4] = ["xargs", "parallel", "find", "watch"];
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// What the walk found, in words that never carry a message's text.
#[derive(Default)]
pub struct Findings {
    /// One entry per removal: where, the line and the shape of attribution.
    pub notes: Vec<String>,
    /// Message sources the walk could not read, so were not checked.
    pub unread: Vec<String>,
    /// A `-s` was added to a commit that had no sign-off.
    pub signed_off: bool,
}

/// Where text a handler writes into a script ends up.
#[derive(Clone, Copy)]
pub enum Inline<'a> {
    /// A script, or a quoted word, which keeps text exactly as written.
    Anywhere,
    /// The body of this heredoc, which its shell can expand or end early.
    InHeredoc(&'a Heredoc),
    /// The body of a heredoc inside another one: nothing is written there.
    Nowhere,
}

pub struct Rewritten {
    pub command: String,
    pub findings: Findings,
}

/// `command` with the AI attribution removed from every message it writes.
pub fn rewrite(command: &str, dir: &Path) -> Rewritten {
    let mut findings = Findings::default();
    let command = walk(command, dir, 0, Inline::Anywhere, &mut findings);
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
    /// How many scripts deep this one is, as `bash -c "..."` strings nest.
    pub depth: usize,
    pub inline: Inline<'a>,
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

    /// Whether a carrier such as `xargs` runs the program, so a heredoc given
    /// to the command would reach the carrier instead.
    pub fn under_stdin_carrier(&self) -> bool {
        let words = &self.cmd().words;
        let from = program_index(words).unwrap_or(self.at).min(self.at);
        words[from..self.at]
            .iter()
            .any(|w| STDIN_CARRIERS.contains(&program_name(w).as_str()))
    }
}

/// The edits the handlers make to the script, in characters of its text.
#[derive(Default)]
pub struct Plan {
    pub edits: Vec<(Range<usize>, String)>,
}

fn walk(script: &str, dir: &Path, depth: usize, inline: Inline, findings: &mut Findings) -> String {
    let cmds = commands(script);
    let chars: Vec<char> = script.chars().collect();
    let mut plan = Plan::default();
    // The directory at each parenthesis depth, and the group that set it: a
    // `cd` in `( ... )` ends there and does not reach a sibling group.
    let mut dirs = vec![(0, dir.to_path_buf())];
    for index in 0..cmds.len() {
        let (depth_in_script, group) = (cmds[index].depth, cmds[index].group);
        while dirs.len() <= depth_in_script {
            dirs.push((group, dirs.last().map(|d| d.1.clone()).unwrap_or_default()));
        }
        dirs.truncate(depth_in_script + 1);
        if dirs[depth_in_script].0 != group {
            let parent = dirs[depth_in_script - 1].1.clone();
            dirs[depth_in_script] = (group, parent);
        }
        follow_cd(&cmds[index].words, &mut dirs[depth_in_script].1);
        for (at, name) in locate(&cmds[index].words) {
            let call = Call {
                chars: &chars,
                cmds: &cmds,
                index,
                at,
                dir: &dirs[depth_in_script].1,
                depth,
                inline,
            };
            match name.as_str() {
                "git" => git::handle(&call, &mut plan, findings),
                "gh" => gh::handle(&call, &mut plan, findings),
                shell if SHELLS.contains(&shell) && depth < MAX_NESTING => {
                    shell_script(&call, &mut plan, findings);
                }
                "eval" => eval_scripts(&call, &mut plan, findings),
                _ if depth < MAX_NESTING && cmds[index].words[at].contains(char::is_whitespace) => {
                    // A whole script in the program position, as `env -S '...'` leaves it.
                    nested_word(&call, at, None, &mut plan, findings);
                }
                _ => {}
            }
        }
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

/// The programs a command runs and the index of each word: past wrappers such
/// as `env`, and through the carriers that run a `git`, `gh` or a shell given
/// among their own words.
pub fn locate(words: &[String]) -> Vec<(usize, String)> {
    let Some(at) = program_index(words) else {
        return Vec::new();
    };
    let name = program_name(&words[at]);
    if !CARRIERS.contains(&name.as_str()) {
        return vec![(at, name)];
    }
    let carried: Vec<(usize, String)> = (at + 1..words.len())
        .map(|i| (i, program_name(&words[i])))
        .filter(|(_, n)| {
            matches!(n.as_str(), "git" | "gh" | "eval") || SHELLS.contains(&n.as_str())
        })
        .collect();
    if carried.is_empty() {
        vec![(at, name)]
    } else {
        carried
    }
}

/// The script a shell runs: the word after `-c`, or a heredoc fed to it.
fn shell_script(call: &Call, plan: &mut Plan, findings: &mut Findings) {
    let words = &call.cmd().words;
    let flag = (call.at + 1..words.len()).find(|&i| {
        words[i].starts_with('-') && !words[i].starts_with("--") && words[i].contains('c')
    });
    if let Some(at) = flag.filter(|at| at + 1 < words.len()) {
        nested_word(call, at + 1, None, plan, findings);
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
        let inside = match call.inline {
            Inline::Anywhere => Inline::InHeredoc(heredoc),
            _ => Inline::Nowhere,
        };
        let rewritten = walk(&heredoc.text, call.dir, call.depth + 1, inside, findings);
        if rewritten != heredoc.text {
            plan.edits.push((heredoc.body.clone(), rewritten));
        }
    }
}

/// `eval` joins its words into one script, so a word that holds a whole
/// command line is rewritten as one.
fn eval_scripts(call: &Call, plan: &mut Plan, findings: &mut Findings) {
    for at in call.at + 1..call.cmd().words.len() {
        if call.cmd().words[at].contains(char::is_whitespace) {
            nested_word(call, at, None, plan, findings);
        }
    }
}

/// Rewrites the word at `at`, past its first `skip` characters, as a script.
/// A word the shell expands first is left alone, since requoting it would
/// change what it expands to.
pub fn nested_word(
    call: &Call,
    at: usize,
    skip: Option<usize>,
    plan: &mut Plan,
    findings: &mut Findings,
) {
    if call.span(at).dynamic || call.depth >= MAX_NESTING {
        return;
    }
    let Some(range) = sources::value_span(call, at, skip) else {
        return;
    };
    let text: String = call.cmd().words[at]
        .chars()
        .skip(skip.unwrap_or(0))
        .collect();
    let rewritten = walk(&text, call.dir, call.depth + 1, call.inline, findings);
    if rewritten != text {
        plan.edits.push((range, quote(&rewritten)));
    }
}

/// The result of `git <args>` in `dir`, trimmed of its final newline.
pub fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
    let mut command = Process::new("git");
    command.arg("-C").arg(dir).args(args);
    let out = run_with_timeout(&mut command, GIT_TIMEOUT)?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}
