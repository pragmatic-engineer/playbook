// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: MIT

//! The `git` calls that write a message: `commit`, `tag` and `merge`. Reads
//! the message from `-m`, from `-F` and from a heredoc fed to `-F -`, and
//! follows the script a `git rebase --exec` runs.

use super::engine::{nested_word, Call, Findings, Plan};
use super::sources::{apply, from_path, from_word, resolve, value_span, Source, Target};
use crate::common::cli_opts::{named, Opt, Spec};
use std::path::PathBuf;

// The specs list the options whose value can be a message or hides one, and
// the flags that change where a message comes from. `other_long` holds the
// rest of the subcommand's long names, as `--git-completion-helper-all` of the
// installed git lists them, so an exact name or a prefix shared with one of
// them is not taken for an option here. A test checks it against real git.
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
    other_long: &[
        "quiet",
        "verbose",
        "status",
        "gpg-sign",
        "all",
        "include",
        "interactive",
        "patch",
        "unified",
        "inter-hunk-context",
        "only",
        "no-verify",
        "dry-run",
        "short",
        "branch",
        "ahead-behind",
        "porcelain",
        "long",
        "null",
        "no-post-rewrite",
        "untracked-files",
        "pathspec-from-file",
        "pathspec-file-nul",
        "allow-empty",
        "allow-empty-message",
        "verify",
        "post-rewrite",
        "no-quiet",
        "no-verbose",
        "no-file",
        "no-author",
        "no-date",
        "no-message",
        "no-reedit-message",
        "no-reuse-message",
        "no-fixup",
        "no-squash",
        "no-reset-author",
        "no-trailer",
        "no-signoff",
        "no-template",
        "no-cleanup",
        "no-status",
        "no-gpg-sign",
        "no-all",
        "no-include",
        "no-interactive",
        "no-patch",
        "no-only",
        "no-dry-run",
        "no-short",
        "no-branch",
        "no-ahead-behind",
        "no-porcelain",
        "no-long",
        "no-null",
        "no-amend",
        "no-untracked-files",
        "no-pathspec-from-file",
        "no-pathspec-file-nul",
        "no-allow-empty",
        "no-allow-empty-message",
    ],
    abbreviate: true,
};
const MERGE: Spec = Spec {
    short_values: "mFsX",
    short_attached: "S",
    long_values: &["message", "file", "strategy", "strategy-option", "cleanup"],
    long_flags: &["no-edit", "edit", "squash", "no-commit", "signoff"],
    other_long: &[
        "stat",
        "summary",
        "compact-summary",
        "log",
        "commit",
        "ff",
        "ff-only",
        "rerere-autoupdate",
        "verify-signatures",
        "into-name",
        "verbose",
        "quiet",
        "abort",
        "quit",
        "continue",
        "allow-unrelated-histories",
        "progress",
        "gpg-sign",
        "autostash",
        "overwrite-ignore",
        "no-verify",
        "verify",
        "no-stat",
        "no-summary",
        "no-compact-summary",
        "no-log",
        "no-squash",
        "no-cleanup",
        "no-ff",
        "no-rerere-autoupdate",
        "no-verify-signatures",
        "no-strategy",
        "no-strategy-option",
        "no-message",
        "no-into-name",
        "no-verbose",
        "no-quiet",
        "no-abort",
        "no-quit",
        "no-continue",
        "no-allow-unrelated-histories",
        "no-progress",
        "no-gpg-sign",
        "no-autostash",
        "no-overwrite-ignore",
        "no-signoff",
    ],
    abbreviate: true,
};
const TAG: Spec = Spec {
    short_values: "mFuf",
    short_attached: "",
    long_values: &["message", "file", "local-user", "cleanup", "trailer"],
    long_flags: &["annotate", "sign", "force", "edit", "no-edit"],
    other_long: &[
        "list",
        "delete",
        "verify",
        "create-reflog",
        "column",
        "contains",
        "no-contains",
        "with",
        "without",
        "merged",
        "no-merged",
        "omit-empty",
        "sort",
        "points-at",
        "format",
        "color",
        "ignore-case",
        "no-annotate",
        "no-file",
        "no-trailer",
        "no-sign",
        "no-cleanup",
        "no-local-user",
        "no-force",
        "no-create-reflog",
        "no-column",
        "no-omit-empty",
        "no-sort",
        "no-points-at",
        "no-format",
        "no-color",
        "no-ignore-case",
    ],
    abbreviate: true,
};
const REBASE: Spec = Spec {
    short_values: "x",
    short_attached: "",
    long_values: &["exec"],
    long_flags: &[],
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

pub fn handle(call: &Call, plan: &mut Plan, findings: &mut Findings) {
    let Some((sub_at, dir)) = skip_globals(call) else {
        return;
    };
    let call = Call { dir: &dir, ..*call };
    let run = Run {
        call,
        rest: sub_at + 1,
    };
    match call.cmd().words[sub_at].as_str() {
        "commit" => commit(&run, plan, findings),
        "merge" => message_command(&run, &MERGE, "the merge message", plan, findings),
        "tag" => tag(&run, plan, findings),
        "rebase" => rebase(&run, plan, findings),
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

fn commit(run: &Run, plan: &mut Plan, findings: &mut Findings) {
    let opts = COMMIT.scan(&run.call.cmd().words[run.rest..]);
    let (sources, _) = message_sources(run, &opts, "the commit message", findings);
    apply(sources, plan, findings);
}

fn tag(run: &Run, plan: &mut Plan, findings: &mut Findings) {
    let opts = TAG.scan(&run.call.cmd().words[run.rest..]);
    let (sources, _) = message_sources(run, &opts, "the tag message", findings);
    apply(sources, plan, findings);
}

fn message_command(run: &Run, spec: &Spec, label: &str, plan: &mut Plan, findings: &mut Findings) {
    let opts = spec.scan(&run.call.cmd().words[run.rest..]);
    let (sources, _) = message_sources(run, &opts, label, findings);
    apply(sources, plan, findings);
}

/// The commands `-x` and `--exec` run are scripts of their own.
fn rebase(run: &Run, plan: &mut Plan, findings: &mut Findings) {
    let opts = REBASE.scan(&run.call.cmd().words[run.rest..]);
    for opt in named(&opts, &["x", "exec"]) {
        let at = opt.value_word + run.rest;
        if opt.value.is_some() {
            nested_word(&run.call, at, opt.inline_at, plan, findings);
        }
    }
}

/// The text of every `-m` and `-F` option, and whether there was one.
fn message_sources(
    run: &Run,
    opts: &[Opt],
    label: &str,
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
                let spans = &call.cmd().spans;
                let whole = spans[opt.word + run.rest].start..spans[value_at].end;
                let kept = kept_letters(opt, &call.cmd().words[opt.word + run.rest]);
                let mut found = from_word(call, value_at, opt.inline_at, label, findings);
                for source in &mut found {
                    if let Target::Word { whole: slot, .. } = &mut source.target {
                        *slot = Some((whole.clone(), kept.clone()));
                    }
                }
                sources.extend(found);
            }
            "F" | "file" => {
                written = true;
                let dynamic = call.cmd().spans[value_at].dynamic;
                let place = value_span(call, value_at, opt.inline_at);
                sources.extend(from_path(call, value, dynamic, place, label, findings));
            }
            _ => {}
        }
    }
    (sources, written)
}

