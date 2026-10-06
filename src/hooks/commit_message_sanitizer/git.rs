// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The `git` calls that write a message: `commit`, `tag` and `merge`. Reads
//! the message from `-m`, from `-F` and from a heredoc fed to `-F -`.

use super::engine::{Call, Findings, Plan};
use super::sources::{apply, from_path, from_word, resolve, Source, Target};
use crate::common::cli_opts::{Opt, Spec};
use std::collections::HashSet;
use std::path::PathBuf;

// The specs list the options whose value can be a message or hides one, and
// the flags that change where a message comes from. Other options need no
// entry: git rejects an abbreviation that is ambiguous among all of its
// options, so a prefix that matches one entry here is the option git means.
const COMMIT: Spec = Spec {
    short_values: "mFCct",
    short_attached: "Su",
    long_values: &[
        "message",
        "file",
        "reuse-message",
        "reedit-message",
        "author",
        "date",
        "template",
        "cleanup",
        "fixup",
        "squash",
        "trailer",
    ],
    long_flags: &["amend", "no-edit", "edit", "reset-author", "signoff"],
    other_long: &[],
    abbreviate: true,
};
const MERGE: Spec = Spec {
    short_values: "mFsX",
    short_attached: "S",
    long_values: &["message", "file", "strategy", "strategy-option", "cleanup"],
    long_flags: &["no-edit", "edit", "squash", "no-commit", "signoff"],
    other_long: &[],
    abbreviate: true,
};
const TAG: Spec = Spec {
    short_values: "mFuf",
    short_attached: "",
    long_values: &["message", "file", "local-user", "cleanup", "trailer"],
    long_flags: &["annotate", "sign", "force", "edit", "no-edit"],
    other_long: &[],
    abbreviate: true,
};
/// Global options that take a value in the next word.
const GLOBAL_WITH_VALUE: [&str; 6] = [
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--super-prefix",
    "--attr-source",
    "--config-env",
];

/// What every subcommand handler needs.
struct Run<'a> {
    call: Call<'a>,
    /// Index of the first word after the subcommand.
    rest: usize,
}

pub fn handle(call: &Call, plan: &mut Plan, used: &mut HashSet<usize>, findings: &mut Findings) {
    let Some((sub_at, dir)) = skip_globals(call) else {
        return;
    };
    let call = Call { dir: &dir, ..*call };
    let run = Run {
        call,
        rest: sub_at + 1,
    };
    match call.cmd().words[sub_at].as_str() {
        "commit" => commit(&run, plan, used, findings),
        "merge" => message_command(&run, &MERGE, "the merge message", plan, used, findings),
        "tag" => tag(&run, plan, used, findings),
        _ => {}
    }
}

/// The index of the subcommand word and the directory git runs in, past the
/// global options.
fn skip_globals(call: &Call) -> Option<(usize, PathBuf)> {
    let words = &call.cmd().words;
    let mut dir = call.dir.to_path_buf();
    let mut at = call.at + 1;
    while let Some(word) = words.get(at) {
        match word.as_str() {
            "-C" => {
                if let Some(target) = words.get(at + 1) {
                    dir = resolve(target, &dir);
                }
                at += 2;
            }
            "-c" => at += 2,
            w if GLOBAL_WITH_VALUE.contains(&w) => at += 2,
            w if w.starts_with('-') => at += 1,
            _ => return Some((at, dir)),
        }
    }
    None
}

fn commit(run: &Run, plan: &mut Plan, used: &mut HashSet<usize>, findings: &mut Findings) {
    let opts = COMMIT.scan(&run.call.cmd().words[run.rest..]);
    let (sources, _) = message_sources(run, &opts, "the commit message", used, findings);
    apply(sources, plan, findings);
}

fn tag(run: &Run, plan: &mut Plan, used: &mut HashSet<usize>, findings: &mut Findings) {
    let opts = TAG.scan(&run.call.cmd().words[run.rest..]);
    let (sources, _) = message_sources(run, &opts, "the tag message", used, findings);
    apply(sources, plan, findings);
}

fn message_command(
    run: &Run,
    spec: &Spec,
    label: &str,
    plan: &mut Plan,
    used: &mut HashSet<usize>,
    findings: &mut Findings,
) {
    let opts = spec.scan(&run.call.cmd().words[run.rest..]);
    let (sources, _) = message_sources(run, &opts, label, used, findings);
    apply(sources, plan, findings);
}

/// The text of every `-m` and `-F` option, and whether there was one.
fn message_sources(
    run: &Run,
    opts: &[Opt],
    label: &str,
    used: &mut HashSet<usize>,
    findings: &mut Findings,
) -> (Vec<Source>, bool) {
    let call = &run.call;
    let mut sources = Vec::new();
    let mut written = false;
    for opt in opts {
        let value_at = opt.value_word + run.rest;
        let Some(value) = opt
            .value
            .as_deref()
            .filter(|_| value_at < call.cmd().words.len())
        else {
            continue;
        };
        match opt.name.as_str() {
            "m" | "message" => {
                written = true;
                used.insert(value_at);
                let spans = &call.cmd().spans;
                let whole = spans[opt.word + run.rest].start..spans[value_at].end;
                let mut found = from_word(call, value_at, opt.inline_at, label, findings);
                for source in &mut found {
                    if let Target::Word { whole: slot, .. } = &mut source.target {
                        *slot = Some(whole.clone());
                    }
                }
                sources.extend(found);
            }
            "F" | "file" => {
                written = true;
                used.insert(value_at);
                let dynamic = call.cmd().spans[value_at].dynamic;
                sources.extend(from_path(call, value, dynamic, label, findings));
            }
            _ => {}
        }
    }
    (sources, written)
}
