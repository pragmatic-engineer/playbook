// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The `gh pr` calls that write text on GitHub: `create` (also spelled `new`),
//! `edit` and `merge`. A PR title and body are free text, while the subject
//! and body of a merge become a commit message.

use super::engine::{Call, Findings, Plan};
use super::sources::{apply, from_path, from_word, value_span, Named, Rule, Source};
use crate::common::cli_opts::{Opt, Spec};

const PR_WRITE: Spec = Spec {
    short_values: "tbFaBHlmprRT",
    short_attached: "",
    long_values: &[
        "title",
        "body",
        "body-file",
        "assignee",
        "base",
        "head",
        "label",
        "milestone",
        "project",
        "reviewer",
        "repo",
        "template",
    ],
    long_flags: &["fill", "fill-first", "fill-verbose", "draft", "web"],
    other_long: &[],
    abbreviate: false,
};
const PR_MERGE: Spec = Spec {
    short_values: "tbFAR",
    short_attached: "",
    long_values: &[
        "subject",
        "body",
        "body-file",
        "author-email",
        "match-head-commit",
        "repo",
    ],
    long_flags: &[
        "merge",
        "squash",
        "rebase",
        "auto",
        "admin",
        "delete-branch",
    ],
    other_long: &[],
    abbreviate: false,
};

pub fn handle(call: &Call, plan: &mut Plan, findings: &mut Findings) {
    let words = &call.cmd().words;
    let (Some(group), Some(verb)) = (words.get(call.at + 1), words.get(call.at + 2)) else {
        return;
    };
    if group != "pr" {
        return;
    }
    let rest = call.at + 3;
    match verb.as_str() {
        "create" | "new" | "edit" => {
            let opts = PR_WRITE.scan(&words[rest..]);
            let title = text_sources(call, rest, &opts, &["t", "title"], "the PR title", findings);
            let body = text_sources(
                call,
                rest,
                &opts,
                &["b", "body", "F", "body-file"],
                "the PR body",
                findings,
            );
            apply(&title, Rule::Prose, false, plan, findings);
            apply(&body, Rule::Prose, false, plan, findings);
        }
        "merge" => {
            let opts = PR_MERGE.scan(&words[rest..]);
            let names = ["t", "subject", "b", "body", "F", "body-file"];
            let parts = text_sources(
                call,
                rest,
                &opts,
                &names,
                "the merge commit message",
                findings,
            );
            apply(&parts, Rule::Commit, true, plan, findings);
        }
        _ => {}
    }
}

/// The text of each option called one of `names`, as a body file when the
/// option is `-F` or `--body-file`.
fn text_sources(
    call: &Call,
    rest: usize,
    opts: &[Opt],
    names: &[&str],
    label: &str,
    findings: &mut Findings,
) -> Vec<Source> {
    let mut sources = Vec::new();
    for opt in opts.iter().filter(|o| names.contains(&o.name.as_str())) {
        let at = opt.value_word + rest;
        let Some(value) = opt.value.as_deref().filter(|_| at < call.cmd().words.len()) else {
            continue;
        };
        if matches!(opt.name.as_str(), "F" | "body-file") {
            let named = Named {
                path: value,
                dynamic: call.cmd().spans[at].dynamic,
                replace: value_span(call, at, opt.inline_at),
                opener: "- ",
            };
            sources.extend(from_path(call, &named, label, findings));
        } else {
            sources.extend(from_word(call, at, opt.inline_at, label, findings));
        }
    }
    sources
}