/// What stays of the word of a short `-m` when the option goes: the other
/// letters of a cluster such as `-nm`, or nothing.
fn kept_letters(opt: &Opt, word: &str) -> String {
    let others = match word.strip_prefix('-').filter(|w| !w.starts_with('-')) {
        Some(cluster) if opt.name == "m" => cluster.split('m').next().unwrap_or_default(),
        _ => "",
    };
    if others.is_empty() {
        String::new()
    } else {
        format!("-{others}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::test_support::scratch_dir;
    use std::process::Command;

    /// The long option names `git <sub>` lists, read from the installed git.
    fn git_long_options(sub: &str) -> Vec<String> {
        let repo = scratch_dir("git-options");
        std::fs::create_dir_all(&repo).expect("scratch dir");
        let run = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&repo)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .expect("git runs")
        };
        run(&["init", "-q"]);
        let out = run(&[sub, "--git-completion-helper-all"]);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let _ = std::fs::remove_dir_all(&repo);
        text.split_whitespace()
            .filter(|w| w.starts_with("--") && *w != "--")
            .map(|w| w[2..].trim_end_matches('=').to_string())
            .collect()
    }

    /// `spec` without the long names the installed git lacks, which an older
    /// git never had. Each one left out is printed.
    fn known_to_git(sub: &str, spec: &Spec, names: &[String]) -> Spec {
        let keep = |list: &[&'static str]| -> &'static [&'static str] {
            let (known, missing): (Vec<&str>, Vec<&str>) = list
                .iter()
                .copied()
                .partition(|name| names.iter().any(|n| n == name));
            if !missing.is_empty() {
                eprintln!("note: the installed git {sub} has no {missing:?}, so they are skipped");
            }
            Box::leak(known.into_boxed_slice())
        };
        Spec {
            short_values: spec.short_values,
            short_attached: spec.short_attached,
            long_values: keep(spec.long_values),
            long_flags: keep(spec.long_flags),
            other_long: keep(spec.other_long),
            abbreviate: spec.abbreviate,
        }
    }

    /// The option git resolves `--prefix` to: an exact name, else the only
    /// name it starts. `None` when git refuses it as ambiguous.
    fn git_resolves(names: &[String], prefix: &str) -> Option<String> {
        if names.iter().any(|n| n == prefix) {
            return Some(prefix.to_string());
        }
        let mut starts = names.iter().filter(|n| n.starts_with(prefix));
        match (starts.next(), starts.next()) {
            (Some(only), None) => Some(only.clone()),
            _ => None,
        }
    }

    #[test]
    fn every_abbreviation_git_accepts_resolves_the_way_git_does() {
        for (sub, spec) in [("commit", COMMIT), ("merge", MERGE), ("tag", TAG)] {
            let names = git_long_options(sub);
            assert!(names.len() > 10, "git {sub} lists its options");
            let spec = known_to_git(sub, &spec, &names);
            let read: Vec<&str> = spec
                .long_values
                .iter()
                .chain(spec.long_flags)
                .copied()
                .collect();
            for name in &names {
                for end in 1..=name.len() {
                    let prefix = &name[..end];
                    let Some(expected) = git_resolves(&names, prefix) else {
                        continue;
                    };
                    let got = spec.scan(&[format!("--{prefix}")]);

                    let ours = got[0].name.as_str();
                    if read.contains(&expected.as_str()) {
                        assert_eq!(ours, expected, "git {sub} --{prefix}");
                    } else {
                        assert!(
                            !read.contains(&ours),
                            "git {sub} --{prefix} is {expected}, not {ours}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_specs_only_name_options_git_has() {
        for (sub, spec) in [("commit", COMMIT), ("merge", MERGE), ("tag", TAG)] {
            let names = git_long_options(sub);
            for must in ["message", "file"] {
                assert!(names.iter().any(|n| n == must), "git {sub} has no --{must}");
            }
            let ours = spec
                .long_values
                .iter()
                .chain(spec.long_flags)
                .chain(spec.other_long);
            for name in ours {
                if !names.iter().any(|n| n == name) {
                    eprintln!("note: the installed git {sub} has no --{name}, so it is skipped");
                }
            }
        }
    }

    #[test]
    fn a_trailing_cluster_letter_is_kept_when_its_message_goes() {
        let message = Opt {
            name: "m".to_string(),
            value: None,
            word: 0,
            value_word: 1,
            inline_at: None,
        };

        assert_eq!(kept_letters(&message, "-m"), "");
        assert_eq!(kept_letters(&message, "-nm"), "-n");
        assert_eq!(kept_letters(&message, "-anm"), "-an");
        assert_eq!(kept_letters(&message, "--message"), "");
    }
}
